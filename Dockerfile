FROM rust:1.86 AS builder

RUN apt-get update && apt-get install -y \
    clang mold \
    libpq-dev libssl-dev pkg-config libssl3 libpq5 \
    && rm -rf /var/lib/apt/lists/

WORKDIR /app

COPY . .

RUN curl -L https://sp1up.succinct.xyz | bash && \
    export PATH="$HOME/.sp1/bin:$PATH" && \
    sp1up

RUN cargo clean && cargo build --release 

FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y libpq-dev libpq5 ca-certificates && rm -rf /var/lib/apt/lists/*

COPY --from=builder /app/target/release/node /usr/local/bin/prover-node

CMD ["prover-node"]