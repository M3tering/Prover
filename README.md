# M3tering Prover 🐻‍❄️

The prover validates signed meter actions, aggregates energy and produces SP1 proofs for sparse [Rollup](https://github.com/M3tering/Rollup) state updates. The host reads only selected storage words and signing keys, plans a batch that fits its gas budget, proves the transition, and submits the resulting absolute replacements.

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

The host pins its RPC reads to one canonical block hash for consistency, but that block is not committed and imposes no expiry. Proofs for disjoint words can coexist. Proofs sharing a word conflict, even when they change different meters in that word. A proof cannot replay after its own successful transition because old words and nonces have changed.

The contract writes in one loop before a static verifier call. Verification failure rolls back all writes and length changes. After success, `StateCommitted(bytes updates)` emits the exact payload for indexers.

## Build and test

Use Rust stable and SP1 **6.3.1**. The proving-enabled host needs protobuf/protoc, pkg-config and OpenSSL development headers. Install SP1 from the official Succinct distribution, pinning `sp1up --version v6.3.1`.

```sh
cargo fmt --all -- --check
cargo test --locked -p m3tering-protocol -p m3tering-prover
cargo build --locked -p m3tering-prover --features proving
cargo prove build --locked -p m3tering-program --output-directory elf
SP1_ELF=elf/m3tering-program WITNESS_FILE=fixtures/input.json \
  cargo run --locked -p m3tering-prover --features proving -- execute-fixture
```

`execute-fixture` executes the actual guest locally, verifies that its public output matches the shared transition code and prints the program verification key. It neither buys a proof nor submits an Ethereum transaction. Re-derive the key after changing the guest, dependencies or toolchain. Set that key and the target-chain verifier/M3ter addresses in the Rollup constants before deployment.

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

Each invocation handles one batch. Schedule subsequent invocations with unique output directories. Keep one submitter per wallet/deployment unless you provide distributed coordination. A local file lock coordinates this host on one machine only.

`run` checks the deployed program key against the local ELF before buying a proof. It locally executes the guest, validates the expected public output, bounds cycles/PGU, checks SP1 credits, and applies an explicit network price cap and timeout. Ethereum `PRIVATE_KEY` and `SP1_PRIVATE_KEY` are separate credentials. Never commit real credentials.

Before submitting, the host verifies the returned proof, checks current words/used keys again, simulates the call, estimates gas, adds 20% headroom and checks the signer's pending ETH balance using maximum fees. Only a successful, sufficiently confirmed, still-canonical receipt allows selected consumed database messages to be marked verified.

Proofs and transaction receipts are saved. Output directories containing `proof.bin` or `transaction.json` cannot be overwritten. Preserve a paid proof after a send failure; it can be reused only while its old words and used keys remain current. There is no automatic paid retry, queue quarantine, distributed reservation service or submit-only recovery daemon. Malformed entries, nonce gaps and overflow can require operator attention.

### Streamr ingestion

The ingestion client subscribes to `STREAM_ID` and saves action bytes/signatures in PostgreSQL. It can run independently of proving:

```sh
cd streamr-client
npm ci
# Set DATABASE_URL and STREAM_ID in the environment or .env.
# STREAMR_CLIENT_PORT defaults to 3000.
npm start
```

Startup applies the checked-in database migration. `/health` serves the process health endpoint. Queue ingestion does not authenticate an energy transition; proof generation and on-chain verification do that.

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
