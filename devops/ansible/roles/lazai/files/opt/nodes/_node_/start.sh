#!/usr/bin/env bash
set -euo pipefail

# -------- CONFIG --------
HOST_ALIAS="${HOST_ALIAS:-$(hostname | cut -f1 -d'.')}"               # e.g. rpc0
NODE_TYPE="${NODE_TYPE:-$(echo "${HOST_ALIAS}" | sed 's/[0-9]*$//')}" # e.g. rpc
NODE_DIR="/opt/nodes/${HOST_ALIAS}"

# Compose files
MALA_YAML="compose-${NODE_TYPE}-mala.yaml"
RETH_YAML="compose-${NODE_TYPE}-reth.yaml"

# -------- LOG/REQS --------
log() { printf '[%s] %s\n' "$(date -u +'%Y-%m-%dT%H:%M:%SZ')" "$*"; }
die() { log "ERROR: $*"; exit 1; }
need() { command -v "$1" >/dev/null 2>&1 || die "Missing dependency: $1"; }
for cmd in curl jq ss grep docker; do need "$cmd"; done

cd "$NODE_DIR"

# -------- HELPERS --------
port_listening() {
  local port="$1"
  if ss -H -ltn "sport = :$port" 2>/dev/null | grep -q .; then return 0; fi
  if ss -H -ltn 2>/dev/null | grep -q ":$port "; then return 0; fi
  return 1
}

wait_reachable_port() {
  local port="$1" desc="$2" need="$3" interval="$4" timeout="${5:-180}"
  local ok=0 elapsed=0
  log "Waiting for $desc (port $port) LISTEN ($need consecutive successes, max ${timeout}s)…"
  while (( ok < need )); do
    if port_listening "$port"; then
      ((ok++)); log "  Port $port listening (${ok}/${need})"
    else
      ok=0; log "  Port $port not listening; retry in ${interval}s…"
    fi
    sleep "$interval"; ((elapsed+=interval))
    if (( elapsed >= timeout )); then log "Timeout waiting for port $port to listen."; return 1; fi
  done
  log "$desc port-listen check succeeded."; return 0
}

wait_reachable_http() {
  local url="$1" desc="$2" need="$3" interval="$4" timeout="${5:-180}"
  local ok=0 elapsed=0
  log "Waiting for $desc ($url) HEALTHY ($need consecutive successes, max ${timeout}s)…"
  while (( ok < need )); do
    if curl -sS --connect-timeout 1 --max-time 2 "$url" >/dev/null 2>&1; then
      ((ok++)); log "  Reachable (${ok}/${need})"
    else
      ok=0; log "  Not yet reachable; retry in ${interval}s…"
    fi
    sleep "$interval"; ((elapsed+=interval))
    if (( elapsed >= timeout )); then log "Timeout waiting for $desc to be healthy."; return 1; fi
  done
  log "$desc HTTP health check succeeded."; return 0
}

wait_jsonrpc_ready() {
  local url="$1" desc="$2" need="$3" interval="$4" timeout="${5:-240}"
  local ok=0 elapsed=0
  log "Waiting for $desc JSON-RPC ready ($need consecutive successes, max ${timeout}s)…"
  while (( ok < need )); do
    if curl -sS -H "Content-Type: application/json" "$url" \
      -d '{"jsonrpc":"2.0","id":1,"method":"eth_blockNumber","params":[]}' \
      | jq -er 'select(.result|type=="string")' >/dev/null 2>&1; then
      ((ok++)); log "  JSON-RPC OK (${ok}/${need})"
    else
      ok=0; log "  JSON-RPC not ready; retry in ${interval}s…"
    fi
    sleep "$interval"; ((elapsed+=interval))
    if (( elapsed >= timeout )); then log "Timeout waiting for JSON-RPC readiness."; return 1; fi
  done
  log "$desc JSON-RPC readiness check succeeded."; return 0
}

# -------- START ORDER (mala gated on reth) --------

# 1) Start reth first
if [[ -f "$RETH_YAML" ]]; then
  log "Starting reth with $RETH_YAML …"
  docker compose -f "$RETH_YAML" up -d || die "reth start failed"
  wait_reachable_port 8545 "reth RPC" 5 3 180 \
    && wait_reachable_http  "http://localhost:8545" "reth RPC endpoint" 5 3 180 \
    && wait_jsonrpc_ready   "http://localhost:8545" "reth" 5 3 240 \
    || die "Reth failed to become healthy — aborting."
else
  die "$RETH_YAML not found — cannot start mala without reth."
fi

# 2) Start mala only if reth is healthy
if [[ -f "$MALA_YAML" ]]; then
  log "Starting mala with $MALA_YAML …"
  docker compose -f "$MALA_YAML" up -d || die "mala start failed"
  wait_reachable_port 29000 "mala metrics" 3 5 180 \
    && wait_reachable_http  "http://localhost:29000/metrics" "mala metrics endpoint" 3 5 180 \
    || die "Mala failed to become healthy."
else
  log "$MALA_YAML not found, skipping mala start."
fi

log "Services started and healthy."
