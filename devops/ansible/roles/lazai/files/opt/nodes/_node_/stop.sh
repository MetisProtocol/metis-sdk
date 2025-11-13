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
for cmd in curl ss grep docker; do need "$cmd"; done

cd "$NODE_DIR"

# -------- HELPERS --------
port_listening() {
  local port="$1"
  if ss -H -ltn "sport = :$port" 2>/dev/null | grep -q .; then return 0; fi
  if ss -H -ltn 2>/dev/null | grep -q ":$port "; then return 0; fi
  return 1
}

wait_unreachable_http() {
  local url="$1" desc="$2" need="$3" interval="$4" timeout="${5:-180}"
  local fails=0 elapsed=0
  log "Waiting for $desc ($url) UNREACHABLE ($need consecutive failures, max ${timeout}s)…"
  while (( fails < need )); do
    if curl -sS --connect-timeout 1 --max-time 2 "$url" >/dev/null 2>&1; then
      fails=0; log "  Still reachable; retry in ${interval}s…"
    else
      ((fails++)); log "  Not reachable (${fails}/${need})"
    fi
    sleep "$interval"; ((elapsed+=interval))
    if (( elapsed >= timeout )); then log "Timeout waiting for $desc to be unreachable."; return 1; fi
  done
  log "$desc unreachable check succeeded."; return 0
}

wait_unreachable_port() {
  local port="$1" desc="$2" need="$3" interval="$4" timeout="${5:-180}"
  local fails=0 elapsed=0
  log "Waiting for $desc (port $port) CLOSED ($need consecutive checks, max ${timeout}s)…"
  while (( fails < need )); do
    if port_listening "$port"; then
      fails=0; log "  Port $port still listening; retry in ${interval}s…"
    else
      ((fails++)); log "  Port $port closed (${fails}/${need})"
    fi
    sleep "$interval"; ((elapsed+=interval))
    if (( elapsed >= timeout )); then log "Timeout waiting for port $port to close."; return 1; fi
  done
  log "$desc port-close check succeeded."; return 0
}

# -------- STOP ORDER --------
# 1) Stop mala (metrics 29000)
if [[ -f "$MALA_YAML" ]]; then
  log "Stopping mala with $MALA_YAML …"
  docker compose -f "$MALA_YAML" down --timeout 60 --remove-orphans || log "mala down returned non-zero"
  wait_unreachable_http "http://localhost:29000/metrics" "mala metrics" 3 5 180 || die "mala metrics still reachable"
  wait_unreachable_port 29000 "mala metrics" 3 5 180 || die "port 29000 still listening"
else
  log "$MALA_YAML not found, skipping mala stop."
fi

# 2) Stop reth (RPC 8545)
if [[ -f "$RETH_YAML" ]]; then
  log "Stopping reth with $RETH_YAML …"
  docker compose -f "$RETH_YAML" down --timeout 60 --remove-orphans || log "reth down returned non-zero"
  wait_unreachable_http "http://localhost:8545" "reth RPC" 5 3 240 || die "reth RPC still reachable"
  wait_unreachable_port 8545 "reth RPC" 5 3 240 || die "port 8545 still listening"
else
  log "$RETH_YAML not found, skipping reth stop."
fi

log "All services stopped and unreachable."
