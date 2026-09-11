use std::{collections::HashMap, env, format, println, sync::Arc};

use alloy_primitives::{B256, Bytes, U256, hex};
use axum::{
    Router,
    extract::{Query, State},
    http::StatusCode,
    response::Json,
    routing::{get, post},
};
use diesel::{
    PgConnection, RunQueryDsl,
    prelude::{Insertable, Queryable, QueryableByName},
    r2d2::{self, ConnectionManager, PooledConnection},
    sql_query, table,
};

use energy_tracker_lib::{
    Payload, ProofStruct, PublicValuesStruct, calc_slot_key, decode_slice, destructure_payload,
    extract_nonce, to_b256,
};
use energy_tracker_verifier::{
    Provider, check_gas_balance, check_program_vkey, commit_state, get_block_rpl_bytes,
    get_previous_values, get_provider, get_storage_proofs,
};
use eyre::Result;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sp1_sdk::{
    Elf, HashableKey, ProveRequest, Prover, ProverClient, ProvingKey, SP1ProofWithPublicValues,
    SP1Stdin, SP1VerifyingKey, include_elf,
    network::{NetworkMode, signer::NetworkSigner},
    utils::setup_logger,
};
use tokio::time::{self, Duration};

/// The ELF (executable and linkable format) file for the Succinct RISC-V zkVM.
pub const ENERGY_TRACKER_ELF: Elf = include_elf!("energy-tracker-program");

#[derive(Debug, Clone, Serialize, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProofFixture {
    previous_balances: B256,
    previous_nonces: B256,
    new_balances: Bytes,
    new_nonces: Bytes,
    block_hash: B256,
    vkey: String,
    public_values: String,
    proof: Bytes,
}

type DbPool = r2d2::Pool<ConnectionManager<PgConnection>>;

#[derive(Queryable, QueryableByName, Insertable, Serialize, Debug)]
struct M3terPayload {
    id: i32,
    m3ter_id: i64,
    message: String,
    signature: String,
    nonce: i64,
    energy: i64,
    is_verified: bool,
}

#[derive(Insertable, Deserialize)]
#[diesel(table_name = m3ter_payloads)]
struct NewM3terPayload {
    m3ter_id: i64,
    message: String,
    signature: String,
    nonce: i64,
    energy: i64,
    is_verified: bool,
}

table! {
    m3ter_payloads (id) {
        id -> Int4,
        m3ter_id -> Int8,
        message -> VarChar,
        signature -> VarChar,
        nonce -> Int8,
        energy -> Int8,
        is_verified -> Bool,
    }
}

fn get_query_string() -> String {
    let query_string = "SELECT *
            FROM m3ter_payloads
            WHERE is_verified = FALSE
            AND m3ter_id <> 7
            ORDER BY m3ter_id ASC, nonce ASC
        ";
    let limit = env::var("QUERY_LIMIT").unwrap_or_else(|_| "".to_string());
    let limit = if !limit.is_empty() {
        println!("limit value {}", limit);
        match limit.parse::<u64>() {
            Ok(l) => format!("{} LIMIT {}", query_string, l),
            Err(_) => query_string.to_string(),
        }
    } else {
        query_string.to_string()
    };
    println!("limit query {}", limit);
    limit
}

#[tokio::main]
async fn main() {
    dotenvy::dotenv().ok();
    sp1_sdk::utils::setup_logger();
    // Define a simple route
    println!("connecting to database...");
    let db_pool = establish_db_connection();
    let db_state = Arc::new(db_pool.clone());
    println!("connected to database");

    tokio::spawn(async move {
        let duration = env::var("BLOCK_INTERVAL")
            .unwrap_or_else(|_| String::from("10000"))
            .parse::<u64>()
            .unwrap_or(10000);
        let mut interval = time::interval(Duration::from_secs(duration));
        loop {
            interval.tick().await;
            match db_pool.get() {
                Ok(mut conn) => {
                    _ = update_payload(&mut conn).await;
                    let proving_payload = sql_query(get_query_string())
                        .load::<M3terPayload>(&mut conn)
                        .expect("Failed to load payloads");
                    if proving_payload.is_empty() {
                        println!("No new payloads to process");
                        continue;
                    }
                    let mut grouped: HashMap<String, Vec<energy_tracker_lib::M3terPayload>> =
                        HashMap::new();
                    for payload in &proving_payload {
                        grouped
                            .entry(payload.m3ter_id.to_string())
                            .or_default()
                            .push(energy_tracker_lib::M3terPayload::new(
                                payload.message.clone(),
                                payload.signature.clone(),
                                payload.nonce as u64,
                                payload.energy as u64,
                            ));
                    }
                    for (k, v) in &grouped {
                        println!("m3ter {}, with payload length {}", k, v.len());
                    }
                    println!("========start running prover=============");
                    let (_, hash) = match run_prover(grouped, "groth16").await {
                        Ok(res) => res,
                        Err(e) => {
                            eprintln!("Prover error: {}", e);
                            return;
                        }
                    };
                    println!("Committed state with tx hash: {}", hash);
                    let _ = update_payload(&mut conn).await;
                }
                Err(e) => {
                    eprintln!("Failed to get DB connection: {:?}", e);
                    break;
                }
            }
        }
    });

    let port = env::var("PROVER_NODE_PORT").unwrap_or_else(|_| "8080".to_string());
    let addr = format!("0.0.0.0:{}", port);

    let app = Router::new()
        .route("/", get(root))
        .route("/batch-payloads", post(batch_payload_handler))
        .route("/health", get(health))
        .route("/run_prover", get(run_prover_handler))
        .route("/vkey", get(get_prover_vkey))
        .route(
            "/update_verified_payloads",
            get(update_verified_payloads_handler),
        )
        .with_state(db_state);

    println!("Starting server on http://localhost:{}", port);
    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    axum::serve::serve(listener, app)
        .await
        .expect("server should start");
}

fn establish_db_connection() -> DbPool {
    let database_url = env::var("DATABASE_URL").expect("DATABASE_URL not set in .env");
    let manager = ConnectionManager::<PgConnection>::new(&database_url);
    r2d2::Pool::builder()
        .build(manager)
        .expect("Failed to create pool.")
}

// Handler function
async fn root() -> Json<serde_json::Value> {
    Json(json!({ "message": "Hello, world!" }))
}

async fn health(State(db_state): State<Arc<DbPool>>) -> Json<serde_json::Value> {
    let connection = db_state.get().is_ok();
    let code = if connection { 200 } else { 500 };
    Json(json!({ "code": code, "success": code == 200 }))
}

async fn run_prover_handler(
    State(db_state): State<Arc<DbPool>>,
    Query(params): Query<HashMap<String, String>>,
) -> (StatusCode, Json<serde_json::Value>) {
    let proof_type = params
        .get("proof_type")
        .map(|s| {
            if s != "plonk" && s != "groth16" {
                "groth16".to_string()
            } else {
                s.clone()
            }
        })
        .unwrap_or("groth16".to_string());

    let mut conn = match db_state.get() {
        Ok(state) => state,
        Err(e) => {
            return (
                StatusCode::BAD_GATEWAY,
                Json(json!({ "error": true, "message": format!("encountered error {:?}", e) })),
            );
        }
    };

    tokio::spawn(async move {
        
        let _ = update_payload(&mut conn).await;
        let proving_payload = match sql_query(get_query_string()).load::<M3terPayload>(&mut conn) {
            Ok(p) => p,
            Err(e) => {
                eprintln!("Failed to load payloads: {:?}", e);
                return;
            }
        };

        if proving_payload.is_empty() {
            println!("No payloads to process");
            return;
        }

        let mut grouped: HashMap<String, Vec<energy_tracker_lib::M3terPayload>> = HashMap::new();
        for payload in &proving_payload {
            grouped
                .entry(payload.m3ter_id.to_string())
                .or_default()
                .push(energy_tracker_lib::M3terPayload::new(
                    payload.message.clone(),
                    payload.signature.clone(),
                    payload.nonce as u64,
                    payload.energy as u64,
                ));
        }

        let (_, hash) = match run_prover(grouped, &proof_type).await {
            Ok(res) => res,
            Err(e) => {
                eprintln!("Prover error: {}", e);
                return;
            }
        };
        println!("Committed state with tx hash: {}", hash);
        let _ = update_payload(&mut conn).await;
        println!("Updated payloads as verified");
    });

    (
        StatusCode::OK,
        Json(json!({
            "code": 200,
            "message": "Prove generation started..."
        })),
    )
}

async fn get_prover_vkey() -> Json<serde_json::Value> {
    let private_key = env::var("PRIVATE_KEY").expect("PRIVATE_KEY not set in .env");
    let prover = ProverClient::builder()
        .network_for(NetworkMode::Mainnet)
        .private_key(&private_key)
        .build()
        .await;
    let pk = prover.setup(ENERGY_TRACKER_ELF).await.unwrap();
    Json(json!({
        "vkey": pk.verifying_key().bytes32()
    }))
}

async fn batch_payload_handler(
    State(db_state): State<Arc<DbPool>>,
    Json(payloads): Json<Vec<Value>>,
) -> (StatusCode, Json<serde_json::Value>) {
    struct PayloadItem {
        pub m3ter_id: i64,
        pub message: String,
    }

    impl From<Value> for PayloadItem {
        fn from(value: Value) -> Self {
            Self {
                m3ter_id: value["m3ter_id"].as_i64().unwrap(),
                message: String::from(value["message"].as_str().unwrap_or("")),
            }
        }
    }
    let mut connection = db_state.get().unwrap();
    let received_count = payloads.len();

    let new_payloads = payloads
        .into_iter()
        .map(PayloadItem::from)
        .filter(|item| {
            is_unique_nonce(&mut connection, item.m3ter_id, extract_nonce(&item.message))
        })
        .map(|payload| {
            let m3ter_id = payload.m3ter_id;
            let (message, signature, nonce, energy) = destructure_payload(&payload.message);
            NewM3terPayload {
                m3ter_id,
                message: message.to_string(),
                signature: signature.to_string(),
                nonce: nonce as i64,
                energy: energy as i64,
                is_verified: false,
            }
        })
        .collect::<Vec<NewM3terPayload>>();

    println!("Inserting payload");
    let inserted: Vec<M3terPayload> = diesel::insert_into(m3ter_payloads::table)
        .values(&new_payloads)
        .get_results(&mut connection)
        .expect("Failed to insert payload");

    println!("Inserted payload: {:?}", inserted);
    (
        StatusCode::OK,
        Json(
            json!({ "inserted": inserted, "nonces_inserted": inserted.len(), "nonces_repeated": received_count - inserted.len() }),
        ),
    )
}

async fn update_verified_payloads_handler(
    State(db_state): State<Arc<DbPool>>,
) -> (StatusCode, Json<serde_json::Value>) {
    let mut connection = db_state.get().unwrap();
    let _ = update_payload(&mut connection).await;

    (
        StatusCode::OK,
        Json(json!({
            "code": 200,
            "success": true
        })),
    )
}

async fn run_prover(
    payload: HashMap<String, Vec<energy_tracker_lib::M3terPayload>>,
    proof_type: &str,
) -> Result<(ProofFixture, String)> {
    setup_logger();
    let provider = get_provider().await.unwrap();
    let private_key = env::var("PRIVATE_KEY").expect("PRIVATE_KEY not set in .env");
    let signer = NetworkSigner::local(&private_key).unwrap();
    if !check_gas_balance(&provider).await.unwrap() {
        println!("Insufficient gas balance to proceed with proving.");
        return Err(eyre::eyre!("Insufficient gas balance"));
    }
    println!("=============setting up prover client===============");
    let prover_client = ProverClient::builder()
        // .cpu()
        .network_for(NetworkMode::Mainnet)
        .signer(signer)
        .build()
        .await;

    let pk = prover_client.setup(ENERGY_TRACKER_ELF).await.unwrap();
    let vk = pk.verifying_key();
    println!("=============proceeding to build program inputs===============");
    let (payload, _) = match build_proving_payload(&provider, payload, vk).await {
        Ok(res) => res,
        Err(e) => return Err(eyre::eyre!("Failed to build payload: {:?}", e)),
    };

    let mut stdin = SP1Stdin::new();
    stdin.write(&payload);
    // println!("====starting execution report======");
    // let (_, report) = prover_client
    //     .execute(ENERGY_TRACKER_ELF, stdin.clone())
    //     .await
    //     .unwrap();
    // println!("report {:?}", report);
    println!("====starting proof generation======");
    let proof = match match proof_type {
        "plonk" => {
            prover_client
                .prove(&pk, stdin)
                .skip_simulation(true)
                .plonk()
                .await
        }
        "groth16" => {
            prover_client
                .prove(&pk, stdin)
                .skip_simulation(true)
                .groth16()
                .await
        }
        _ => panic!("Unsupported proof type: {}", proof_type),
    } {
        Ok(proof) => proof,
        Err(e) => return Err(eyre::eyre!("Prover error: {:?}", e)),
    };

    let (proof_fixture, err) = create_proof_fixture(&proof, vk);
    if err.is_some() {
        return Err(eyre::eyre!("Failed to create proof fixture: {:?}", err));
    }
    println!("Proof generated successfully proof = {:?}", &proof_fixture);

    println!("Committing state ...");
    let hash = match commit_state(
        &provider,
        &proof_fixture.new_balances,
        &proof_fixture.new_nonces,
        &proof_fixture.proof,
    )
    .await
    {
        Ok(tx_hash) => tx_hash,
        Err(e) => return Err(eyre::eyre!("Failed to commit state: {:?}", e)),
    };
    // unimplemented!("go back")
    Ok((proof_fixture, hash.to_string()))
}

async fn build_proving_payload(
    provider: &impl Provider,
    payload: HashMap<String, Vec<energy_tracker_lib::M3terPayload>>,
    vk: &SP1VerifyingKey,
) -> Result<(Payload, B256)> {
    let previous_nonces = get_previous_values(&provider, U256::from(1)).await.unwrap();
    let previous_balances = get_previous_values(&provider, U256::from(0)).await.unwrap();
    
    let previous_nonces = if previous_nonces.len() > 2 { previous_nonces } else { Bytes::new() };
    let previous_balances = if previous_balances.len() > 2 { previous_balances } else { Bytes::new() };

    let slot_keys = payload
        .keys()
        .map(|key| {
            let m3ter_id: u64 = key.parse().expect("meter id not valid");
            m3ter_id
        })
        .map(|m3ter_id| to_b256(calc_slot_key(U256::from(m3ter_id)).unwrap()))
        .collect();

    let (account_proof, encoded_account, storage_hash, proofs, anchor_block) =
        get_storage_proofs(&provider, slot_keys).await.unwrap();
    let block_bytes = get_block_rpl_bytes(&provider, anchor_block).await.unwrap();
    if !check_program_vkey(&provider, vk.bytes32_raw())
        .await
        .unwrap()
    {
        return Err(eyre::eyre!(
            "Program Vkey does not match the on-chain value"
        ));
    }
    println!("Loaded payloads: {:?}", payload);
    println!("Anchor Block: {:?}", anchor_block);
    Ok((
        Payload {
            mempool: payload,
            previous_nonces: previous_nonces.into(),
            previous_balances: previous_balances.into(),
            proofs: ProofStruct {
                account_proof,
                encoded_account,
                storage_hash,
                proofs,
            },
            block_bytes,
        },
        anchor_block,
    ))
}

async fn update_payload(
    connection: &mut PooledConnection<ConnectionManager<PgConnection>>,
) -> Result<()> {
    use self::m3ter_payloads::dsl::*;
    use diesel::prelude::*;
    #[derive(Debug)]
    enum DataStrategy {
        Persist,
        Delete,
    }
    let strategy: DataStrategy = match env::var("DATA_STRATEGY").unwrap().as_str() {
        "persist" => DataStrategy::Persist,
        "delete" => DataStrategy::Delete,
        _ => DataStrategy::Persist,
    };
    let provider = get_provider().await.expect("Failed to get provider");
    let nonces = get_previous_values(&provider, U256::from(1)).await.unwrap();
    let nonces_in_db = m3ter_payloads
        .select(m3ter_id)
        .distinct()
        .load::<i64>(connection)?;
    nonces_in_db.iter().for_each(|m3ter| {
        let n = *m3ter as usize;
        let start = n * 6;
        let end = n * 6 + 6;
        let nonce_value = decode_slice(&nonces[start..end].try_into().unwrap()) as i64;
        let rows = match strategy {
            DataStrategy::Persist => diesel::update(m3ter_payloads)
                .filter(m3ter_id.eq(m3ter))
                .filter(nonce.le(nonce_value))
                .set(is_verified.eq(true))
                .execute(connection),
            DataStrategy::Delete => diesel::delete(m3ter_payloads)
                .filter(m3ter_id.eq(m3ter))
                .filter(nonce.le(nonce_value))
                .execute(connection),
        };

        println!(
            "rows updated {} with strategy {:?}",
            rows.unwrap_or_default(),
            strategy
        );
    });
    Ok(())
}

fn is_unique_nonce(
    connection: &mut PooledConnection<ConnectionManager<PgConnection>>,
    i_m3ter_id: i64,
    i_nonce: i64,
) -> bool {
    use self::m3ter_payloads::dsl::*;
    use diesel::prelude::*;

    match m3ter_payloads
        .filter(m3ter_id.eq(i_m3ter_id).and(nonce.eq(i_nonce)))
        .first::<M3terPayload>(connection)
    {
        Ok(_) => {
            println!(
                "Nonce {} for m3ter {} already exists in the database",
                i_nonce, i_m3ter_id
            );
            false
        }
        Err(_) => {
            println!("Nonce {} for m3ter {} is unique", i_nonce, i_m3ter_id);
            true
        }
    }
}

fn create_proof_fixture(proof: &SP1ProofWithPublicValues, vk: &SP1VerifyingKey) -> (ProofFixture, Option<String>) {
    let bytes = proof.public_values.as_slice();
if bytes.len() < 96 {
    let s = String::from_utf8_lossy(bytes);
    println!("Public values too short ({} bytes): {}", bytes.len(), s);
    return (ProofFixture::default(), Some(format!("Public values too short ({} bytes): {}", bytes.len(), s)));
}
    let output = PublicValuesStruct::from_bytes(bytes);
    let PublicValuesStruct {
        previous_balances,
        previous_nonces,
        new_balances,
        new_nonces,
        block_hash,
    } = output;

    // Create the testing fixture so we can test things end-to-end.
    (ProofFixture {
        previous_balances,
        previous_nonces,
        new_balances,
        new_nonces,
        block_hash,
        vkey: vk.bytes32().to_string(),
        public_values: format!("0x{}", hex::encode(bytes)),
        proof: proof.bytes().into(),
    }, None)
}
