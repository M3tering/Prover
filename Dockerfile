FROM rust:1.86 
WORKDIR /app

RUN apt-get update && apt-get install -y libpq-dev libssl-dev pkg-config curl libssl3 libpq5 ca-certificates && rm -rf /var/lib/apt/lists/

COPY . .

RUN curl -L https://sp1up.succinct.xyz | bash && \
    export PATH="$HOME/.sp1/bin:$PATH" && \
    sp1up

RUN cargo clean && \
    cargo build --release 

RUN cargo install diesel_cli --no-default-features --features postgres

# RUN apt-get update && apt-get install -y libssl3 libpq5 libpq-dev ca-certificates && rm -rf /var/lib/apt/lists/*

# COPY --from=builder /app/target/release/node /app/node
# COPY --from=builder /root/.cargo/bin/diesel /usr/local/bin/diesel

COPY /migrations /app/migrations
COPY .env /app/.env

CMD ["sh", "-c", "diesel migration run --database-url $DATABASE_URL && /app/target/release/node"]
