mod budget;
use alloy::{
    providers::{Provider, ProviderBuilder},
    rpc::types::TransactionRequest,
    signers::local::PrivateKeySigner,
};
use alloy_primitives::{keccak256, Address, Bytes, B256, U256};
use alloy_sol_types::{sol, SolCall};
use anyhow::{anyhow, bail, ensure, Context, Result};
use fs2::FileExt;
use m3tering_protocol::transition::{self, Input, Transition};
use m3tering_protocol::SignedAction;
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
    str::FromStr,
};

sol! { function commitState(bytes updates, bytes proof); }
#[derive(Clone)]
struct Rpc {
    url: String,
    client: reqwest::Client,
}
impl Rpc {
    async fn call(&self, method: &str, params: Value) -> Result<Value> {
        // Do not log URLs: they may embed credentials.
        let response = self
            .client
            .post(&self.url)
            .json(&json!({"jsonrpc":"2.0","id":1,"method":method,"params":params}))
            .send()
            .await
            .map_err(|_| anyhow!("RPC transport failed"))?;
        ensure!(
            response.status().is_success(),
            "RPC HTTP error {}",
            response.status()
        );
        let v: Value = response.json().await?;
        if let Some(e) = v.get("error") {
            bail!("{method}: {e}");
        }
        v.get("result")
            .cloned()
            .filter(|v| !v.is_null())
            .ok_or_else(|| anyhow!("{method}: missing result"))
    }
    async fn view(&self, to: Address, name: &str, block: Value) -> Result<U256> {
        let data = format!(
            "0x{}",
            hex::encode(&keccak256(format!("{name}()")).as_slice()[..4])
        );
        quantity(
            &self
                .call("eth_call", json!([{"to":to,"data":data},block]))
                .await?,
        )
    }
    async fn view_arg(
        &self,
        to: Address,
        signature: &str,
        arg: U256,
        block: Value,
    ) -> Result<U256> {
        let mut data = keccak256(signature).as_slice()[..4].to_vec();
        data.extend(arg.to_be_bytes::<32>());
        quantity(
            &self
                .call(
                    "eth_call",
                    json!([{"to":to,"data":format!("0x{}",hex::encode(data))},block]),
                )
                .await?,
        )
    }
}
fn quantity(value: &Value) -> Result<U256> {
    U256::from_str(
        value
            .as_str()
            .ok_or_else(|| anyhow!("expected hex quantity"))?,
    )
    .map_err(Into::into)
}
fn env_u64(name: &str, default: u64) -> Result<u64> {
    Ok(std::env::var(name)
        .map(|s| s.parse())
        .unwrap_or(Ok(default))?)
}
fn required(name: &str) -> Result<String> {
    std::env::var(name).with_context(|| format!("set {name}"))
}
fn budget(block_limit: u64) -> Result<budget::Budget> {
    let post_repricing = match std::env::var("GAS_SCHEDULE").as_deref() {
        Ok("glamsterdam-model") => true,
        Ok("current") | Err(_) => false,
        _ => bail!("GAS_SCHEDULE must be current or glamsterdam-model"),
    };
    let execution_limit = env_u64("EXECUTION_GAS_BUDGET", 15_000_000)?
        .min(16_777_216)
        .min(block_limit);
    let total_limit = env_u64("TOTAL_GAS_BUDGET", 15_000_000)?
        .min(block_limit)
        .min(if post_repricing {
            u64::MAX
        } else {
            execution_limit
        });
    Ok(budget::Budget {
        execution_limit,
        total_limit,
        fixed_execution: env_u64("FIXED_EXECUTION_ALLOWANCE", 1_500_000)?,
        per_word_overhead: env_u64("PER_WORD_OVERHEAD_ALLOWANCE", 12_000)?,
        post_repricing,
    })
}
async fn actions() -> Result<BTreeMap<u64, Vec<SignedAction>>> {
    if let Ok(path) = std::env::var("ACTIONS_FILE") {
        return Ok(serde_json::from_slice(&std::fs::read(path)?)?);
    }
    let (client, connection) =
        tokio_postgres::connect(&required("DATABASE_URL")?, tokio_postgres::NoTls).await?;
    tokio::spawn(async move {
        if let Err(e) = connection.await {
            eprintln!("database connection: {e}");
        }
    });
    let limit = i64::try_from(env_u64("MAX_ACTIONS", 10_000)?)?;
    let rows=client.query("SELECT m3ter_id,message,signature FROM m3ter_payloads WHERE is_verified = false ORDER BY m3ter_id,nonce,id LIMIT $1",&[&limit]).await?;
    let mut result = BTreeMap::<u64, Vec<SignedAction>>::new();
    for row in rows {
        let id: i64 = row.get(0);
        result
            .entry(u64::try_from(id)?)
            .or_default()
            .push(SignedAction {
                message: row.get(1),
                signature: row.get(2),
            });
    }
    Ok(result)
}
async fn plan(rpc: &Rpc, rollup: Address) -> Result<(Input, Transition, budget::Budget)> {
    let mut pending = actions().await?;
    let mut remaining = usize::try_from(env_u64("MAX_ACTIONS", 10000)?)?;
    pending.retain(|_, a| {
        a.truncate(remaining);
        remaining -= a.len();
        !a.is_empty()
    });
    let max = usize::try_from(env_u64("MAX_SLOTS", 4096)?)?;
    ensure!(max > 0 && max <= 4096, "invalid MAX_SLOTS");
    let indices: BTreeSet<u64> = pending.keys().map(|m| m / 4).collect();
    let head = rpc
        .call("eth_getBlockByNumber", json!(["latest", false]))
        .await?;
    let read_hash: B256 = serde_json::from_value(head["hash"].clone())?;
    let tag = json!({"blockHash":read_hash,"requireCanonical":true});
    let chain_id = quantity(&rpc.call("eth_chainId", json!([])).await?)?;
    let b = budget(quantity(&head["gasLimit"])?.try_into()?)?;
    println!("Conservative gas admission: at most {} reused words or {} fresh words (before MAX_SLOTS/action limits)", b.max_homogeneous(false,4096), b.max_homogeneous(true,4096));
    let mut words = BTreeMap::new();
    let mut keys = BTreeMap::new();
    let mut costs = Vec::new();
    let word_count = rpc.view(rollup, "wordCount", tag.clone()).await?;
    ensure!(
        indices.iter().all(|i| *i <= transition::MAX_SLOT),
        "id exceeds uint32 slot"
    );
    for slot in indices.into_iter().take(max) {
        let old = if U256::from(slot) < word_count {
            rpc.view_arg(rollup, "words(uint256)", U256::from(slot), tag.clone())
                .await?
        } else {
            U256::ZERO
        };
        costs.push(old);
        if !b.fits(&costs) {
            costs.pop();
            break;
        }
        let mut group = [B256::ZERO; 4];
        for pair in 0..4 {
            if !pending.contains_key(&(slot * 4 + pair as u64)) {
                continue;
            }
            group[pair] = B256::from(
                rpc.view_arg(
                    rollup,
                    "publicKey(uint64)",
                    U256::from(slot * 4 + pair as u64),
                    tag.clone(),
                )
                .await?
                .to_be_bytes::<32>(),
            );
        }
        words.insert(slot, old);
        keys.insert(slot, group);
    }
    pending.retain(|m, _| words.contains_key(&(m / 4)));
    ensure!(!pending.is_empty(), "no actions fit gas budgets");
    let input = Input {
        chain_id,
        rollup,
        words,
        keys,
        actions: pending,
    };
    let t = transition::execute(&input).map_err(|e| anyhow!(e))?;
    println!(
        "Planned {} words, {} packed payload bytes, 32 public-value bytes",
        t.updates.len(),
        t.payload.len()
    );
    Ok((input, t, b))
}
async fn preflight(rpc: &Rpc, input: &Input, t: &Transition) -> Result<()> {
    let head = rpc
        .call("eth_getBlockByNumber", json!(["latest", false]))
        .await?;
    let tag = json!({"blockHash":head["hash"],"requireCanonical":true});
    let word_count = rpc.view(input.rollup, "wordCount", tag.clone()).await?;
    ensure!(
        quantity(&rpc.call("eth_chainId", json!([])).await?)? == input.chain_id,
        "chain changed"
    );
    for u in &t.updates {
        let current = if U256::from(u.slot) < word_count {
            rpc.view_arg(
                input.rollup,
                "words(uint256)",
                U256::from(u.slot),
                tag.clone(),
            )
            .await?
        } else {
            U256::ZERO
        };
        ensure!(current == u.old, "old word changed");
        for pair in 0..4 {
            if transition::unpack(u.old, pair as u64) == transition::unpack(u.new, pair as u64) {
                continue;
            }
            ensure!(
                B256::from(
                    rpc.view_arg(
                        input.rollup,
                        "publicKey(uint64)",
                        U256::from(u.slot * 4 + pair as u64),
                        tag.clone()
                    )
                    .await?
                    .to_be_bytes::<32>()
                ) == input.keys[&u.slot][pair],
                "key rotated; rebuild proof"
            );
        }
    }
    Ok(())
}
async fn funding(rpc: &Rpc, signer: Address, gas: u64) -> Result<(u128, u128)> {
    let max_fee: u128 = required("MAX_FEE_PER_GAS_WEI")?.parse()?;
    let priority: u128 = required("MAX_PRIORITY_FEE_PER_GAS_WEI")?.parse()?;
    ensure!(priority <= max_fee, "priority fee exceeds max fee");
    let head = rpc
        .call("eth_getBlockByNumber", json!(["latest", false]))
        .await?;
    let base: u128 = quantity(&head["baseFeePerGas"])?.try_into()?;
    ensure!(
        base.checked_add(priority).is_some_and(|f| f <= max_fee),
        "fee cap below current base fee + priority"
    );
    let balance = quantity(
        &rpc.call("eth_getBalance", json!([signer, "pending"]))
            .await?,
    )?;
    let reserve = U256::from_str(&std::env::var("ETH_RESERVE_WEI").unwrap_or_else(|_| "0".into()))?;
    budget::require_funding(balance, gas, max_fee, reserve)?;
    Ok((max_fee, priority))
}
#[cfg(feature = "proving")]
async fn prove_and_send(
    rpc: &Rpc,
    input: &Input,
    t: &Transition,
    b: budget::Budget,
    out: &Path,
) -> Result<()> {
    use sp1_sdk::{
        network::{signer::NetworkSigner, NetworkMode},
        Elf, HashableKey, ProveRequest, Prover, ProverClient, ProvingKey, SP1ProofMode, SP1Stdin,
    };
    let signer = PrivateKeySigner::from_str(&required("PRIVATE_KEY")?)?;
    // An OS lock serializes cooperating processes sharing a submission wallet.
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(std::env::temp_dir().join(format!("m3ter-prover-{}.lock", signer.address())))?;
    lock.try_lock_exclusive()
        .context("another prover batch is using this wallet")?;
    preflight(rpc, input, t).await?;
    let (_, estimated) = b.estimate(&t.updates.iter().map(|u| u.old).collect::<Vec<_>>())?;
    let estimated = estimated
        .checked_mul(6)
        .and_then(|x| x.checked_add(4))
        .map(|x| x / 5)
        .ok_or_else(|| anyhow!("gas budget overflow"))?;
    funding(rpc, signer.address(), estimated).await?;
    let network_key = required("SP1_PRIVATE_KEY")?;
    let network_signer = NetworkSigner::local(&network_key)?;
    let client = ProverClient::builder()
        .network_for(NetworkMode::Mainnet)
        .signer(network_signer)
        .build()
        .await;
    let elf = Elf::from(std::fs::read(required("SP1_ELF")?)?);
    let pk = client.setup(elf.clone()).await?;
    let onchain = rpc
        .view(input.rollup, "SP1_PROGRAM_VKEY", json!("latest"))
        .await?;
    ensure!(
        B256::from(onchain.to_be_bytes::<32>()) == B256::from_str(&pk.verifying_key().bytes32())?,
        "deployed program key differs from prover guest"
    );
    let mut stdin = SP1Stdin::new();
    stdin.write(input);
    // Execute locally before requesting a paid proof. Compare complete public values.
    let (public, report) = client
        .execute(elf, stdin.clone())
        .cycle_limit(env_u64("MAX_PROVER_CYCLES", 100_000_000)?)
        .calculate_gas(true)
        .await?;
    ensure!(
        public.as_slice() == t.public_values.as_slice(),
        "guest/host statement mismatch"
    );
    let cycles = report
        .total_instruction_count()
        .checked_mul(110)
        .and_then(|v| v.checked_div(100))
        .ok_or_else(|| anyhow!("cycle overflow"))?;
    let pgu = report
        .gas()
        .ok_or_else(|| anyhow!("SP1 execution did not report PGU"))?
        .checked_mul(110)
        .and_then(|v| v.checked_div(100))
        .ok_or_else(|| anyhow!("PGU overflow"))?;
    ensure!(
        cycles <= env_u64("MAX_PROVER_CYCLES", 100_000_000)?
            && pgu <= env_u64("MAX_PROVER_PGU", 100_000_000)?,
        "batch exceeds configured proving budget; reduce MAX_SLOTS/MAX_ACTIONS"
    );
    let price = required("MAX_PRICE_PER_PGU")?.parse::<u64>()?;
    let params = client
        .get_proof_request_params(SP1ProofMode::Groth16)
        .await?;
    let params = match params {
        sp1_sdk::network::proto::GetProofRequestParamsResponse::Auction(p) => p,
        _ => bail!("network does not provide auction prices"),
    };
    ensure!(
        params.tick_size == 0 || price % params.tick_size == 0,
        "MAX_PRICE_PER_PGU must be a multiple of the auction tick"
    );
    let base = params.base_fee.parse::<u64>()?;
    let reserve = U256::from_str(&required("SP1_CREDIT_RESERVE_WEI")?)?;
    let cost = U256::from(pgu)
        .checked_mul(U256::from(price))
        .and_then(|x| x.checked_add(U256::from(base)))
        .and_then(|x| x.checked_add(reserve))
        .ok_or_else(|| anyhow!("credit cost overflow"))?;
    ensure!(
        client.get_balance().await? >= cost,
        "insufficient SP1 network credits (PROVE wei)"
    );
    preflight(rpc, input, t).await?;
    funding(rpc, signer.address(), estimated).await?;
    let proof = client
        .prove(&pk, stdin)
        .groth16()
        .cycle_limit(cycles)
        .gas_limit(pgu)
        .max_price_per_pgu(price)
        .timeout(std::time::Duration::from_secs(env_u64(
            "PROOF_TIMEOUT_SECONDS",
            3600,
        )?))
        .await?;
    client.verify(&proof, pk.verifying_key(), None)?;
    ensure!(
        proof.public_values.as_slice() == t.public_values.as_slice(),
        "proof statement mismatch"
    );
    proof.save(out.join("proof.bin"))?;
    preflight(rpc, input, t).await?;
    let data = commitStateCall {
        updates: t.payload.clone().into(),
        proof: proof.bytes().into(),
    }
    .abi_encode();
    let request = json!({"from":signer.address(),"to":input.rollup,"data":format!("0x{}",hex::encode(&data))});
    rpc.call("eth_call", json!([request, "pending"])).await?;
    let actual: u64 =
        quantity(&rpc.call("eth_estimateGas", json!([request])).await?)?.try_into()?;
    let gas = actual
        .checked_mul(120)
        .and_then(|v| v.checked_div(100))
        .ok_or_else(|| anyhow!("gas overflow"))?;
    ensure!(
        gas <= b.total_limit,
        "final gas with 20% headroom exceeds budget; keep proof, do not send"
    );
    let (max_fee, priority) = funding(rpc, signer.address(), gas).await?;
    let provider = ProviderBuilder::new()
        .wallet(signer)
        .connect_http(rpc.url.parse()?);
    let tx = TransactionRequest::default()
        .to(input.rollup)
        .input(Bytes::from(data).into())
        .gas_limit(gas)
        .max_fee_per_gas(max_fee)
        .max_priority_fee_per_gas(priority);
    let pending = provider.send_transaction(tx).await?;
    std::fs::write(
        out.join("transaction.json"),
        serde_json::to_vec_pretty(&json!({"hash":pending.tx_hash()}))?,
    )?;
    let receipt = pending
        .with_required_confirmations(env_u64("CONFIRMATIONS", 2)?)
        .get_receipt()
        .await?;
    ensure!(
        receipt.status(),
        "commit reverted; do not mark actions verified"
    );
    std::fs::write(
        out.join("receipt.json"),
        serde_json::to_vec_pretty(&receipt)?,
    )?;
    println!("Committed {}", receipt.transaction_hash);
    if std::env::var_os("ACTIONS_FILE").is_none() {
        // Only mark the selected messages consumed, and only after a successful,
        // still-canonical confirmed receipt. No new proof => no DB mutation.
        let number = receipt
            .block_number
            .ok_or_else(|| anyhow!("receipt missing block number"))?;
        let canonical = rpc
            .call(
                "eth_getBlockByNumber",
                json!([format!("0x{number:x}"), false]),
            )
            .await?;
        ensure!(
            Some(serde_json::from_value::<B256>(canonical["hash"].clone())?) == receipt.block_hash,
            "receipt reorged before database reconciliation"
        );
        reconcile(input, t).await?;
    }
    Ok(())
}
#[cfg(feature = "proving")]
async fn reconcile(input: &Input, t: &Transition) -> Result<()> {
    let (mut client, connection) =
        tokio_postgres::connect(&required("DATABASE_URL")?, tokio_postgres::NoTls).await?;
    tokio::spawn(async move {
        let _ = connection.await;
    });
    let transaction = client.transaction().await?;
    for (id, actions) in &input.actions {
        let Some(update) = t.updates.iter().find(|u| u.slot == id / 4) else {
            continue;
        };
        let (_, nonce) = transition::unpack(update.new, id % 4);
        for action in actions {
            let bytes = hex::decode(action.message.strip_prefix("0x").unwrap_or(&action.message))?;
            ensure!(bytes.len() == 8, "invalid signed message");
            if u32::from_be_bytes(bytes[..4].try_into()?) <= nonce {
                let id = i64::try_from(*id)?;
                transaction.execute("UPDATE m3ter_payloads SET is_verified=true WHERE m3ter_id=$1 AND message=$2 AND signature=$3 AND is_verified=false",&[&id,&action.message,&action.signature]).await?;
            }
        }
    }
    transaction.commit().await?;
    Ok(())
}
#[cfg(feature = "proving")]
async fn execute_fixture() -> Result<()> {
    use sp1_sdk::{Elf, HashableKey, Prover, ProverClient, ProvingKey, SP1Stdin};
    let input: Input = serde_json::from_slice(&std::fs::read(required("WITNESS_FILE")?)?)?;
    let expected = transition::execute(&input).map_err(|e| anyhow!(e))?;
    let client = ProverClient::builder().light().build().await;
    let elf = Elf::from(std::fs::read(required("SP1_ELF")?)?);
    let pk = client.setup(elf.clone()).await?;
    println!("Program verification key: {}", pk.verifying_key().bytes32());
    let mut stdin = SP1Stdin::new();
    stdin.write(&input);
    let (public, report) = client
        .execute(elf, stdin)
        .cycle_limit(env_u64("MAX_PROVER_CYCLES", 100_000_000)?)
        .calculate_gas(true)
        .await?;
    ensure!(
        public.as_slice() == expected.public_values.as_slice(),
        "SP1 guest public values differ from host"
    );
    println!(
        "SP1 local execution: {} cycles; {:?} PGU",
        report.total_instruction_count(),
        report.gas()
    );
    Ok(())
}
#[tokio::main]
async fn main() -> Result<()> {
    let mode = std::env::args().nth(1).unwrap_or_else(|| "plan".into());
    #[cfg(feature = "proving")]
    if mode == "execute-fixture" {
        return execute_fixture().await;
    }
    ensure!(
        mode == "plan" || mode == "run",
        "usage: prover-rollup-host [plan|run]"
    );
    let rpc = Rpc {
        url: required("RPC_URL")?,
        client: reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(60))
            .build()?,
    };
    let rollup = required("ROLLUP_ADDRESS")?.parse()?;
    let out = std::path::PathBuf::from(
        std::env::var("OUTPUT_DIR").unwrap_or_else(|_| "prover-output".into()),
    );
    std::fs::create_dir_all(&out)?;
    ensure!(!out.join("proof.bin").exists() && !out.join("transaction.json").exists(),"OUTPUT_DIR contains a paid proof or transaction: choose a new directory; do not overwrite it");
    let (input, t, b) = plan(&rpc, rollup).await?;
    std::fs::write(out.join("input.json"), serde_json::to_vec_pretty(&input)?)?;
    std::fs::write(out.join("transition.json"), serde_json::to_vec_pretty(&t)?)?;
    if mode == "run" {
        #[cfg(feature = "proving")]
        prove_and_send(&rpc, &input, &t, b, &out).await?;
        #[cfg(not(feature = "proving"))]
        {
            let _ = b;
            bail!("rebuild with --features proving to submit paid requests");
        }
    }
    Ok(())
}
