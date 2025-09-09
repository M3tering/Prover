use std::collections::HashMap;

use alloy::{
    dyn_abi::DynSolValue, eips::BlockNumberOrTag, hex, json_abi::JsonAbi, primitives::{Address, Bytes, B256, U256}, providers::{Provider, ProviderBuilder}, signers::local::PrivateKeySigner
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
    "0x6E31632D6A7Af8d30766AA9E216c49F5AAb846c2"
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
            "inputs": [
                {
                    "internalType": "uint256",
                    "name": "anchorBlock",
                    "type": "uint256"
                },
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
    let rpc_url = std::env::var("RPC_URL")
        .unwrap_or_else(|_| "https://eth-sepolia.g.alchemy.com/v2/URjQnzNCUHumxPFL8VDoFBmpX4uqL6X8".to_string());
    println!("Connecting to provider at: {}", rpc_url);
    let private_key = "3b62b0fb8da4fc79eff9236c50527cd8bb9cd7c264f1c838b105d4570aa0491e";//std::env::var("PRIVATE_KEY").expect("private key should exist in env");
    let private_key = if private_key.starts_with("0x") {
        private_key.strip_prefix("0x").unwrap()
    } else {
        private_key
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

pub async fn get_storage_proofs(
    provider: &impl Provider,
    slots: Vec<B256>,
) -> Result<(Vec<Bytes>, Vec<u8>, B256, HashMap<B256, (U256, Vec<Bytes>)>, u64)> {
    let anchor_block = provider.get_block_number().await?;

    println!("slots {:?}", slots);
    let proof = provider.get_proof(get_m3ter_address(), slots);

    println!("geting storage_proofs at block = {:?}", anchor_block);
    let proof_at_block = proof
        .number(anchor_block)
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

pub async fn get_block_rpl_bytes(provider: &impl Provider, block_number: u64) -> Result<Vec<u8>> {
    let block = provider
        .get_block_by_number(BlockNumberOrTag::Number(block_number))
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
    anchor_block: u64,
    account_blob: &Bytes,
    nonce_blob: &Bytes,
    proof: &Bytes,
) -> Result<B256> {
    let rollup_address = get_rollup_address();
    let abi: JsonAbi = get_rollup_abi();
    let interface = Interface::new(abi);
    let anchor_block: U256 = U256::from(anchor_block);
    let contract = interface.connect(rollup_address, provider);

    println!("Committing state at block {}", anchor_block);
    let call_builder = contract.function(
        "commitState",
        &[
            anchor_block.into(),
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
