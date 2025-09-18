#!/bin/bash
# filepath: /home/godwin/energy-tracker/setup.sh

set -e

echo "=== Energy Tracker Setup Script ==="

REPO_URL="https://github.com/M3tering/Prover.git"
REPO_DIR="Prover"

# Clone the repository if not already present
if [ ! -d "$REPO_DIR" ]; then
    echo "Cloning repository from $REPO_URL..."
    git clone "$REPO_URL"
    cd "$REPO_DIR"
else
    echo "Repository already cloned."
    cd "$REPO_DIR"
fi

# Check for Rust
if ! command -v cargo &> /dev/null; then
    echo "Rust not found. Please install Rust from https://rustup.rs/"
    exit 1
fi

# Check for Diesel CLI
if ! command -v diesel &> /dev/null; then
    echo "Installing Diesel CLI..."
    cargo install diesel_cli --no-default-features --features postgres
fi

# Check for PostgreSQL
if ! command -v psql &> /dev/null; then
    echo "PostgreSQL not found. Please install PostgreSQL."
    exit 1
fi

# Create .env if missing
if [ ! -f .env ]; then
    if [ -f .env.example ]; then
        cp .env.example .env
        echo "Copied .env.example to .env"
        echo "update .env to avoid errors during runtime"
    else
        echo "No .env.example found. Please create a .env file."
        exit 1
    fi
fi

# Create database
DB_NAME="m3tering-db"
if psql -lqt | cut -d \| -f 1 | grep -qw "$DB_NAME"; then
    echo "Database $DB_NAME already exists."
else
    echo "Creating database $DB_NAME..."
    createdb "$DB_NAME"
fi

# Run Diesel migrations
echo "Running Diesel migrations..."
diesel migration run

echo "=== Setup Complete! ==="
echo "You can now run the node with: cargo run --release"