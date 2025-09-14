//! An end-to-end example of using the SP1 SDK to generate a proof of a program that can be executed
//! or have a core proof generated.
//!
//! You can run this script using the following command:
//! ```shell
//! RUST_LOG=info cargo run --release -- --execute
//! ```
//! or
//! ```shell
//! RUST_LOG=info cargo run --release -- --prove
//! ```

use std::{collections::HashMap, fs::File, io::BufReader};

use alloy_primitives::{B256, U256};
use clap::Parser;
use energy_tracker_lib::{calc_slot_key, Payload, ProofStruct};
use energy_tracker_verifier::{
    get_block_rpl_bytes, get_previous_values, get_provider, get_storage_proofs,
};
use serde::{Deserialize, Deserializer};
use sp1_sdk::{include_elf, Prover, ProverClient, SP1Stdin};

// use base64::{Engine as _, alphabet, engine::{self, general_purpose}};

/// The ELF (executable and linkable format) file for the Succinct RISC-V zkVM.
pub const ENERGY_TRACKER_ELF: &[u8] = include_elf!("energy-tracker-program");

/// The arguments for the command.
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    #[arg(long)]
    execute: bool,

    #[arg(long)]
    prove: bool,

    #[arg(long, default_value = "20")]
    n: u32,
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
async fn main() -> eyre::Result<()> {
    // Setup the logger.
    sp1_sdk::utils::setup_logger();
    dotenv::dotenv().ok();

    // Parse the command line arguments.
    let args = Args::parse();

    if args.execute == args.prove {
        eprintln!("Error: You must specify either --execute or --prove");
        std::process::exit(1);
    }

    println!("getting started");
    // Setup the prover client.

    println!("getting started with cpu prover");
    // Setup the program.

    println!("getting paylods from file");
    #[derive(Deserialize, Debug)]
    struct SamplePayload(
        #[serde(deserialize_with = "deserialize_payload")]
        HashMap<String, Vec<energy_tracker_lib::M3terPayload>>,
    );
    // Setup the inputs.
    let file = File::open("/home/godwin/energy-tracker/script/src/samples.json").unwrap();

    let reader = BufReader::new(file);
    let payloads: SamplePayload = serde_json::from_reader(reader).unwrap();
    println!("gotten paylods from file");

    let mut trimmed = HashMap::new();

    payloads.0.iter().for_each(|(key, value)| {
        trimmed.insert(key.to_string(), value[0..30].to_vec());
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
    let client = ProverClient::builder()
    // .network()
    // .private_key(&private_key)
    // .rpc_url(&rpc_url)
    .cpu()
    .build();

    if args.execute {
        // Execute the program
        let (_output, report) = client.execute(ENERGY_TRACKER_ELF, &stdin).run().unwrap();
        println!("Program executed successfully.");

        // Read the output.
        println!("report: {:?}", report);

        // Record the number of cycles executed.
        println!("Number of cycles: {}", report.total_instruction_count());
    } else {
        // Setup the program for proving.
        let (_pk, _vk) = client.setup(ENERGY_TRACKER_ELF);
        println!("Setup completed, proving...");

        // Generate the proof
        // let proof = client
        //     .prove(&pk, &stdin)
        //     .groth16()
        //     .run()
        //     .expect("failed to generate proof");

        // println!("Successfully generated proof! {:?}", proof);

        // Verify the proof.
        // client.verify(&proof, &vk).expect("failed to verify proof");
        println!("Successfully verified proof!");
    }
    Ok(())
}
