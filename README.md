# Energy Tracker SP1 Project

This project is an end-to-end [SP1](https://github.com/succinctlabs/sp1) template for generating proofs of RISC-V programs, tracking energy payloads, and interacting with EVM-compatible proofs.

## Requirements

- [Rust](https://rustup.rs/)
- [SP1](https://docs.succinct.xyz/docs/sp1/getting-started/install)
- [PostgreSQL](https://www.postgresql.org/download/)
- [Diesel CLI](https://diesel.rs/guides/getting-started/)

## Setup

1. **Clone the repository:**
   ```sh
   git clone https://github.com/your-org/energy-tracker.git
   cd energy-tracker
   ```

2. **Install dependencies:**
   - Install Rust and SP1 as described above.
   - Install Diesel CLI:
     ```sh
     cargo install diesel_cli --no-default-features --features postgres
     ```

3. **Configure environment variables:**
   - Copy the example environment file and edit as needed:
     ```sh
     cp .env.example .env
     ```
   - Set your database URL and any required RPC URLs in `.env`.

4. **Setup the database:**
   - Create the database:
     ```sh
     createdb m3tering-db
     ```
   - Run Diesel migrations:
     ```sh
     diesel migration run
     ```

## Running the Program

### Build and Run the Node

The main backend service is in the `node` package. To start the server:

```sh
cargo run --release
```

### Proving Thread

The proving process runs in a **dedicated background thread** using Tokio.  
This thread periodically (interval configurable via the `BLOCK_INTERVAL` environment variable) queries the database for unverified payloads, groups them, and runs the prover logic.  
Once a proof is generated, the thread commits the state and updates the relevant payloads as verified in the database.

**Key points:**
- The proving thread does not block the main server endpoints.
- The interval for proving is set via the `.env` file (`BLOCK_INTERVAL`).
- All database and proving operations are handled asynchronously.


The server will start on `http://localhost:8080` and expose several endpoints:
- `/payload` — Submit a single energy payload (POST)
- `/batch-payloads` — Submit multiple payloads (POST)
- `/run_prover` — Generate a proof (GET)
- `/vkey` — Retrieve the verification key (GET)
- `/health` — Health check (GET)

### Example: Submit a Payload

```sh
curl -X POST http://localhost:8080/payload \
  -H "Content-Type: application/json" \
  -d '{"m3ter_id":12345,"message":"payload_data"}'
```

### Example: Generate a Proof

```sh
curl "http://localhost:8080/run_prover?proof_type=groth16"
```

## Troubleshooting

- Ensure your `.env` file is correctly configured and loaded.
- Make sure PostgreSQL is running and accessible.
- If you encounter build errors, update your toolchains and dependencies.

## License

MIT