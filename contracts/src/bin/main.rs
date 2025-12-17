use alloy::primitives::{B256, U256};
use energy_tracker_verifier::{get_m3ter_address, get_provider, get_storage_at, get_storage_proofs};


#[tokio::main]
async fn main() {
   let provider = get_provider().await.expect("Failed to get provider");
   let slot = calc_slot_key(U256::from(18u64));
   let address = get_m3ter_address();

   println!("slot to b256: {:?}", to_b256(slot.unwrap()));

   let value = get_storage_proofs(&provider, [to_b256(slot.unwrap())].to_vec()).await.expect("Failed to get storage value");
   
   println!("Storage proofs at slot {:?}: {:?}", slot.unwrap(), value.3);
}

pub fn calc_slot_key(key: U256) -> Option<U256> {
    let slot_literal: U256 =
        "97075990194835763561528983445257952440596761921281503889599705229225710478219"
            .parse()
            .expect("invalid slot literal");

    key.checked_add(slot_literal)
}

pub fn to_b256(value: U256) -> B256 {
    B256::from_slice(&value.to_be_bytes_vec())
}