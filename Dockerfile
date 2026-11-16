# syntax=docker/dockerfile:1.7
ARG RUST_IMAGE=rust:1.99.1-bookworm@sha256:48b7f435b8fc6378ce43b44f15e3a47fd8919b2825f835c39bd4bd3da50af5b8
ARG NODE_IMAGE=node:24.16.0-bookworm-slim@sha256:2c87ef9bd3c6a3bd4b472b4bec2ce9d16354b0c574f736c476489d09f560a203
FROM ${RUST_IMAGE} AS builder
RUN apt-get update && apt-get install -y --no-install-recommends protobuf-compiler libprotobuf-dev pkg-config libssl-dev && test -f /usr/include/google/protobuf/empty.proto && rm -rf /var/lib/apt/lists/*
WORKDIR /src
COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY lib lib
COPY host host
COPY program program
ENV CARGO_BUILD_JOBS=1 CARGO_PROFILE_RELEASE_LTO=false CARGO_PROFILE_RELEASE_CODEGEN_UNITS=16
RUN --mount=type=cache,target=/usr/local/cargo/registry --mount=type=cache,target=/usr/local/cargo/git cargo build --locked --release -p m3tering-prover --features proving
FROM ${NODE_IMAGE} AS dependencies
RUN apt-get update && apt-get install -y --no-install-recommends python3 make g++ cmake ca-certificates && rm -rf /var/lib/apt/lists/*
WORKDIR /app
COPY streamr-client/package*.json ./
RUN npm ci --omit=dev && npm cache clean --force
FROM ${NODE_IMAGE} AS api
RUN apt-get update && apt-get install -y --no-install-recommends libssl3 ca-certificates && rm -rf /var/lib/apt/lists/*
COPY --from=dependencies /app/node_modules /app/node_modules
COPY streamr-client /app
WORKDIR /app
USER node
EXPOSE 3000
CMD ["node", "index.js"]

FROM api AS worker
COPY --from=builder /src/target/release/m3tering-prover /usr/local/bin/m3tering-prover
COPY release /opt/m3tering/release
COPY fixtures /opt/m3tering/fixtures
WORKDIR /opt/m3tering
RUN sha256sum -c release/program.sha256
ENV SP1_ELF=/opt/m3tering/release/program.elf BATCH_DIR=/data/batches SP1_CACHE_DIR=/data/cache
WORKDIR /app
CMD ["node", "worker-main.js"]
