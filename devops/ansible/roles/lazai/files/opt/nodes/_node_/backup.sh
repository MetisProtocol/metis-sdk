#!/usr/bin/env bash
set -euo pipefail

######################### CONFIG #########################

HOST_ALIAS="${HOST_ALIAS:-$(hostname | cut -f1 -d'.')}"               # e.g. rpc0
NODE_TYPE="${NODE_TYPE:-$(echo "${HOST_ALIAS}" | sed 's/[0-9]*$//')}" # e.g. rpc

# JSON-RPC endpoints
REMOTE_RPC="${REMOTE_RPC:-https://mainnet.lazai.network/}"
LOCAL_RPC="${LOCAL_RPC:-http://localhost:8545/}"
MAX_LAG="${MAX_LAG:-60}"

# Paths
NODE_DIR="/opt/nodes/${HOST_ALIAS}"
SOURCE_DIRS=(${SOURCE_DIRS:-${NODE_DIR}/mala/data})
SOURCE_DIRS+=("${NODE_DIR}/reth/rethdata")
BACKUP_DIR="${BACKUP_DIR:-${NODE_DIR}/_backup}"

LOCKFILE="${LOCKFILE:-/var/lock/lazai_tar_backup.lock}"

# Compression
GZIP_CMD="$(command -v pigz || echo gzip)"
GZIP_EXT="gz"
WRITE_SHA256="${WRITE_SHA256:-1}"

# S3
AWS_REGION="${AWS_REGION?}"
AWS_S3_BUCKET="${AWS_S3_BUCKET?}"
AWS_S3_PREFIX="${AWS_S3_PREFIX:-mainnet/backups/${HOST_ALIAS}}"
S3_SSE="${S3_SSE:-AES256}"
S3_STORAGE_CLASS="${S3_STORAGE_CLASS:-STANDARD}"
S3_MAX_RETRIES="${S3_MAX_RETRIES:-3}"

# S3 retention
S3_RETAIN_DAYS="${S3_RETAIN_DAYS:-15}"
S3_KEEP_NEWEST_COUNT="${S3_KEEP_NEWEST_COUNT:-20}"

# Local retention (keep newest N archives)
KEEP_LOCAL_COUNT="${KEEP_LOCAL_COUNT:-3}"

######################################

log() { printf '[%s] %s\n' "$(date -u +'%Y-%m-%dT%H:%M:%SZ')" "$*"; }
die() { log "ERROR: $*"; exit 1; }
need() { command -v "$1" >/dev/null 2>&1 || die "Missing dependency: $1"; }

for cmd in curl jq tar "${GZIP_CMD}" sha256sum flock date find ls aws docker ss grep; do need "$cmd"; done

# Prevent overlapping runs
exec 9>"$LOCKFILE"
if ! flock -n 9; then
  log "Another run is in progress; exiting."
  exit 0
fi

mkdir -p "$BACKUP_DIR"

######################################
# Helpers
######################################

get_block_dec() {
  local url="$1"
  local hex
  hex="$(curl -fsS -H "Content-Type: application/json" "$url" \
           -d '{"jsonrpc":"2.0","id":1,"method":"eth_blockNumber","params":[]}' \
        | jq -r '.result')" || return 1
  [[ "$hex" =~ ^0x[0-9a-fA-F]+$ ]] || return 2
  printf '%d\n' "$hex"
}

# --- Port presence (robust) ---
port_listening() {
  local port="$1"
  # Preferred: ss filter + no header
  if ss -H -ltn "sport = :$port" 2>/dev/null | grep -q .; then
    return 0
  fi
  # Fallback: scan all listens and grep the port token
  if ss -H -ltn 2>/dev/null | grep -q ":$port "; then
    return 0
  fi
  return 1
}

# -------- Wait helpers (all return non-zero on failure/timeout) --------

wait_unreachable_http() {
  local url="$1" desc="$2" need="$3" interval="$4" timeout="${5:-180}"
  local fails=0 elapsed=0
  log "Waiting for $desc ($url) to become UNREACHABLE ($need consecutive failures, max ${timeout}s)..."
  while (( fails < need )); do
    if curl -sS --connect-timeout 1 --max-time 2 "$url" >/dev/null 2>&1; then
      fails=0; log "  Still reachable; retry in ${interval}s…"
    else
      ((fails++)); log "  Not reachable (${fails}/${need})"
    fi
    sleep "$interval"; ((elapsed+=interval))
    if (( elapsed >= timeout )); then
      log "Timeout waiting for $desc to become unreachable."
      return 1
    fi
  done
  log "$desc unreachable check succeeded."
  return 0
}

wait_unreachable_port() {
  local port="$1" desc="$2" need="$3" interval="$4" timeout="${5:-180}"
  local fails=0 elapsed=0
  log "Waiting for $desc (port $port) to CLOSE ($need consecutive checks, max ${timeout}s)…"
  while (( fails < need )); do
    if port_listening "$port"; then
      fails=0; log "  Port $port still listening; retry in ${interval}s…"
    else
      ((fails++)); log "  Port $port closed (${fails}/${need})"
    fi
    sleep "$interval"; ((elapsed+=interval))
    if (( elapsed >= timeout )); then
      log "Timeout waiting for port $port to close."
      return 1
    fi
  done
  log "$desc port-close check succeeded."
  return 0
}

wait_reachable_http() {
  local url="$1" desc="$2" need="$3" interval="$4" timeout="${5:-180}"
  local ok=0 elapsed=0
  log "Waiting for $desc ($url) to become HEALTHY ($need consecutive successes, max ${timeout}s)…"
  while (( ok < need )); do
    if curl -sS --connect-timeout 1 --max-time 2 "$url" >/dev/null 2>&1; then
      ((ok++)); log "  Reachable (${ok}/${need})"
    else
      ok=0; log "  Not yet reachable; retry in ${interval}s…"
    fi
    sleep "$interval"; ((elapsed+=interval))
    if (( elapsed >= timeout )); then
      log "Timeout waiting for $desc to become healthy."
      return 1
    fi
  done
  log "$desc HTTP health check succeeded."
  return 0
}

wait_reachable_port() {
  local port="$1" desc="$2" need="$3" interval="$4" timeout="${5:-180}"
  local ok=0 elapsed=0
  log "Waiting for $desc (port $port) to LISTEN ($need consecutive successes, max ${timeout}s)…"
  while (( ok < need )); do
    if port_listening "$port"; then
      ((ok++)); log "  Port $port listening (${ok}/${need})"
    else
      ok=0; log "  Port $port not listening; retry in ${interval}s…"
    fi
    sleep "$interval"; ((elapsed+=interval))
    if (( elapsed >= timeout )); then
      log "Timeout waiting for port $port to listen."
      return 1
    fi
  done
  log "$desc port-listen check succeeded."
  return 0
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
    if (( elapsed >= timeout )); then
      log "Timeout waiting for JSON-RPC readiness."
      return 1
    fi
  done
  log "$desc JSON-RPC readiness check succeeded."
  return 0
}

######################################
# 1) Check block lag BEFORE stopping services
######################################

log "Querying remote block number: $REMOTE_RPC"
remote_dec="$(get_block_dec "$REMOTE_RPC")" || die "Failed to read remote block number"
log "Remote block (dec): $remote_dec"

log "Querying local block number: $LOCAL_RPC"
local_dec="$(get_block_dec "$LOCAL_RPC")" || die "Failed to read local block number"
log "Local block (dec):  $local_dec"

if (( remote_dec >= local_dec )); then
  lag=$(( remote_dec - local_dec ))
else
  lag=$(( local_dec - remote_dec ))
fi
log "Block lag: $lag (max allowed: $MAX_LAG)"
if (( lag > MAX_LAG )); then
  log "Lag exceeds threshold — skipping backup."
  exit 0
fi

######################################
# 2) Stop containers before backup
######################################

cd "$NODE_DIR"

MALA_YAML="compose-${NODE_TYPE}-mala.yaml"
RETH_YAML="compose-${NODE_TYPE}-reth.yaml"

# Stop mala first
if [[ -f "$MALA_YAML" ]]; then
  log "Stopping mala stack with $MALA_YAML ..."
  docker compose -f "$MALA_YAML" down --timeout 60 --remove-orphans || log "mala down returned non-zero"
  wait_unreachable_http "http://localhost:29000/metrics" "mala metrics endpoint" 3 5 180 || die "mala metrics still reachable"
  wait_unreachable_port 29000 "mala metrics" 3 5 180 || die "port 29000 still listening"
else
  log "$MALA_YAML not found, skipping mala stop."
fi

# Then stop reth
if [[ -f "$RETH_YAML" ]]; then
  log "Stopping reth stack with $RETH_YAML ..."
  docker compose -f "$RETH_YAML" down --timeout 60 --remove-orphans || log "reth down returned non-zero"
  wait_unreachable_http "http://localhost:8545" "reth RPC endpoint" 5 3 240 || die "reth RPC still reachable"
  wait_unreachable_port 8545 "reth RPC" 5 3 240 || die "port 8545 still listening"
else
  log "$RETH_YAML not found, skipping reth stop."
fi

######################################
# 3) Create tarball
######################################

ts="$(date -u +'%Y%m%d-%H%M%S')"
archive_basename="mala-backup-${ts}-blk${local_dec}.tar.${GZIP_EXT}"
archive_path="${BACKUP_DIR%/}/${archive_basename}"

tar_args=(-c --xattrs --acls --warning=no-file-changed --exclude="discovery-secret" --exclude="known-peers.json" --exclude=priv_validator_key.json)
for src in "${SOURCE_DIRS[@]}"; do
  [[ -d "$src" ]] || die "Source directory not found: $src"
  tar_args+=( -C / "${src#/}" )
done

log "Creating archive: $archive_path"
tar "${tar_args[@]}" | "${GZIP_CMD}" -c > "$archive_path"
[[ -s "$archive_path" ]] || die "Archive is empty: $archive_path"
log "Archive size: $(stat -c '%s bytes' "$archive_path")"

if [[ "$WRITE_SHA256" == "1" ]]; then
  sha_file="${archive_path}.sha256"
  (cd "$(dirname "$archive_path")" && sha256sum "$(basename "$archive_path")") > "$sha_file"
  log "Wrote checksum: $sha_file"
fi

######################################
# 4) Upload to S3
######################################

S3_URI="s3://${AWS_S3_BUCKET}/${AWS_S3_PREFIX%/}/$(basename "$archive_path")"
AWS_CP=(aws s3 cp --only-show-errors --region "$AWS_REGION" "$archive_path" "$S3_URI" --storage-class "$S3_STORAGE_CLASS")
[[ -n "$S3_SSE" ]] && AWS_CP+=(--sse "$S3_SSE")

log "Uploading archive to ${S3_URI} …"
n=0
until ("${AWS_CP[@]}"); do
  (( n++ >= S3_MAX_RETRIES )) && die "S3 upload failed after ${S3_MAX_RETRIES} attempts."
  log "Upload failed, retrying ($n/${S3_MAX_RETRIES})…"
  sleep 2
done
log "Archive upload complete."

if [[ -f "${archive_path}.sha256" ]]; then
  S3_SHA_URI="${S3_URI}.sha256"
  AWS_CP_SHA=(aws s3 cp --only-show-errors --region "$AWS_REGION" "${archive_path}.sha256" "$S3_SHA_URI" --storage-class "$S3_STORAGE_CLASS")
  [[ -n "$S3_SSE" ]] && AWS_CP_SHA+=(--sse "$S3_SSE")
  "${AWS_CP_SHA[@]}" && log "Uploaded checksum to ${S3_SHA_URI}"
fi

######################################
# 4.5) Local retention: keep newest N (default 3)
######################################

log "Applying local retention: keep newest ${KEEP_LOCAL_COUNT} archive(s) in ${BACKUP_DIR}"
mapfile -d '' -t _archives < <(
  find "${BACKUP_DIR}" -maxdepth 1 -type f -name "mala-backup-*.tar.${GZIP_EXT}" \
    -printf "%T@ %p\0" 2>/dev/null | sort -zr -n | cut -z -d' ' -f2-
)
if ((${#_archives[@]} > KEEP_LOCAL_COUNT)); then
  for f in "${_archives[@]:${KEEP_LOCAL_COUNT}}"; do
    [[ -e "$f" ]] || continue
    log "  deleting: $f"
    rm -f -- "$f" "$f.sha256" 2>/dev/null || true
  done
else
  log "  nothing to prune (found ${#_archives[@]} archives)."
fi

######################################
# 5) S3 retention: keep all <= 15 days; among older, keep newest 20
######################################

if (( S3_RETAIN_DAYS > 0 )); then
  CUTOFF_ISO_UTC="$(date -u -d "${S3_RETAIN_DAYS} days ago" '+%Y-%m-%dT%H:%M:%SZ')"
  log "S3 retention: keep all <= ${S3_RETAIN_DAYS} days; among older, keep newest ${S3_KEEP_NEWEST_COUNT}"
  TMP_JSON="$(mktemp)"
  trap 'rm -f "$TMP_JSON" "${TMP_JSON}.new" 2>/dev/null || true' RETURN
  echo '[]' > "$TMP_JSON"

  CONT=""
  while :; do
    PAGE="$(aws s3api list-objects-v2 --bucket "${AWS_S3_BUCKET}" --prefix "${AWS_S3_PREFIX%/}/" ${CONT:+--continuation-token "$CONT"} --output json)"
    jq -c '.Contents // [] | map({Key, LastModified})' <<<"$PAGE" | jq -s 'add' | jq -s '.[0] + (.[1] // [])' "$TMP_JSON" - > "${TMP_JSON}.new"
    mv "${TMP_JSON}.new" "$TMP_JSON"
    CONT="$(jq -r '.NextContinuationToken // empty' <<<"$PAGE")"
    [[ -n "$CONT" ]] || break
  done

  TO_DELETE_KEYS="$(
    jq -r --arg cutoff "$CUTOFF_ISO_UTC" --argjson keepN "$S3_KEEP_NEWEST_COUNT" '
      sort_by(.LastModified) | reverse |
      { fresh: [ .[] | select(.LastModified >= $cutoff) ],
        older: [ .[] | select(.LastModified <  $cutoff) ] } |
      (.older | .[$keepN:]) | .[]?.Key
    ' "$TMP_JSON"
  )"

  if [[ -n "$TO_DELETE_KEYS" ]]; then
    log "Deleting $(wc -l <<<"$TO_DELETE_KEYS" | tr -d ' ') old object(s) beyond keep policy…"

    while IFS= read -r BATCH; do
      [[ -z "$BATCH" ]] && continue

      echo "BATCH: $BATCH"

      # BATCH is now a full JSON array: [{"Key":"..."}, ...]
      PAYLOAD="$(jq -n --argjson arr "$BATCH" '{Objects: $arr}')"

      echo "PAYLOAD: $PAYLOAD"

      aws s3api delete-objects \
        --bucket "${AWS_S3_BUCKET}" \
        --delete "$PAYLOAD" \
        --output text >/dev/null || true

      # Log deleted keys from this batch
      jq -r '.[].Key' <<<"$BATCH" \
        | sed 's/^/  deleted: /' \
        | while read -r L; do log "$L"; done

    done < <(
      jq -n -c --argjson keys "$(jq -R -s 'split("\n") | map(select(length>0))' <<<"$TO_DELETE_KEYS")" '
        [ range(0; ($keys|length); 1000) as $i |
          ($keys[$i : ($i+1000)]) | map({Key:.})
        ][]        # <- each element is one batch: an array of {Key:...}
      '
    )

  else
    log "Nothing to delete on S3 (policy already satisfied)."
  fi
fi

######################################
# 6) Start stacks again — mala only if reth is healthy
######################################

# Start reth first; abort if reth not healthy
if [[ -f "$RETH_YAML" ]]; then
  log "Starting reth stack with $RETH_YAML ..."
  docker compose -f "$RETH_YAML" up -d || die "reth start failed"

  wait_reachable_port 8545 "reth RPC" 5 3 180 \
    && wait_reachable_http  "http://localhost:8545" "reth RPC endpoint" 5 3 180 \
    && wait_jsonrpc_ready   "http://localhost:8545" "reth" 5 3 240 \
    || die "Reth failed to become healthy — aborting without starting mala."
else
  die "$RETH_YAML not found — cannot start mala without reth."
fi

# Only start mala if reth is healthy
if [[ -f "$MALA_YAML" ]]; then
  log "Starting mala stack with $MALA_YAML ..."
  docker compose -f "$MALA_YAML" up -d || die "mala start failed"

  wait_reachable_port 29000 "mala metrics" 3 5 180 \
    && wait_reachable_http  "http://localhost:29000/metrics" "mala metrics endpoint" 3 5 180 \
    || die "Mala failed to become healthy."
else
  log "$MALA_YAML not found, skipping mala start."
fi

######################################
# Done
######################################
log "Backup complete and services healthy."
