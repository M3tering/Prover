# M3tering Prover 🐻‍❄️

The prover validates signed meter actions, aggregates energy and produces SP1 proofs for sparse [Rollup](https://github.com/M3tering/Rollup) state updates. The host reads only selected storage words and signing keys, plans a batch that fits its gas budget, proves the transition, and submits the resulting absolute replacements.

## Run a node

Install Docker Engine / Docker Desktop with Compose. From this repository:

```sh
./node setup
./node start
./node status
```

Setup asks for the RPC URL, deployed Rollup address, Ethereum/SP1 credentials, optional Streamr stream, and proof interval. Secrets are saved in a private, git-ignored `.env`; do not share that file. The normal installation pulls images built by GitHub Actions; it does not compile Rust or SP1 locally. `./node update` pulls the newest published branch images. Pin `PROVER_IMAGE` and `API_IMAGE` to the release's `@sha256:...` digests for an exact deployment, rather than following a moving branch tag.

The publishing workflow `.github/workflows/images.yml` runs protocol/ingestion tests, builds both Docker targets on GitHub-hosted runners, executes the real SP1 guest in the worker image, and publishes only after its checks pass. Images are `ghcr.io/m3tering/prover:propter-polar-bear` (worker) and `ghcr.io/m3tering/prover:propter-polar-bear-api` (API), with immutable commit-specific `sha-COMMIT` tags as well. Publishing uses GitHub's built-in token; it needs no Ethereum key or proving credits. It never buys a proof. Existing production `main` image tags are not overwritten by this workflow.

To build from source instead, run `./node build`, then `./node start`. Source builds are large: prefer at least 8 GB RAM and 15 GB free disk space. The checked-in Dockerfile pins base-image digests; Rust 1.99.1, Cargo.lock, package-lock.json and SP1 6.3.1 pin application inputs. The guest ELF is checked in with its SHA-256 and program vkey manifest and verified during the image build. OS security packages are resolved at image build time; published image digests identify the exact runtime. Images target Linux amd64; Docker can emulate this on ARM machines, with a performance penalty. `WORKER_MEMORY` sets the worker memory limit (default 4 GB).

Compose runs three persistent services: PostgreSQL, an HTTP/Streamr ingestion API, and the scheduled proving worker. The API does **not** receive the Ethereum/SP1 spending keys. PostgreSQL is internal; HTTP binds only to `127.0.0.1:19000`. Database and proof artifacts survive restarts and `docker compose down`; **do not use `down -v`** unless intentionally deleting all state. Each database is bound to one chain and Rollup address.

For public HTTPS, point a hostname at your server and place nginx or another TLS proxy in front of port 19000. `ops/test-prover.nginx.conf` is the configuration for `test-prover.m3ter.ing`. Obtain a trusted certificate with Certbot. Do not expose a plaintext public port as a substitute for HTTPS.

```sh
./node logs api       # ingestion / Streamr status
./node logs worker    # schedule and batch status
./node network        # read SP1 credit balance and quoted price
./node backup         # PostgreSQL backup under backups/
./node stop           # retains all data
```

Also back up the `proofs` Docker volume, which contains paid requests, proofs, signed transactions and receipts. Restore the database and proof files together. Inspect batch `host.log` files in that volume for detailed preflight failures. No public endpoint can trigger spending; the worker is controlled by private configuration.

### Receiving actions

Both transports accept the same JSON array used by the previous deployment:

```json
[{"m3ter_id": 7, "message": "<144 hex characters: 8 signed bytes followed by 64 signature bytes>"}]
```

POST it to `https://YOUR_HOST/batch-payloads` with `Content-Type: application/json`, or publish it to `STREAM_ID`. The two signed uint32 fields are big-endian nonce and energy in mWh. The API checks exact byte lengths, meter/nonce capacity, Ed25519 signatures against the **current Rollup key**, and already-consumed nonces before storing anything. It derives all accounting fields from the signed bytes. The whole batch is atomic; an invalid action rejects it. Identical retries deduplicate, conflicting actions at the same meter nonce reject, and the database unique constraint also handles concurrent arrivals. HTTP returns 202 with accepted/duplicate counts, 400 for invalid data, 413 for oversized bodies, and 429/503 for retryable congestion or RPC failures. Default limits are 1,000 actions and 256 KiB per HTTP batch. Streamr failures are logged; publishers should retain/retry data or use HTTPS acknowledgements when delivery confirmation is needed.

`GET /health` checks the API/database and reports chain, Rollup and Streamr status; it does not claim that a paid proof has succeeded. A fresh Rollup starts every account and nonce at zero. Meter owners must call `setPublicKey(id,key)`, then meters must start with nonce **1**. Old high-nonce traffic is not a substitute for a genesis checkpoint. The new node uses an independent database. Signatures cover only nonce and energy, without a chain, Rollup address or genesis identifier. Reusing a signing key after resetting nonces therefore allows historical signatures to validate again when their nonces fit. Use a new signing key for a production reset, or explicitly design a versioned signing domain; the requested Sepolia test reset uses the supplied existing meter 7 key.

### Scheduling, budgets and recovery

Set `PROVING_ENABLED=true` to permit paid work. `PROVE_INTERVAL_SECONDS` controls the delay between runs (default 3,600 seconds). The worker checks the queue on startup, serializes its batches, automatically halves the action cap after a local cycle/PGU admission failure (before any purchase), and checks signatures/nonces again before selecting eligible actions. Nonce gaps wait without blocking unrelated meters. Consumed queue entries are retained for audit. A confirmed/finalized chain read is used for queue cleanup; the proof host performs its own current-state checks.

The host chooses a batch within its mixed fresh/reused slot gas budget, then measures storage-writing gas on the live chain using `eth_estimateGas` with a simulation-only verifier override, shrinks the batch if necessary, reserves verifier overhead and 20% headroom, checks wallet funding and guest identity, and executes locally before purchasing a proof. `MAX_PRICE_PER_PGU=auto` uses the SP1 network's quoted maximum price per PGU, with no added percentage buffer. Set a numeric value for a fixed cap, and optionally `MAX_PROOF_COST_WEI` for an absolute per-proof PROVE limit. Ethereum gas limits do not bound the price of PROVE. Network credit and Ethereum balance checks fail before purchase when insufficient.

A durable marker is written **before** requesting a paid proof. The returned request ID is saved immediately; the worker automatically recovers that same request after a timeout or restart. Saved proofs are reverified and reused while their old state/keys remain current. Ethereum transactions are signed and saved before broadcasting, so recovery rebroadcasts identical bytes/hash. Only a successful, confirmed, canonical receipt marks selected messages consumed. An ambiguous paid request without a saved request ID pauses further buying; inspect the SP1 request history and recover its ID before proceeding. Stale proofs or reverted transactions also require operator review instead of automatic repurchase.

```sh
./node recover /data/batches/BATCH_DIRECTORY
```

Gas, cycle/PGU, fee and credit limits remain configurable in `.env`/Compose. A valid proof cannot guarantee inclusion if another writer changes state or fees change. Use one worker and one submission wallet for this deployment; unrelated wallet activity is not coordinated.

## Repository layout

- `lib`: signed-action accounting, packed state and the shared statement/wire codec.
- `program`: SP1 guest that executes the transition and commits its 32-byte public value.
- `host`: RPC/database/file inputs, gas/funding admission, local execution, network proving and transaction submission.
- `fixtures`: deterministic signed-action input and matching contract encoding.
- `streamr-client`: gossip ingestion and PostgreSQL schema for the host's queue.

The workspace packages are `m3tering-protocol`, `m3tering-program` and `m3tering-prover`. Protocol documentation lives in this README.

## Protocol

Four meter records share one uint256 word. Meter `id` uses slot `id/4` and lane `id%4`. Each lane contains a uint40 Wh account in its low bits and a uint24 nonce in its high bits. The slot range is uint32, covering meter IDs 0–17,179,869,183.

An action signs exactly eight bytes: uint32 big-endian nonce followed by uint32 big-endian **mWh**. Ed25519 signatures are verified over these exact bytes. Consumed nonces are skipped; a gap or invalid signature stops that meter's accepted prefix. Malformed encodings and field overflows reject the witness.

The guest aggregates accepted mWh per meter and adds `floor(sum/1000)` Wh once per commit. Fractional Wh are discarded between commits. Zero-Wh increments still advance the nonce. Account and nonce limits are 1,099,511,627,775 Wh and 16,777,215. Untouched lanes are preserved, and all selected meters sharing a word produce one update for that word.

Every public key stays fully readable in Rollup storage. Only the current `M3ter.ownerOf(id)` can change it. Zero revokes. Keys are authenticated at commit time; rotating a used key invalidates a pending proof. The prover performs no MPT verification or block-header decoding.

### Wire format

`commitState(bytes updates,bytes proof)` receives strictly ascending **36-byte records**:

```
uint32 slot || uint256 newWord
```

Both integers are big endian. Every changed slot has its own four-byte ID and absolute replacement. There are no deltas, modes, block anchors or global sequence parameters.

The guest's public output is `keccak256(transcript)` (32 bytes). The transcript contains chain ID (32 bytes), Rollup address (20), then slot (4), old word (32), new word (32), used-key hash (32) for each update. The prefix is 52 bytes and each record is 100 bytes.

The used-key hash covers four 32-byte entries: the full key for a changed lane and zero for an unchanged lane. The contract reconstructs this transcript from its live old words/keys and the proposed replacements. Old words and keys do not appear in commit calldata. Destination fields prevent proof reuse on another chain/deployment; old words reject stale updates; indices, new words and keys authenticate the exact transition. Count is implicit and the configured SP1 key identifies the program.

The RPC must support EIP-1898 hash-pinned calls and `eth_estimateGas` state overrides. The latter changes only a simulation; real commits always use the actual verifier.

The host pins its RPC reads to one canonical block hash for consistency, but that block is not committed and imposes no expiry. Proofs for disjoint words can coexist. Proofs sharing a word conflict, even when they change different meters in that word. A proof cannot replay after its own successful transition because old words and nonces have changed.

The contract writes in one loop before a static verifier call. Verification failure rolls back all writes and length changes. After success, `StateCommitted(bytes updates)` emits the exact payload for indexers.

## Build and test

Use Rust **1.99.1** and SP1 **6.3.1**. The proving-enabled host needs protobuf/protoc, pkg-config and OpenSSL development headers. Install SP1 from the official Succinct distribution, pinning `sp1up --version v6.3.1`.

```sh
cargo fmt --all -- --check
cargo test --locked -p m3tering-protocol -p m3tering-prover
cargo build --locked -p m3tering-prover --features proving
cargo prove build --locked -p m3tering-program --output-directory elf
SP1_ELF=elf/m3tering-program WITNESS_FILE=fixtures/input.json \
  cargo run --locked -p m3tering-prover --features proving -- execute-fixture
```

`execute-fixture` executes the actual guest locally, verifies that its public output matches the shared transition code and prints the program verification key. It neither buys a proof nor submits an Ethereum transaction. Re-derive the key after changing the guest, dependencies or toolchain. Set that key and the target-chain verifier/M3ter addresses before deployment. The Sepolia Rollup permits only its deploying wallet to change the vkey and disables that setter on other chains.

To regenerate the cross-language fixtures:

```sh
FIXTURE_DIR="$PWD/fixtures" cargo test --locked -p m3tering-protocol --test protocol end_to_end_and_export
```

Copy `fixtures/encoding.json` to the Rollup repository's `test/fixtures/statement.json` when releasing a coordinated codec change. Unit/integration tests cover signatures, mWh/Wh conversion, truncation, capacities, shared words, statement bindings and four-byte slots. Budget tests check the largest admitted homogeneous batch and rejection of one additional slot. CI also builds and executes the guest.

## Inputs and operation

Copy `.env.example` values into your process environment. The Rust host does not automatically load dotenv files. Set `RPC_URL`, `ROLLUP_ADDRESS`, a unique `OUTPUT_DIR`, and exactly one input source:

- `ACTIONS_FILE`: JSON object keyed by decimal meter IDs, with arrays of `{ "message": "8-byte hex", "signature": "64-byte hex" }`.
- `DATABASE_URL`: PostgreSQL queue using `streamr-client/db/migrations/initial_00.sql`.

Actions must be ordered by nonce per meter. Accounting always parses nonce/energy from the signed bytes, never trusts separate database metadata for those values. The host reads `wordCount()` before `words(slot)`; unallocated slots are zero because the generated array getter reverts beyond its length.

```sh
# Read state, select a fitting batch and save input.json/transition.json:
cargo run --locked -p m3tering-prover -- plan

# Buy a network proof and submit only after admission checks pass:
cargo run --locked -p m3tering-prover --features proving -- run
```

Each CLI invocation handles one batch. The packaged worker schedules invocations with unique output directories. Keep one submitter per wallet/deployment unless you provide distributed coordination. A local file lock coordinates this host on one machine only.

`run` checks the deployed program key against the local ELF before buying a proof. It locally executes the guest, validates the expected public output, bounds cycles/PGU, checks SP1 credits, and applies an explicit network price cap and timeout. Ethereum `PRIVATE_KEY` and `SP1_PRIVATE_KEY` are separate credentials. Never commit real credentials.

Before submitting, the host verifies the returned proof, checks current words/used keys again, simulates the call, estimates gas, adds 20% headroom and checks the signer's pending ETH balance using maximum fees. Only a successful, sufficiently confirmed, still-canonical receipt allows selected consumed database messages to be marked verified.

Proofs, request IDs, signed transactions and receipts are saved. Use the packaged worker or `recover` to resume a paid attempt. Never delete an unfinished attempt to bypass its safety checks. Standalone host database input requires the current ingestion migrations and `queue_status=ready` preparation by the worker.

## Gas admission and capacity

`MAX_SLOTS=4096` is a selection ceiling, not a promise that 4,096 updates fit in one transaction. The host stops selecting slots when its **mixed** fresh/reused cost estimate exceeds either execution or total gas budget. Selection happens before local proving and paid network requests. It also bounds action count, cycles and PGU.

For K reused words and Z fresh words, with F fixed overhead and H per-word overhead:

```
current gas = F + H*(K+Z) + 5,000*K + 22,100*Z
forecast execution = F + H*(K+Z) + 12,200*(K+Z)
forecast total = forecast execution + 97,920*Z
```

Defaults: F=1.5m, H=12,000, both budgets=15m. Every result receives 20% headroom, rounded upward. Current total gas is clamped to the transaction/execution cap; budgets also respect the observed block limit. The allowance covers four cold key reads per changed word, loop/transcript work, calldata and event costs. The fixed allowance reserves verifier/proof/array-length overhead and requires deployment-specific calibration.

| Schedule | Default all-reused maximum | Default all-fresh maximum |
|---|---:|---:|
| `current` | 647 | 322 |
| `glamsterdam-model` | 454 | 90 |

Four-byte IDs add one payload byte per slot. At 512 slots, local receipts increased by 6,682 gas over three-byte IDs; the existing conservative overhead still covers it, so admission defaults are unchanged. Actual mock-verifier ceilings at 16,777,216 gas were 1,117 reused / 521 fresh words for ordinary fixture values, or 1,100 / 517 with all-nonzero new words. These exclude real SP1 verification and are **not** production limits. The Rollup README and benchmark JSON contain the receipt measurements.

The Glamsterdam model uses the stated [EIP-8037](https://eips.ethereum.org/EIPS/eip-8037) and [EIP-8038](https://eips.ethereum.org/EIPS/eip-8038) prices; selecting it does not activate a fork. Choose the schedule and allowances appropriate to the target chain. Higher total state-gas budgets can admit more fresh words only where chain rules permit; execution limits still apply.

A valid proof does not guarantee sufficient gas or inclusion. Reorgs, rotations, concurrent commits, fee changes and other wallet spending can race preflight checks. Oversized local execution is rejected before buying a proof; lower caps and replan. Deployment validation must include a real proof against the intended verifier and gas calibration on the actual target client.
