# M3terChain Validity Rollup Prover

This repository implements an [SP1](https://github.com/succinctlabs/sp1) based prover client for generating SNARK proofs of offchain energy metering, consistent with the m3tering protocol. 

## Requirements

- [Rust](https://rustup.rs/)
- [SP1](https://docs.succinct.xyz/docs/sp1/getting-started/install)
- [PostgreSQL](https://www.postgresql.org/download/)
- [Diesel CLI](https://diesel.rs/guides/getting-started/)
- [Docker](https://www.docker.com/get-started/)

## Quick Setup

The setup uses prebuilt docker images of the `prover` and the `prover/streamr-client`.

- Clone the repository

```sh
git clone https://github.com/M3tering/Prover.git
```

```sh
cd Prover
```

- Set up environment variables

```sh
cp .env.example .env
```

> Make sure you set all the variables in the `.env` file

- Start the docker containers

```sh
docker compose up --build -d
```

- See Logs

```sh
docker compose logs
```

## Manual Setup

1. **Clone the repository:**
   ```sh
   git clone https://github.com/m3tering/prover.git
   cd prover
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

### Expose Local Server with ngrok

If you want to expose your local server to the internet (for testing webhooks, integrations, etc.), you can use ngrok:
Install ngrok (if not already installed)

Expose your local server (port 8080):

```sh
ngrok http 8080
```

Copy the forwarding URL from the ngrok output and use it to access your local server from anywhere.

Note: You may need to sign up for an ngrok account and authenticate your agent for extended usage.

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

[MIT](LICENSE)
