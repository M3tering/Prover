use std::{collections::HashMap, fmt::Debug};

use alloy_primitives::{Bytes, B256, U256};
use alloy_sol_types::sol;
use alloy_trie::Nibbles;
use ed25519_dalek::VerifyingKey;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

mod util;
use util::validate_signature;

pub use util::{
    calc_slot_key, decode_slice, destructure_payload, extract_nonce, get_state_root, to_b256,
    to_keccak_hash, to_u256, trim_zeros, verify_account_proof,
};

sol! {
    #[derive(Serialize, Deserialize, Debug)]
    struct PublicValuesStruct {
        bytes32 block_hash;
        bytes32 previous_balances;
        bytes32 previous_nonces;
        bytes new_balances;
        bytes new_nonces;
    }
}

impl PublicValuesStruct {
    pub fn concat_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend(self.block_hash.as_slice());
        bytes.extend(self.previous_balances.as_slice());
        bytes.extend(self.previous_nonces.as_slice());
        bytes.extend(self.new_balances.clone());
        bytes.extend(self.new_nonces.clone());
        bytes
    }

    pub fn from_bytes(bytes: &[u8]) -> Self {
        let block_hash = B256::from_slice(&bytes[0..32]);
        let previous_balances = B256::from_slice(&bytes[32..64]);
        let previous_nonces = B256::from_slice(&bytes[64..96]);
        let update_values = &bytes[96..];
        let split_point = update_values.len() / 2;
        let new_balances = update_values[1..split_point].to_vec().into();
        let new_nonces = update_values[(split_point + 1)..].to_vec().into();

        PublicValuesStruct {
            block_hash,
            previous_balances,
            previous_nonces,
            new_balances,
            new_nonces,
        }
    }
}

fn deserialize_hex<'de, D>(deserializer: D) -> Result<Vec<u8>, D::Error>
where
    D: Deserializer<'de>,
{
    let s: String = Deserialize::deserialize(deserializer)?;
    let s = if s.starts_with("0x") {
        s.strip_prefix("0x").unwrap()
    } else {
        s.as_str()
    };
    hex::decode(s).map_err(serde::de::Error::custom)
}

pub fn serialize_hex<S>(bytes: &Vec<u8>, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    let hex_str = hex::encode(bytes);
    serializer.serialize_str(&hex_str)
}

#[derive(Serialize, Deserialize, Debug)]
pub struct ProofStruct {
    pub storage_hash: B256,
    pub proofs: HashMap<B256, (U256, Vec<Bytes>)>,
    pub encoded_account: Vec<u8>,
    pub account_proof: Vec<Bytes>,
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Payload {
    pub mempool: HashMap<String, Vec<M3terPayload>>,
    #[serde(deserialize_with = "deserialize_hex", serialize_with = "serialize_hex")]
    pub previous_nonces: Vec<u8>,
    #[serde(deserialize_with = "deserialize_hex", serialize_with = "serialize_hex")]
    pub previous_balances: Vec<u8>,
    pub proofs: ProofStruct,
    pub block_bytes: Vec<u8>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct M3terRawPayload(String);

impl M3terRawPayload {
    pub fn to_m3ter_payload(&self) -> M3terPayload {
        let (message, signature, nonce, energy) = util::destructure_payload(&self.0);
        M3terPayload::new(message.to_string(), signature.to_string(), nonce, energy)
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct M3terPayload {
    message: String,
    signature: String,
    nonce: u64,
    energy: u64,
}

impl M3terPayload {
    pub fn new(message: String, signature: String, nonce: u64, energy: u64) -> Self {
        M3terPayload {
            message,
            signature,
            nonce,
            energy,
        }
    }

    fn _msg_to_vec(&self) -> Vec<u8> {
        hex::decode(self.message.clone()).expect("Failed to decode hex")
    }
}

#[derive(Serialize, Deserialize, Debug)]
pub struct M3ter {
    pub m3ter_id: String,
    pub public_key: String,
}

impl M3ter {
    pub fn new(m3ter_id: &str, public_key: &str) -> Self {
        M3ter {
            m3ter_id: String::from(m3ter_id),
            public_key: String::from(public_key),
        }
    }

    fn validate_payload(&self, payload: &M3terPayload, verifying_key: VerifyingKey) -> bool {
        match validate_signature(&payload.message, &payload.signature, verifying_key) {
            Some(is_valid) => is_valid,
            None => {
                println!("Invalid signature for payload: {:?}", payload);
                false
            }
        }
    }

    fn verify_public_key(&self, storage_hash: &B256, proof: &Vec<Bytes>) -> bool {
        println!("storage hash = {:?}\nproof = {:?}", storage_hash, proof);
        let (m3ter_id, public_key) = (&self.m3ter_id, &self.public_key);
        let m3ter_id = m3ter_id.parse::<u64>().expect("invalid m3ter id");
        let slot_key = calc_slot_key(U256::from(m3ter_id)).expect("invalid slot key");

        let slot_key = Nibbles::unpack(to_keccak_hash(slot_key.to_be_bytes_vec()));
        let public_key = if public_key.starts_with("0x") {
            public_key.strip_prefix("0x").unwrap()
        } else {
            public_key.as_str()
        };

        let expected_value = U256::from_be_slice(&hex::decode(public_key).unwrap());
        println!("expected_value = {:?}", expected_value);
        let expected_value = alloy_rlp::encode(expected_value);
        let result =
            alloy_trie::proof::verify_proof(*storage_hash, slot_key, Some(expected_value), proof);
        match result {
            Ok(()) => true,
            Err(err) => {
                println!("Failed to verify proof: {:?}", err);
                false
            }
        }
    }
}

pub fn track_energy(
    m3ter: M3ter,
    m3ter_payloads: &[M3terPayload],
    start_nonce: u64,
    (storage_hash, proof): (&B256, &Vec<Bytes>),
) -> (u64, u64) {
    if !m3ter.verify_public_key(storage_hash, proof) {
        println!(
            "encountered invalid public_key for m3ter {}",
            m3ter.m3ter_id
        );
        return (0, start_nonce);
    }

    let verifying_key = util::build_verifying_key(&m3ter.public_key).unwrap();
    let mut nonce = start_nonce;
    let mut energy_sum = 0u64;
    for payload in m3ter_payloads {
        println!("nonce {}, payload nonce {}", nonce, payload.nonce);
        if nonce >= payload.nonce {
            continue;
        }
        if nonce + 1 != payload.nonce {
            println!(
                "Invalid nonce: {} not consercutive to {} for m3ter_id {}",
                &nonce, &payload.nonce, &m3ter.m3ter_id
            );
            break;
        }
        if !m3ter.validate_payload(payload, verifying_key) {
            println!("Invalid payload: {:?}", payload);
            break;
        };
        
        nonce = payload.nonce;
        energy_sum += payload.energy;
        println!(
            "State: energy {:?}, nonce {:?}",
            payload.energy, payload.nonce
        );
    }

    (energy_sum, nonce)
}
