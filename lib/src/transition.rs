//! Authenticated packed-state transitions and the Rollup statement codec.
use crate::{accounting, SignedAction};
use alloy_primitives::{keccak256, Address, B256, U256};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
pub const MAX_SLOT: u64 = u32::MAX as u64;
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Input {
    pub chain_id: U256,
    pub rollup: Address,
    pub words: BTreeMap<u64, U256>,
    pub keys: BTreeMap<u64, [B256; 4]>,
    pub actions: BTreeMap<u64, Vec<SignedAction>>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Update {
    pub slot: u64,
    pub old: U256,
    pub new: U256,
    pub keys_hash: B256,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Transition {
    pub public_values: B256,
    pub updates: Vec<Update>,
    pub payload: Vec<u8>,
}
pub fn unpack(word: U256, pair: u64) -> (u64, u32) {
    assert!(pair < 4);
    let value = ((word >> (pair as usize * 64)) & U256::from(u64::MAX)).to::<u64>();
    (value & accounting::MAX_ACCOUNT, (value >> 40) as u32)
}
fn replace(word: U256, pair: u64, account: u64, nonce: u32) -> U256 {
    let shift = pair as usize * 64;
    (word & !(U256::from(u64::MAX) << shift))
        | ((U256::from(account) | (U256::from(nonce) << 40)) << shift)
}
pub fn keys_hash(keys: &[B256; 4]) -> B256 {
    let mut bytes = Vec::with_capacity(128);
    for key in keys {
        bytes.extend(key.as_slice());
    }
    keccak256(bytes)
}
pub fn statement(input: &Input, updates: &[Update]) -> B256 {
    let mut bytes = Vec::with_capacity(52 + 100 * updates.len());
    bytes.extend(input.chain_id.to_be_bytes::<32>());
    bytes.extend(input.rollup.as_slice());
    for u in updates {
        bytes.extend(&u.slot.to_be_bytes()[4..]);
        bytes.extend(u.old.to_be_bytes::<32>());
        bytes.extend(u.new.to_be_bytes::<32>());
        bytes.extend(u.keys_hash.as_slice());
    }
    keccak256(bytes)
}
pub fn execute(input: &Input) -> Result<Transition, String> {
    let mut grouped: BTreeMap<u64, Vec<(u64, &Vec<SignedAction>)>> = BTreeMap::new();
    for (m, a) in &input.actions {
        if m / 4 > MAX_SLOT {
            return Err("id exceeds uint32 slot".into());
        }
        grouped.entry(m / 4).or_default().push((*m, a));
    }
    let mut updates = Vec::new();
    for (slot, meters) in grouped {
        let old = *input.words.get(&slot).ok_or("missing old word")?;
        let mut new = old;
        let keys = input.keys.get(&slot).ok_or("missing key group")?;
        for (id, actions) in meters {
            let key = keys[(id % 4) as usize];
            if key == B256::ZERO {
                return Err("unregistered id".into());
            }
            let (account, nonce) = unpack(new, id % 4);
            let (account, nonce) = accounting::apply(account, nonce, key.0, actions)?;
            new = replace(new, id % 4, account, nonce);
        }
        if new != old {
            let mut used_keys = *keys;
            for pair in 0..4 {
                if unpack(old, pair) == unpack(new, pair) {
                    used_keys[pair as usize] = B256::ZERO;
                }
            }
            updates.push(Update {
                slot,
                old,
                new,
                keys_hash: keys_hash(&used_keys),
            });
        }
    }
    if updates.is_empty() {
        return Err("no state changes".into());
    }
    if updates.len() > u32::MAX as usize {
        return Err("too many updates".into());
    }
    let public_values = statement(input, &updates);
    let payload = encode(&updates);
    Ok(Transition {
        public_values,
        updates,
        payload,
    })
}
/// Fixed 36-byte records: uint32 slot, uint256 absolute new word, both big endian.
pub fn encode(updates: &[Update]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(36 * updates.len());
    for u in updates {
        assert!(u.slot <= MAX_SLOT, "slot exceeds uint32");
        bytes.extend(&u.slot.to_be_bytes()[4..]);
        bytes.extend(u.new.to_be_bytes::<32>());
    }
    bytes
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pair_boundaries() {
        let mut w = U256::ZERO;
        for p in 0..4 {
            w = replace(
                w,
                p,
                accounting::MAX_ACCOUNT - p,
                accounting::MAX_NONCE - p as u32,
            );
        }
        for p in 0..4 {
            assert_eq!(
                unpack(w, p),
                (
                    accounting::MAX_ACCOUNT - p,
                    accounting::MAX_NONCE - p as u32
                )
            );
        }
    }
    #[test]
    fn fixed_records_include_every_index() {
        let u = vec![
            Update {
                slot: 2,
                old: U256::ZERO,
                new: U256::from(1),
                keys_hash: B256::ZERO,
            },
            Update {
                slot: 3,
                old: U256::ZERO,
                new: U256::from(1),
                keys_hash: B256::ZERO,
            },
        ];
        let b = encode(&u);
        assert_eq!(b.len(), 72);
        assert_eq!(&b[..4], &[0, 0, 0, 2]);
        assert_eq!(&b[36..40], &[0, 0, 0, 3]);
    }
}
