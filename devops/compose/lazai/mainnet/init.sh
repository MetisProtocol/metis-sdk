#!/usr/bin/env bash
set -e

######################################
# Helpers
######################################

log() {
  echo "[$(date '+%Y-%m-%d %H:%M:%S')] $1"
}

######################################
# 1) Create assets directory
######################################

log "Creating assets directory (if not exists)"
mkdir -p assets


######################################
# 2) Generate JWT secret if not exists
######################################

if [[ ! -f "assets/jwtsecret.txt" ]]; then
  log "Generating JWT secret"
  openssl rand -hex 32 > assets/jwtsecret.txt
  log "JWT secret created at assets/jwtsecret.txt"
else
  log "JWT secret already exists, skipping generation"
fi

######################################
# 3) Download genesis.json
######################################

log "Downloading genesis.json"
wget -q \
  https://raw.githubusercontent.com/0xLazAI/lazai-genesis/refs/heads/main/mainnet/genesis.json \
  -O assets/genesis.json

log "genesis.json downloaded to assets/genesis.json"
log "Setup completed successfully"
