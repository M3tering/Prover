//! An end-to-end example of using the SP1 SDK to generate a proof of a program that can have an
//! EVM-Compatible proof generated which can be verified on-chain.
//!
//! You can run this script using the following command:
//! ```shell
//! RUST_LOG=info cargo run --release --bin evm -- --system groth16
//! ```
//! or
//! ```shell
//! RUST_LOG=info cargo run --release --bin evm -- --system plonk
//! ```

use alloy_primitives::{Bytes, B256, U256};
use clap::{Parser, ValueEnum};
use energy_tracker_lib::{
    calc_slot_key, Payload, ProofStruct, PublicValuesStruct,
};
use energy_tracker_verifier::{get_block_rpl_bytes, get_provider, get_previous_values, get_storage_proofs};
use eyre::Result;
use serde::{Deserialize, Deserializer, Serialize};
use sp1_sdk::{
    include_elf, HashableKey, Prover, ProverClient, SP1ProofWithPublicValues, SP1Stdin, SP1VerifyingKey
};
use std::path::PathBuf;
use std::{collections::HashMap, fs::File, io::BufReader};

/// The ELF (executable and linkable format) file for the Succinct RISC-V zkVM.
pub const ENERGY_TRACKER_ELF: &[u8] = include_elf!("energy-tracker-program");

/// The arguments for the EVM command.
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct EVMArgs {
    #[arg(long, value_enum, default_value = "groth16")]
    system: ProofSystem,
}

/// Enum representing the available proof systems
#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, ValueEnum, Debug)]
enum ProofSystem {
    Plonk,
    Groth16,
}
/// A fixture that can be used to test the verification of SP1 zkVM proofs inside Solidity.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProofFixture {
    previous_balances: B256,
    previous_nonces: B256,
    new_balances: Bytes,
    new_nonces: Bytes,
    block_hash: B256,
    vkey: String,
    public_values: String,
    proof: String,
}


fn deserialize_payload<'de, D>(
    deserializer: D,
) -> Result<HashMap<String, Vec<energy_tracker_lib::M3terPayload>>, D::Error>
where
    D: Deserializer<'de>,
{
    let initial: HashMap<String, Vec<energy_tracker_lib::M3terRawPayload>> =
        HashMap::deserialize(deserializer).unwrap();
    let mut m3ter_payloads: HashMap<String, Vec<energy_tracker_lib::M3terPayload>> = HashMap::new();

    for (key, value) in initial {
        let payloads: Vec<energy_tracker_lib::M3terPayload> =
            value.iter().map(|v| v.to_m3ter_payload()).collect();
        m3ter_payloads.insert(key, payloads);
    }
    Ok(m3ter_payloads)
}

#[tokio::main]
async fn main() -> Result<()> {
    // Setup the logger.
    sp1_sdk::utils::setup_logger();
 
    // Parse the command line arguments.
    let args = EVMArgs::parse();

    std::env::set_var("SP1_PROVER", "network");
    std::env::set_var(
        "NETWORK_PRIVATE_KEY",
        "3b62b0fb8da4fc79eff9236c50527cd8bb9cd7c264f1c838b105d4570aa0491e",
    );

    println!("getting paylods from file");
    #[derive(Deserialize, Debug)]
    struct SamplePayload(
        #[serde(deserialize_with = "deserialize_payload")]
        HashMap<String, Vec<energy_tracker_lib::M3terPayload>>,
    );
    // Setup the inputs.
    let file = File::open("src/samples.json").unwrap();

    let reader = BufReader::new(file);
    let payloads: SamplePayload = serde_json::from_reader(reader).unwrap();
    println!("gotten paylods from file");

    let mut trimmed = HashMap::new();

    payloads.0.iter().for_each(|(key, value)| {
        trimmed.insert(key.to_string(),  value[0..10].to_vec());
    });

    let provider = get_provider().await?;
    let previous_nonces = dbg!(get_previous_values(&provider, U256::from(1)).await?);
    let previous_balances = dbg!(get_previous_values(&provider, U256::from(0)).await?);

    let slot_keys = trimmed
        .keys()
        .map(|key| {
            let m3ter_id: u64 = key.parse().expect("meter id not valid");
            m3ter_id
        })
        .map(|m3ter_id| calc_slot_key(U256::from(m3ter_id)).unwrap())
        .map(|slot_key| B256::from_slice(&slot_key.to_be_bytes_vec()))
        .collect();

    let (account_proof, encoded_account, storage_hash, proofs, anchor_block) =
        get_storage_proofs(&provider, slot_keys).await?;
    let block_bytes = get_block_rpl_bytes(&provider, anchor_block).await?;

    println!("Anchor Block: {}", anchor_block);
    let payload = Payload {
        mempool: trimmed,
        previous_nonces: previous_nonces.into(),
        previous_balances: previous_balances.into(),
        proofs: Some(ProofStruct {
            account_proof,
            encoded_account,
            storage_hash,
            proofs,
        }),
        block_bytes: Some(block_bytes),
    };

    let mut stdin = SP1Stdin::new();
    stdin.write(&payload);

    // Setup the prover client.
    let client = ProverClient::builder().cpu().build();

    // let (_, _report) = client.execute(ENERGY_TRACKER_ELF, &stdin).run().expect("failed to execute program");

    // Setup the program.
    let (pk, vk) = client.setup(ENERGY_TRACKER_ELF);

    println!("Proof System: {:?}", args.system);
    let proof = match args.system {
        ProofSystem::Plonk => client.prove(&pk, &stdin).plonk().run(),
        ProofSystem::Groth16 => client.prove(&pk, &stdin).groth16().run(),
    }
    .expect("failed to generate proof");

    create_proof_fixture(&proof, &vk, args.system);
    Ok(())
}

// Create a fixture for the given proof.
fn create_proof_fixture(
    proof: &SP1ProofWithPublicValues,
    vk: &SP1VerifyingKey,
    system: ProofSystem,
) {
    let bytes = proof.public_values.as_slice();
    let output = PublicValuesStruct::from_bytes(bytes);
    let PublicValuesStruct {
        previous_balances,
        previous_nonces,
        new_balances,
        new_nonces,
        block_hash,
    } = output;

    // Create the testing fixture so we can test things end-to-end.
    let fixture = ProofFixture {
        previous_balances,
        previous_nonces,
        new_balances,
        new_nonces,
        block_hash,
        vkey: vk.bytes32().to_string(),
        public_values: format!("0x{}", hex::encode(bytes)),
        proof: format!("0x{}", hex::encode(proof.bytes())),
    };

    // Save the fixture to a file.
    let fixture_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../contracts/fixtures");
    std::fs::create_dir_all(&fixture_path).expect("failed to create fixture path");
    std::fs::write(
        fixture_path.join(format!("{:?}-fixture.json", system).to_lowercase()),
        serde_json::to_string_pretty(&fixture).unwrap(),
    )
    .expect("failed to write fixture");
}
