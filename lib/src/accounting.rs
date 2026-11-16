//! Meter accounting: uint40 Wh + uint24 nonce. Wire messages remain u32 nonce/mWh.
use crate::SignedAction;
use ed25519_dalek::{Signature, VerifyingKey};
pub const MAX_ACCOUNT: u64 = (1u64 << 40) - 1;
pub const MAX_NONCE: u32 = (1u32 << 24) - 1;
/// Aggregate accepted mWh, then truncate once per meter per commit (per-commit policy).
pub fn apply(
    account: u64,
    mut nonce: u32,
    public_key: [u8; 32],
    actions: &[SignedAction],
) -> Result<(u64, u32), String> {
    if account > MAX_ACCOUNT || nonce > MAX_NONCE {
        return Err("invalid initial packed fields".into());
    }
    let key = VerifyingKey::from_bytes(&public_key).map_err(|_| "invalid public key")?;
    let mut milliwh = 0u64;
    for action in actions {
        let msg = hex::decode(action.message.strip_prefix("0x").unwrap_or(&action.message))
            .map_err(|_| "invalid message hex")?;
        if msg.len() != 8 {
            return Err("signed message must contain exactly eight bytes".into());
        }
        let next = u32::from_be_bytes(msg[..4].try_into().unwrap());
        let energy = u32::from_be_bytes(msg[4..].try_into().unwrap());
        if next <= nonce {
            continue;
        }
        if nonce.checked_add(1) != Some(next) {
            break;
        }
        let signature = hex::decode(
            action
                .signature
                .strip_prefix("0x")
                .unwrap_or(&action.signature),
        )
        .ok()
        .and_then(|b| Signature::from_slice(&b).ok());
        if !signature.is_some_and(|s| key.verify_strict(&msg, &s).is_ok()) {
            break;
        }
        if next > MAX_NONCE {
            return Err("uint24 nonce capacity exhausted".into());
        }
        milliwh = milliwh
            .checked_add(u64::from(energy))
            .ok_or("mWh aggregate overflow")?;
        nonce = next;
    }
    let account = account
        .checked_add(milliwh / 1000)
        .filter(|a| *a <= MAX_ACCOUNT)
        .ok_or("uint40 Wh capacity exhausted")?;
    Ok((account, nonce))
}
#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};
    fn action(key: &SigningKey, n: u32, e: u32) -> SignedAction {
        let msg = [n.to_be_bytes(), e.to_be_bytes()].concat();
        SignedAction {
            message: hex::encode(&msg),
            signature: hex::encode(key.sign(&msg).to_bytes()),
        }
    }
    #[test]
    fn aggregate_before_truncation() {
        let k = SigningKey::from_bytes(&[42; 32]);
        assert_eq!(
            apply(
                0,
                0,
                k.verifying_key().to_bytes(),
                &[action(&k, 1, 600), action(&k, 2, 600)]
            )
            .unwrap(),
            (1, 2)
        );
    }
    #[test]
    fn explicitly_discards_remainder_between_commits() {
        let k = SigningKey::from_bytes(&[42; 32]);
        let (a, n) = apply(0, 0, k.verifying_key().to_bytes(), &[action(&k, 1, 600)]).unwrap();
        assert_eq!(
            apply(a, n, k.verifying_key().to_bytes(), &[action(&k, 2, 600)]).unwrap(),
            (0, 2)
        );
    }
    #[test]
    fn units_and_caps() {
        let k = SigningKey::from_bytes(&[42; 32]);
        let pk = k.verifying_key().to_bytes();
        assert_eq!(
            apply(0, 0, pk, &[action(&k, 1, 1_000_000)]).unwrap(),
            (1000, 1)
        );
        assert!(apply(MAX_ACCOUNT, 0, pk, &[action(&k, 1, 1000)]).is_err());
        assert!(apply(0, MAX_NONCE, pk, &[action(&k, MAX_NONCE + 1, 1)]).is_err());
    }
    #[test]
    fn signed_accounting_only() {
        let k = SigningKey::from_bytes(&[42; 32]);
        let mut a = action(&k, 1, 1000);
        a.message = "00000001000007d0".into();
        assert_eq!(
            apply(0, 0, k.verifying_key().to_bytes(), &[a]).unwrap(),
            (0, 0)
        );
    }
}
