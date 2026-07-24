#![no_main]
sp1_zkvm::entrypoint!(main);

use core::panic;

use energy_tracker_lib::{
    calc_slot_key, get_state_root, to_b256, to_keccak_hash, to_u256, track_energy, trim_zeros,
    verify_account_proof, M3ter, Payload, PublicValuesStruct,
};

pub fn main() {
    let payload = sp1_zkvm::io::read::<Payload>();
    let address = "9C547B649475f1bE81323AefdbcF209C17961D5E";

    let mempool = payload.mempool;
    let mut initial_nonces = payload.previous_nonces;
    let mut initial_balances = payload.previous_balances;

    if initial_nonces.len() == 2 {
        initial_nonces = vec![]
    } else {
        initial_nonces = initial_nonces[0..].to_vec()
    };
    if initial_balances.len() == 2 {
        initial_balances = vec![]
    } else {
        initial_balances = initial_balances[0..].to_vec()
    };
    let previous_nonces = initial_nonces.to_owned();
    let previous_balances = initial_balances.to_owned();

    let (account_proof, encoded_account, storage_hash, proofs) = match payload.proofs {
        Some(value) => (
            value.account_proof,
            value.encoded_account,
            value.storage_hash,
            value.proofs,
        ),
        None => panic!("storage proofs missing"),
    };

    let (state_root, block_bytes) = match payload.block_bytes {
        Some(value) => (get_state_root(&value), value),
        None => panic!("block bytes missing"),
    };

    if !verify_account_proof(
        state_root,
        hex::decode(address).unwrap(),
        encoded_account,
        account_proof,
    ) {
        panic!("Account proof verification failed");
    };

    let m3ter_position = |m3ter_id: usize| (m3ter_id * 6, m3ter_id * 6 + 6);
    let decode_slice = |data: &[u8; 6]| -> u64 {
        // Convert 6 bytes to i64 (big-endian, pad with zeros)
        let mut buf = [0u8; 8];
        buf[2..].copy_from_slice(data); // pad the first 2 bytes with zeros
        u64::from_be_bytes(buf)
    };

    let encode_slice = |value: u64| -> ([u8; 6], bool) {
        let bytes: [u8; 8] = value.to_be_bytes(); // [u8; 8]
        if bytes[..2][0] + bytes[..2][1] > 0 {
            return ([0; 6], false);
        }

        let six_bytes = &bytes[2..8]; // Take the last 6 bytes (big-endian)
        (six_bytes.try_into().unwrap(), true)
    };

    if previous_nonces.len() != previous_balances.len() {
        panic!(
            "total nonces {} does not equal total balances {}",
            previous_nonces.len(),
            previous_balances.len()
        )
    }
    let mut new_nonces = previous_nonces.clone();
    let mut new_balances = previous_balances.clone();

    for (m3ter_key, m3ter_payloads) in mempool {
        let m3ter_id = m3ter_key.parse::<usize>().unwrap();
        let (public_key, proof) = match proofs.get(dbg!(&to_b256(
            calc_slot_key(to_u256(m3ter_id as u64)).unwrap()
        ))) {
            Some(value) => value,
            None => panic!("failed to get proof for m3ter {}", m3ter_id),
        };
        let public_key = to_b256(*public_key).to_string();
        let m3ter = M3ter::new(&m3ter_key, &public_key);

        let (start, end) = m3ter_position(m3ter_id);
        if start >= previous_nonces.len() || previous_nonces.len() < 6 {
            let padding_len = end - previous_nonces.len();
            new_nonces.extend(vec![0u8; padding_len]);
            new_balances.extend(vec![0u8; padding_len]);
        }

        println!(
            "Decoding previous values for M3ter ID: {}, nonce {}, balance {}",
            m3ter_id,
            hex::encode(&new_nonces[start..end]),
            hex::encode(&new_balances[start..end])
        );
        let current_nonce = decode_slice(&new_nonces[start..end].try_into().unwrap());
        let current_balance = decode_slice(&new_balances[start..end].try_into().unwrap());
        println!(
            "Decoded values = Current Nonce: {}, Current Balance: {}",
            current_nonce, current_balance
        );
        let (energy_sum, latest_nonce) = track_energy(
            m3ter,
            &m3ter_payloads,
            current_nonce,
            (&storage_hash, proof),
        );
        println!(
            "Values after tracking = Energy Sum: {}, Latest Nonce: {}",
            energy_sum, latest_nonce
        );

        let energy_sum = energy_sum + current_balance;

        let (nonce_encoded, nonce_status) = encode_slice(latest_nonce);
        let (balance_encoded, status) = encode_slice(energy_sum);
        if !nonce_status || !status {
            println!(
                "Nonce or balance exceeds the 6-byte limit for m3ter ID: {}",
                m3ter_id
            );
            continue;
        }
        println!(
            "Encoded values = Nonce: {}, Balance: {}",
            &hex::encode(nonce_encoded),
            &hex::encode(balance_encoded)
        );

        new_nonces[start..end].copy_from_slice(&nonce_encoded);
        new_balances[start..end].copy_from_slice(&balance_encoded);

        println!(
            "M3ter ID: {}, Energy Sum: {}, Latest Nonce: {}",
            m3ter_id, energy_sum, latest_nonce
        );
    }

    let new_balances = trim_zeros(new_balances);

    if new_balances == initial_balances {
        panic!("New balances matches previous balances");
    }

    let new_nonces = trim_zeros(new_nonces).into();
    let new_balances = new_balances.into();

    let public_values = PublicValuesStruct {
        block_hash: to_keccak_hash(block_bytes),
        previous_balances: to_keccak_hash(initial_balances),
        previous_nonces: to_keccak_hash(initial_nonces),
        new_balances,
        new_nonces,
    };
    sp1_zkvm::io::commit_slice(&public_values.concat_bytes());
}
