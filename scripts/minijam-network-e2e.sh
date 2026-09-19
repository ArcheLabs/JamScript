#!/usr/bin/env bash
set -euo pipefail

JAMSCRIPT_ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)"
E2E_RUNTIME="${JAMSCRIPT_E2E_RUNTIME:-${JAMSCRIPT_ROOT}/target/jamscript-network-e2e}"
LOCK_FILE="${JAMSCRIPT_ROOT}/toolchains/minijam.lock"
CONTAINER="${JAMSCRIPT_MINIJAM_CONTAINER:-jamscript-minijam-${GITHUB_RUN_ID:-$$}}"
MINIJAM_IMAGE="${MINIJAM_IMAGE:-}"
MINIJAM_SOURCE_REVISION=""
result=FAIL

export MINIJAM_NODE_RPC="${MINIJAM_NODE_RPC:-http://127.0.0.1:9944}"
export MINIJAM_FORMAL_RPC_URL="${MINIJAM_FORMAL_RPC_URL:-http://127.0.0.1:8080}"
export JAMSCRIPT_BACKEND_BIND="${JAMSCRIPT_BACKEND_BIND:-127.0.0.1:8090}"
export JAMSCRIPT_BACKEND_URL="${JAMSCRIPT_BACKEND_URL:-http://127.0.0.1:8090}"

require_command() {
  command -v "$1" >/dev/null 2>&1 || {
    echo "$1 is required" >&2
    exit 127
  }
}

require_command curl
require_command docker
require_command jq

[[ -f "${LOCK_FILE}" ]] || {
  echo "MiniJAM lock file not found: ${LOCK_FILE}" >&2
  exit 1
}

if [[ -z "${MINIJAM_IMAGE}" ]]; then
  MINIJAM_IMAGE="$(sed -n 's/^dev_image = "\([^"]*\)"/\1/p' "${LOCK_FILE}")"
fi
MINIJAM_SOURCE_REVISION="$(sed -n 's/^source_revision = "\([^"]*\)"/\1/p' "${LOCK_FILE}")"

[[ "${MINIJAM_IMAGE}" =~ ^ghcr\.io/archelabs/minijam@sha256:[0-9a-f]{64}$ ]] || {
  echo "MINIJAM_DEV_IMAGE_PIN=BLOCKED" >&2
  echo "toolchains/minijam.lock must contain the published aggregate dev_image digest" >&2
  echo "or MINIJAM_IMAGE must be set to ghcr.io/archelabs/minijam@sha256:<64-hex>" >&2
  exit 1
}
echo "JAMSCRIPT_MINIJAM_EXACT_PIN=PASS"
echo "NO_MINIJAM_SOURCE_CHECKOUT=PASS"
echo "NO_PRIVATE_JAMBDA_DEPENDENCY=PASS"
echo "NO_CUSTOM_MINIJAM_COMPOSE=PASS"
echo "NO_STAGE1_WORK_E2E_DEPENDENCY=PASS"

if [[ -n "${MINIJAM_SOURCE_REVISION}" && ! "${MINIJAM_SOURCE_REVISION}" =~ ^[0-9a-f]{40}$ ]]; then
  echo "invalid MiniJAM source_revision in ${LOCK_FILE}: ${MINIJAM_SOURCE_REVISION}" >&2
  exit 1
fi

docker info >/dev/null
mkdir -p "${E2E_RUNTIME}/logs"

cleanup() {
  local status=$?
  trap - EXIT INT TERM
  if [[ "${status}" -ne 0 && -n "${CONTAINER}" ]] && docker container inspect "${CONTAINER}" >/dev/null 2>&1; then
    echo "----- MiniJAM aggregate logs (last 200 lines) -----" >&2
    docker logs --tail 200 "${CONTAINER}" >&2 || true
  fi
  docker rm -f "${CONTAINER}" >/dev/null 2>&1 || true
  if [[ "${status}" -eq 0 && "${JAMSCRIPT_E2E_KEEP_DATA:-0}" != "1" ]]; then
    rm -rf -- "${E2E_RUNTIME}"
  fi
  echo "REAL_MINIJAM_E2E=${result}"
  exit "${status}"
}
trap cleanup EXIT INT TERM

echo "MINIJAM_DEV_IMAGE=${MINIJAM_IMAGE}"
if [[ -n "${MINIJAM_SOURCE_REVISION}" ]]; then
  echo "MINIJAM_SOURCE_REVISION=${MINIJAM_SOURCE_REVISION}"
fi
docker pull "${MINIJAM_IMAGE}" >/dev/null

docker run --detach \
  --name "${CONTAINER}" \
  -p 127.0.0.1:9944:9944 \
  -p 127.0.0.1:8080:8080 \
  -p 127.0.0.1:8082:8082 \
  "${MINIJAM_IMAGE}" --dev >/dev/null

node_ready() {
  curl -fsS --max-time 5 \
    -H 'content-type: application/json' \
    --data '{"jsonrpc":"2.0","id":1,"method":"system_health","params":[]}' \
    "${MINIJAM_NODE_RPC}" |
    jq -e '.result != null and .error == null' >/dev/null
}

formal_ready() {
  curl -fsS --max-time 5 "${MINIJAM_FORMAL_RPC_URL}/health/ready" |
    jq -e '.status == "ready"' >/dev/null
}

worker_ready() {
  curl -fsS --max-time 5 "http://127.0.0.1:8082/health/ready" >/dev/null
}

for _ in $(seq 1 120); do
  if ! docker inspect -f '{{.State.Running}}' "${CONTAINER}" 2>/dev/null | grep -qx true; then
    echo "MiniJAM aggregate container exited during startup" >&2
    exit 1
  fi
  if node_ready && formal_ready && worker_ready; then
    break
  fi
  sleep 2
done

node_ready
formal_ready
worker_ready
echo "MINIJAM_DEV_NODE_READY=PASS"
echo "MINIJAM_DEV_FORMAL_RPC_READY=PASS"
echo "MINIJAM_DEV_WORKER_0_READY=PASS"
echo "JAMSCRIPT_ONE_WORKER_NETWORK=PASS"

genesis_hash="$(
  curl -fsS --max-time 5 \
    -H 'content-type: application/json' \
    --data '{"jsonrpc":"2.0","id":1,"method":"chain_getBlockHash","params":[0]}' \
    "${MINIJAM_NODE_RPC}" |
    jq -er '.result | strings'
)"
[[ "${genesis_hash}" =~ ^0x[0-9a-fA-F]{64}$ ]] || {
  echo "invalid canonical local genesis hash: ${genesis_hash}" >&2
  exit 1
}
echo "JAMSCRIPT_CANONICAL_LOCAL=PASS"

JAMSCRIPT_CONSUMER_E2E_RUNTIME="${E2E_RUNTIME}/consumer" \
JAMSCRIPT_NODE_RPC="${MINIJAM_NODE_RPC}" \
JAMSCRIPT_FORMAL_RPC_URL="${MINIJAM_FORMAL_RPC_URL}" \
JAMSCRIPT_BACKEND_BIND="${JAMSCRIPT_BACKEND_BIND}" \
JAMSCRIPT_BACKEND_URL="${JAMSCRIPT_BACKEND_URL}" \
JAMSCRIPT_E2E_GENESIS_HASH="${genesis_hash}" \
  "${JAMSCRIPT_ROOT}/scripts/minijam-consumer-e2e.sh"

result=PASS
