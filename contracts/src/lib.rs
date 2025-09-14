use std::collections::HashMap;

use alloy::{
    dyn_abi::DynSolValue,
    hex, json_abi::JsonAbi,
    primitives::{Address, Bytes, B256, U256},
    providers::{Provider, ProviderBuilder},
    signers::local::PrivateKeySigner,
};

use alloy_contract::Interface;
use alloy_rlp::{encode, RlpEncodable};
use eyre::{Ok, Result};

#[derive(Debug, RlpEncodable)]
pub struct Account {
    nonce: u64,
    balance: U256,
    storage_hash: B256,
    code_hash: B256,
}

fn get_rollup_address() -> Address {
    "0xf8f2d4315DB5db38f3e5c45D0bCd59959c603d9b"
        .parse()
        .expect("Invalid address")
}

fn get_m3ter_address() -> Address {
    "0x40a36C0eF29A49D1B1c1fA45fab63762f8FC423F"
        .parse()
        .expect("Invalid address")
}

fn get_rollup_abi() -> JsonAbi {
    let call_abi = r#"[
        {
            "inputs":[],
            "name":"L1Checkpoint",
            "outputs":[
                {
                    "internalType":"bytes32",
                    "name":"",
                    "type":"bytes32"
                }
            ],
            "stateMutability":"view",
            "type":"function"
        },
        {
            "name": "latestStateAddress",
            "type": "function",
            "inputs": [
                {
                    "name": "io",
                    "type": "uint256"
                }
            ],
            "outputs": [
                {
                    "type": "address"
                }
            ],
            "stateMutability": "view"
        },
        {
            "inputs": [],
            "name": "anchorBlock",
            "outputs": [
                {
                    "internalType" :"bytes32",
                    "name":"",
                    "type":
                    "bytes32"
                }
            ],
            "stateMutability": "view",
            "type": "function"
        },
        {
            "inputs": [
                {
                    "internalType": "bytes",
                    "name": "accountBlob",
                    "type": "bytes"
                },
                {
                    "internalType": "bytes",
                    "name": "nonceBlob",
                    "type": "bytes"
                },
                {
                    "internalType": "bytes",
                    "name": "proof",
                    "type": "bytes"
                }
            ],
            "name": "commitState",
            "outputs": [],
            "stateMutability": "nonpayable",
            "type": "function"
        }
    ]"#;

    serde_json::from_str(call_abi).expect("Failed to parse ABI")
}

pub async fn get_provider() -> Result<impl Provider> {
    let rpc_url = std::env::var("RPC_URL").unwrap_or_else(|_| {
        "https://eth-sepolia.g.alchemy.com/v2/URjQnzNCUHumxPFL8VDoFBmpX4uqL6X8".to_string()
    });
    println!("Connecting to provider at: {}", rpc_url);
    let private_key = std::env::var("PRIVATE_KEY").expect("private key should exist in env");
    let private_key = if private_key.starts_with("0x") {
        private_key.strip_prefix("0x").unwrap()
    } else {
        private_key.as_str()
    };
    let signer = PrivateKeySigner::from_slice(
        &hex::decode(private_key).expect("Failed to decode private key"),
    )
    .expect("Failed to create signer from private key");

    let provider = ProviderBuilder::new()
        .wallet(signer)
        .with_cached_nonce_management()
        .connect_http(rpc_url.parse()?);
    Ok(Box::new(provider))
}

pub async fn get_anchor_block_hash(provider: &impl Provider) -> Result<B256> {
    let rollup_address = get_rollup_address();
    let abi: JsonAbi = get_rollup_abi();
    let interface = Interface::new(abi);
    let contract = interface.connect(rollup_address, provider);

    let get_anchor_block = contract.function("anchorBlock", &[])?;
    let result = get_anchor_block.call().await?;

    let anchor_hash = B256::from_slice(result[0].as_fixed_bytes().unwrap().0);
    Ok(anchor_hash)
}

pub async fn get_storage_proofs(
    provider: &impl Provider,
    slots: Vec<B256>,
) -> Result<(
    Vec<Bytes>,
    Vec<u8>,
    B256,
    HashMap<B256, (U256, Vec<Bytes>)>,
    B256,
)> {
    let anchor_block = get_anchor_block_hash(provider).await?;

    println!("slots {:?}", slots);
    let proof = provider.get_proof(get_m3ter_address(), slots);

    println!("geting storage_proofs at block = {:?}", anchor_block);
    let proof_at_block = proof
        .hash(anchor_block)
        .await
        .map_err(|e| eyre::eyre!("Failed to get proof: {}", e))?;

    println!("storage_proofs = {:?}", proof_at_block.storage_proof);

    let account = Account {
        nonce: proof_at_block.nonce,
        balance: proof_at_block.balance,
        code_hash: proof_at_block.code_hash,
        storage_hash: proof_at_block.storage_hash,
    };

    let encoded_account = encode(account);
    let mut storage_proofs: HashMap<B256, (U256, Vec<Bytes>)> = HashMap::new();
    for proof in proof_at_block.storage_proof.iter() {
        storage_proofs
            .entry(proof.key.as_b256())
            .insert_entry((proof.value, proof.proof.clone()));
    }

    Ok((
        proof_at_block.account_proof,
        encoded_account,
        proof_at_block.storage_hash,
        storage_proofs,
        anchor_block,
    ))
}

pub async fn get_block_rpl_bytes(provider: &impl Provider, block_hash: B256) -> Result<Vec<u8>> {
    let block = provider
        .get_block_by_hash(block_hash)
        .await
        .map_err(|e| eyre::eyre!("Failed to get block: {}", e))?;

    if let Some(block) = block {
        let block_header = block.header;
        let block_bytes = encode(block_header.into_consensus());
        Ok(block_bytes)
    } else {
        Err(eyre::eyre!("Block not found"))
    }
}

pub async fn get_previous_values(provider: &impl Provider, selector: U256) -> Result<Bytes> {
    let rollup_address = get_rollup_address();

    let abi: JsonAbi = get_rollup_abi();
    let interface = Interface::new(abi);

    let contract = interface.connect(rollup_address, &provider);
    let call_builder = contract.function("latestStateAddress", &[selector.into()])?;
    println!("getting state address");
    let state_address = call_builder.call().await?;

    println!("state address {:?}", state_address);
    let code = provider
        .get_code_at(state_address[0].as_address().unwrap())
        .await?;
    println!("code length: {}", code.len());
    Ok(code)
}

pub async fn commit_state(
    provider: &impl Provider,
    account_blob: &Bytes,
    nonce_blob: &Bytes,
    proof: &Bytes,
) -> Result<B256> {
    let rollup_address = get_rollup_address();
    let abi: JsonAbi = get_rollup_abi();
    let interface = Interface::new(abi);
    let contract = interface.connect(rollup_address, provider);

    let call_builder = contract.function(
        "commitState",
        &[
            DynSolValue::Bytes(account_blob.to_vec()),
            DynSolValue::Bytes(nonce_blob.to_vec()),
            DynSolValue::Bytes(proof.to_vec()),
        ],
    )?;

    let pending_tx = call_builder.send().await?;

    // Send the transaction
    // let pending_tx = provider.send_raw_transaction(&signed_tx.as_bytes()).await?;
    println!("Transaction sent with hash: {:?}", &pending_tx.tx_hash());
    let hash = *pending_tx.tx_hash();
    // Wait for confirmation
    let receipt = pending_tx.get_receipt().await?;
    println!("Transaction confirmed in block: {:?}", receipt.block_number);
    Ok(hash)
}
