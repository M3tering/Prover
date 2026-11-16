use alloy_primitives::{Address, B256, U256};
use ed25519_dalek::{Signer, SigningKey};
use m3tering_protocol::{transition::*, SignedAction};
use std::collections::BTreeMap;
fn action(k: &SigningKey, n: u32, mwh: u32) -> SignedAction {
    let b = [n.to_be_bytes(), mwh.to_be_bytes()].concat();
    SignedAction {
        message: hex::encode(&b),
        signature: hex::encode(k.sign(&b).to_bytes()),
    }
}
fn fixture() -> Input {
    let k = SigningKey::from_bytes(&[42; 32]);
    let pk = B256::from(k.verifying_key().to_bytes());
    Input {
        chain_id: U256::from(1),
        rollup: Address::repeat_byte(0x11),
        words: BTreeMap::from([
            (0, U256::from(10) | (U256::from(1) << 40)),
            (1, U256::ZERO),
            (1000, U256::ZERO),
        ]),
        keys: BTreeMap::from([(0, [pk; 4]), (1, [pk; 4]), (1000, [pk; 4])]),
        actions: BTreeMap::from([
            (0, vec![action(&k, 2, 25_600), action(&k, 3, 5_600)]),
            (3, vec![action(&k, 1, 99_999)]),
            (4, vec![action(&k, 1, 600)]),
            (4000, vec![action(&k, 1, 7000)]),
        ]),
    }
}
#[test]
fn end_to_end_and_export() {
    let i = fixture();
    let t = execute(&i).unwrap();
    assert_eq!(t.updates.len(), 3);
    assert_eq!(unpack(t.updates[0].new, 0), (41, 3));
    assert_eq!(unpack(t.updates[0].new, 3), (99, 1));
    assert_eq!(unpack(t.updates[1].new, 0), (0, 1));
    assert_eq!(t.public_values.as_slice().len(), 32);
    if let Ok(dir) = std::env::var("FIXTURE_DIR") {
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            format!("{dir}/input.json"),
            serde_json::to_vec_pretty(&i).unwrap(),
        )
        .unwrap();
        std::fs::write(
            format!("{dir}/transition.json"),
            serde_json::to_vec_pretty(&t).unwrap(),
        )
        .unwrap();
        std::fs::write(format!("{dir}/encoding.json"),serde_json::to_vec_pretty(&serde_json::json!({"commitment":t.public_values,"payload":format!("0x{}",hex::encode(&t.payload)),"oldWord":format!("{:#066x}",t.updates[0].old),"keyHash":t.updates[0].keys_hash})).unwrap()).unwrap();
    }
}
#[test]
fn old_words_keys_and_domain_are_bound() {
    let i = fixture();
    let t = execute(&i).unwrap();
    let mut j = i.clone();
    j.chain_id += U256::from(1);
    assert_ne!(statement(&j, &t.updates), t.public_values);
    j = i.clone();
    j.rollup = Address::ZERO;
    assert_ne!(statement(&j, &t.updates), t.public_values);
    let mut u = t.updates.clone();
    u[0].old += U256::from(1);
    assert_ne!(statement(&i, &u), t.public_values);
    u = t.updates.clone();
    u[0].keys_hash = B256::ZERO;
    assert_ne!(statement(&i, &u), t.public_values);
}
#[test]
fn grouping_and_nonce_only_update() {
    let mut i = fixture();
    i.actions.retain(|m, _| *m == 0 || *m == 3);
    assert_eq!(execute(&i).unwrap().updates.len(), 1);
    i = fixture();
    i.actions.retain(|m, _| *m == 4);
    let t = execute(&i).unwrap();
    assert_eq!(unpack(t.updates[0].new, 0), (0, 1));
}

#[test]
fn rejects_id_beyond_slot_capacity() {
    let mut i = fixture();
    let actions = i.actions.remove(&0).unwrap();
    i.actions.insert((MAX_SLOT + 1) * 4, actions);
    assert!(execute(&i).unwrap_err().contains("uint32"));
}

#[test]
fn all_transition_fields_are_bound() {
    let i = fixture();
    let t = execute(&i).unwrap();
    for field in 0..3 {
        let mut changed = t.updates.clone();
        match field {
            0 => changed[0].slot += 1,
            1 => changed[0].new += U256::from(1),
            _ => {
                changed.pop();
            }
        }
        assert_ne!(statement(&i, &changed), t.public_values);
    }
}

#[test]
fn four_byte_slot_boundary_is_preserved() {
    for slot in [0x01020304_u64, MAX_SLOT] {
        let update = Update {
            slot,
            old: U256::ZERO,
            new: U256::from(1),
            keys_hash: B256::ZERO,
        };
        let payload = encode(&[update]);
        assert_eq!(payload.len(), 36);
        assert_eq!(&payload[..4], &(slot as u32).to_be_bytes());
    }
}
