FROM rust:1.86 AS builder
WORKDIR /app

RUN apt-get update && apt-get install -y libpq-dev libssl-dev pkg-config curl

COPY . .

# Install Diesel CLI
RUN cargo install diesel_cli --no-default-features --features postgres
# RUN diesel migration run --database-url=postgres://m3tering:m3tering@db:5432/m3tering-db --migration-dir=migrations

# Install SP1 (see official docs for latest install command)
RUN curl -L https://sp1up.succinct.xyz | bash && \
    export PATH="$HOME/.sp1/bin:$PATH" && \
    sp1up && \
    cargo build --release 

FROM debian:latest
WORKDIR /app

RUN apt-get update && apt-get install -y libssl3 libpq5 ca-certificates && rm -rf /var/lib/apt/lists/*

COPY --from=builder /app/node/target/release/node /app/node

CMD ["/app/node"]