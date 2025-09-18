#!/bin/bash
set -e

echo "=== Energy Tracker Node Setup Script ==="

# --- Ask for domain name (optional) ---
read -p "Enter your domain name (leave blank to use localhost): " DOMAIN_NAME

# --- Prompt DB password ---
read -s -p "Enter password for PostgreSQL user 'm3tering': " DB_PASS
echo
export DB_PASS

# --- Update system ---
sudo apt update && sudo apt upgrade -y

# --- Install dependencies ---
sudo apt install -y build-essential pkg-config libssl-dev \
    libpq-dev postgresql postgresql-contrib \
    curl git nginx ufw

# --- Install Rust ---
if ! command -v rustc &>/dev/null; then
    echo "Installing Rust..."
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
    source $HOME/.cargo/env
fi

# --- Install Diesel CLI ---
if ! command -v diesel &>/dev/null; then
    cargo install diesel_cli --no-default-features --features postgres
fi

# --- Setup PostgreSQL ---
echo "Setting up PostgreSQL..."
sudo -u postgres psql -tc "SELECT 1 FROM pg_roles WHERE rolname='m3tering'" | grep -q 1 || \
    sudo -u postgres psql -c "CREATE USER m3tering WITH PASSWORD '${DB_PASS}';"

sudo -u postgres psql -tc "SELECT 1 FROM pg_database WHERE datname='m3tering-db'" | grep -q 1 || \
    sudo -u postgres psql -c "CREATE DATABASE \"m3tering-db\" OWNER m3tering;"

# --- Clone project ---
echo "Cloning latest project release..."
if [ ! -d "$HOME/energy-tracker-node" ]; then
    git clone --depth=1 --branch main https://github.com/M3tering/Prover.git energy-tracker-node
else
    cd energy-tracker-node && git pull origin main && cd ..
fi

cd energy-tracker-node

# --- Run DB migrations ---
echo "Running Diesel migrations..."
export DATABASE_URL=postgres://m3tering:${DB_PASS}@localhost/m3tering-db
diesel migration run

# --- Install SP1 via sp1up ---
echo "Installing SP1..."
if ! command -v sp1up &>/dev/null; then
    curl -L https://sp1up.succinct.xyz | bash
    export PATH="$HOME/.sp1/bin:$PATH"
    echo 'export PATH="$HOME/.sp1/bin:$PATH"' >> ~/.bashrc
fi

sp1up

# Verify SP1
if command -v cargo-prove &>/dev/null; then
    echo "SP1 installed successfully: $(cargo prove --version)"
else
    echo "⚠️ SP1 installation failed or cargo-prove not found."
fi

# --- Setup environment file ---
cat > .env <<EOL
## RPC_URL for connecting to an Ethereum node
RPC_URL=

## SP1 prover network rpc URL
NETWORK_RPC_URL=

## Proof requester private key
PRIVATE_KEY=

## Interval (in seconds) at which to check for new transactions and create new rollup proofs
BLOCK_INTERVAL=10

## Database connection string
DATABASE_URL=postgres://m3tering:${DB_PASS}@localhost/m3tering-db
EOL

# --- Configure systemd service ---
echo "Configuring systemd service..."
SERVICE_FILE=/etc/systemd/system/energy-tracker-node.service
sudo bash -c "cat > $SERVICE_FILE" <<EOL
[Unit]
Description=Energy Tracker Node Service
After=network.target postgresql.service

[Service]
User=$USER
WorkingDirectory=$HOME/energy-tracker-node
Environment=PATH=$HOME/.cargo/bin:$HOME/.sp1/bin:/usr/bin:/bin
EnvironmentFile=$HOME/energy-tracker-node/.env
ExecStart=$HOME/.cargo/bin/cargo run --release
Restart=always
RestartSec=5
LimitNOFILE=65535

[Install]
WantedBy=multi-user.target
EOL

sudo systemctl daemon-reload
sudo systemctl enable energy-tracker-node
sudo systemctl start energy-tracker-node

# --- Nginx setup ---
if [ -n "$DOMAIN_NAME" ]; then
    echo "Configuring Nginx for domain: $DOMAIN_NAME"
    sudo tee /etc/nginx/sites-available/energy-tracker-node > /dev/null <<EOL
server {
    listen 80;
    server_name ${DOMAIN_NAME};

    location / {
        proxy_pass http://127.0.0.1:8080;
        proxy_http_version 1.1;
        proxy_set_header Upgrade \$http_upgrade;
        proxy_set_header Connection 'upgrade';
        proxy_set_header Host \$host;
        proxy_cache_bypass \$http_upgrade;
    }
}
EOL
    sudo ln -sf /etc/nginx/sites-available/energy-tracker-node /etc/nginx/sites-enabled/
    sudo nginx -t && sudo systemctl restart nginx
else
   sudo tee /etc/nginx/sites-available/energy-tracker-node > /dev/null <<EOL
server {
    listen 80;
    server_name _;

    location / {
        proxy_pass http://127.0.0.1:8080;
        proxy_http_version 1.1;
        proxy_set_header Upgrade \$http_upgrade;
        proxy_set_header Connection 'upgrade';
        proxy_set_header Host \$host;
        proxy_cache_bypass \$http_upgrade;
    }
}
EOL
    sudo ln -sf /etc/nginx/sites-available/energy-tracker-node /etc/nginx/sites-enabled/
    sudo nginx -t && sudo systemctl restart nginx
fi

echo "=== Setup complete! ==="
echo "Service is running: systemctl status energy-tracker-node"
