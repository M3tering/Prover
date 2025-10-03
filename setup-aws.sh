#!/bin/bash
set -e

echo "__/\\\\____________/\\\\_____/\\\\\\\\\\____________________________________________________________/\\\____________________________________________                    
 _\/\\\\\\________/\\\\\\___/\\\///////\\\__________________________________________________________\/\\\____________________________________________                   
  _\/\\\//\\\____/\\\//\\\__\///______/\\\______/\\\_________________________________________________\/\\\_________________________/\\\_______________                  
   _\/\\\\///\\\/\\\/_\/\\\_________/\\\//____/\\\\\\\\\\\_____/\\\\\\\\___/\\/\\\\\\\______/\\\\\\\\_\/\\\__________/\\\\\\\\\____\///___/\\/\\\\\\___                 
    _\/\\\__\///\\\/___\/\\\________\////\\\__\////\\\////____/\\\/////\\\_\/\\\/////\\\___/\\\//////__\/\\\\\\\\\\__\////////\\\____/\\\_\/\\\////\\\__                
     _\/\\\____\///_____\/\\\___________\//\\\____\/\\\_______/\\\\\\\\\\\__\/\\\___\///___/\\\_________\/\\\/////\\\___/\\\\\\\\\\__\/\\\_\/\\\__\//\\\_               
      _\/\\\_____________\/\\\__/\\\______/\\\_____\/\\\_/\\__\//\\///////___\/\\\_________\//\\\________\/\\\___\/\\\__/\\\/////\\\__\/\\\_\/\\\___\/\\\_              
       _\/\\\_____________\/\\\_\///\\\\\\\\\/______\//\\\\\____\//\\\\\\\\\\_\/\\\__________\///\\\\\\\\_\/\\\___\/\\\_\//\\\\\\\\/\\_\/\\\_\/\\\___\/\\\_             
        _\///______________\///____\/////////_________\/////______\//////////__\///_____________\////////__\///____\///___\////////\//__\///__\///____\///__            
__/\\\\\\\\\\\\\_____________________________________________________________________________________/\\\\\_____/\\\_______________________/\\\_________________        
 _\/\\\/////////\\\__________________________________________________________________________________\/\\\\\\___\/\\\______________________\/\\\_________________       
  _\/\\\_______\/\\\__________________________________________________________________________________\/\\\/\\\__\/\\\______________________\/\\\_________________      
   _\/\\\\\\\\\\\\\/___/\\/\\\\\\\______/\\\\\_____/\\\____/\\\_____/\\\\\\\\___/\\/\\\\\\\____________\/\\\//\\\_\/\\\_____/\\\\\___________\/\\\______/\\\\\\\\__     
    _\/\\\/////////____\/\\\/////\\\___/\\\///\\\__\//\\\__/\\\____/\\\/////\\\_\/\\\/////\\\___________\/\\\\//\\\\/\\\___/\\\///\\\____/\\\\\\\\\____/\\\/////\\\_    
     _\/\\\_____________\/\\\___\///___/\\\__\//\\\__\//\\\/\\\____/\\\\\\\\\\\__\/\\\___\///____________\/\\\_\//\\\/\\\__/\\\__\//\\\__/\\\////\\\___/\\\\\\\\\\\__   
      _\/\\\_____________\/\\\_________\//\\\__/\\\____\//\\\\\____\//\\///////___\/\\\___________________\/\\\__\//\\\\\\_\//\\\__/\\\__\/\\\__\/\\\__\//\\///////___  
       _\/\\\_____________\/\\\__________\///\\\\\/______\//\\\______\//\\\\\\\\\\_\/\\\___________________\/\\\___\//\\\\\__\///\\\\\/___\//\\\\\\\/\\__\//\\\\\\\\\\_ 
        _\///______________\///_____________\/////_________\///________\//////////__\///____________________\///_____\/////_____\/////______\///////\//____\//////////__"

# --- Ask for domain name (optional) ---
read -p "Enter your domain name (leave blank to use localhost): " DOMAIN_NAME

# --- Prompt DB password ---
read -s -p "Enter password for PostgreSQL user 'm3tering': " DB_PASS
echo
if [ -z "$DB_PASS" ]; then
    DB_PASS=$(openssl rand -hex 16)
    echo "Generated DB password: $DB_PASS"
fi
export DB_PASS

# --- Prompt Private Key ---
read -s -p "Enter private key: " PRIVATE_KEY
if [ -z "$PRIVATE_KEY" ]; then
    PRIVATE_KEY=$(openssl rand -hex 32)
    echo "Generated private key: $PRIVATE_KEY"
fi
export PRIVATE_KEY

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
cd "$HOME"
if [ ! -d "$HOME/m3terchain-prover" ]; then
    git clone --depth=1 --branch main https://github.com/M3tering/Prover.git m3terchain-prover
else
    cd m3terchain-prover && git pull origin main && cd ..
fi

cd m3terchain-prover

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
touch .env
cat > .env <<EOL
## RPC_URL for connecting to an Ethereum node
RPC_URL=

## SP1 prover network rpc URL
NETWORK_RPC_URL=

## Proof requester private key
PRIVATE_KEY=${PRIVATE_KEY}

## Interval (in seconds) at which to check for new transactions and create new rollup proofs
BLOCK_INTERVAL=10

## Database connection string
DATABASE_URL=${DATABASE_URL}
EOL

# --- Configure systemd service ---
echo "Configuring systemd service..."
SERVICE_FILE=/etc/systemd/system/m3terchain-prover.service
sudo bash -c "cat > $SERVICE_FILE" <<EOL
[Unit]
Description=Energy Tracker Node Service
After=network.target postgresql.service

[Service]
User=$USER
WorkingDirectory=$HOME/m3terchain-prover
Environment=PATH=$HOME/.cargo/bin:$HOME/.sp1/bin:/usr/bin:/bin
EnvironmentFile=$HOME/m3terchain-prover/.env
ExecStart=$HOME/.cargo/bin/cargo run --release
Restart=always
RestartSec=5
LimitNOFILE=65535

[Install]
WantedBy=multi-user.target
EOL

sudo systemctl daemon-reload
sudo systemctl enable m3terchain-prover
sudo systemctl start m3terchain-prover

# --- Nginx setup ---
if [ -n "$DOMAIN_NAME" ]; then
    echo "Configuring Nginx for domain: $DOMAIN_NAME"
    sudo tee /etc/nginx/sites-available/m3terchain-prover > /dev/null <<EOL
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
    sudo ln -sf /etc/nginx/sites-available/m3terchain-prover /etc/nginx/sites-enabled/
    sudo nginx -t && sudo systemctl restart nginx
else
   sudo tee /etc/nginx/sites-available/m3terchain-prover > /dev/null <<EOL
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
    sudo ln -sf /etc/nginx/sites-available/m3terchain-prover /etc/nginx/sites-enabled/
    sudo nginx -t && sudo systemctl restart nginx
fi

echo "    ____         ______          __                    __                  __                                                  
   /  _/___     / ____/___  ____/ /  _      _____     / /________  _______/ /_                                                 
   / // __ \   / / __/ __ \/ __  /  | | /| / / _ \   / __/ ___/ / / / ___/ __/                                                 
 _/ // / / /  / /_/ / /_/ / /_/ /   | |/ |/ /  __/  / /_/ /  / /_/ (__  ) /_                                                   
/___/_/ /_/__ \____/\____/\__,_/    |__/|__/\___/   \__/_/   \__,_/____/\__/  __         _                    __      __       
  ____ _/ / /  ____  / /_/ /_  ___  __________   ____ ___  __  _______/ /_   / /_  _____(_)___  ____ _   ____/ /___ _/ /_____ _
 / __ '/ / /  / __ \/ __/ __ \/ _ \/ ___/ ___/  / __ '__ \/ / / / ___/ __/  / __ \/ ___/ / __ \/ __ '/  / __  / __ '/ __/ __ '/
/ /_/ / / /  / /_/ / /_/ / / /  __/ /  (__  )  / / / / / / /_/ (__  ) /_   / /_/ / /  / / / / / /_/ /  / /_/ / /_/ / /_/ /_/ / 
\__,_/_/_/   \____/\__/_/ /_/\___/_/  /____/  /_/ /_/ /_/\__,_/____/\__/  /_.___/_/  /_/_/' /_/\__, /   \__,_/\__,_/\__/\__,_/  
                                                                                             /____/                            "
echo "Service is running: systemctl status m3terchain-prover"
