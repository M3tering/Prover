FROM rust:1.86 AS builder
WORKDIR /app

RUN apt-get update && apt-get install -y libpq-dev libssl-dev pkg-config curl

COPY . .

RUN curl -L https://sp1up.succinct.xyz | bash && \
    export PATH="$HOME/.sp1/bin:$PATH" && \
    sp1up

RUN cargo clean && \
    cargo build --release 

FROM debian:latest
WORKDIR /app

RUN apt-get update && apt-get install -y libssl3 libpq5 libpq-dev ca-certificates && rm -rf /var/lib/apt/lists/*

COPY --from=builder /app/node/target/release/node /app/node
COPY /migrations /app/migrations
COPY .env /app/.env

RUN curl https://sh.rustup.rs -sSf | bash -s -- -y
ENV PATH="/root/.cargo/bin:${PATH}"
RUN cargo install diesel_cli --no-default-features --features postgres

CMD ["sh", "-c", "diesel migration run && /app/node"]