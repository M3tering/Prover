FROM rust:1.86 
WORKDIR /app

RUN apt-get update && apt-get install -y libpq-dev libssl-dev pkg-config curl libssl3 libpq5 ca-certificates && rm -rf /var/lib/apt/lists/

COPY . .

RUN curl -L https://sp1up.succinct.xyz | bash && \
    export PATH="$HOME/.sp1/bin:$PATH" && \
    sp1up

RUN cargo clean && \
    cargo build --release 

CMD ["/app/target/release/node"]
