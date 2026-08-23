#!/usr/bin/env bash
set -euo pipefail

JAMSCRIPT_ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)"
MINIJAM_ROOT="${JAMSCRIPT_MINIJAM_SDK:-${JAMSCRIPT_ROOT}/../minijam-client}"
MINIJAM_ROOT="$(cd -- "${MINIJAM_ROOT}" && pwd -P)"
E2E_RUNTIME="${MINIJAM_ROOT}/.local/jamscript-network-e2e"
E2E_PROJECT="${E2E_RUNTIME}/game-replay"
ARTIFACTS="${E2E_PROJECT}/dist"
LOCK_FILE="${JAMSCRIPT_ROOT}/toolchains/minijam.lock"

export JAMSCRIPT_MINIJAM_SDK="${MINIJAM_ROOT}"
export MINIJAM_ENABLE_FORMAL_RPC=true
export MINIJAM_NATIVE_RUNTIME_ROOT="${E2E_RUNTIME}"
export MINIJAM_FORMAL_RPC_BIND="${MINIJAM_FORMAL_RPC_BIND:-127.0.0.1:8090}"
export MINIJAM_FORMAL_RPC_URL="${MINIJAM_FORMAL_RPC_URL:-http://127.0.0.1:8090}"
export JAMSCRIPT_ADAPTER_BIND="${JAMSCRIPT_ADAPTER_BIND:-127.0.0.1:8091}"
export JAMSCRIPT_ADAPTER_URL="${JAMSCRIPT_ADAPTER_URL:-http://127.0.0.1:8091}"
export MINIJAM_NODE_RPC="${MINIJAM_NODE_RPC:-http://127.0.0.1:9944}"
export MINIJAM_WORK_RPC="${MINIJAM_WORK_RPC:-${JAMSCRIPT_ADAPTER_URL}}"
export MINIJAM_STATE_RPC="${MINIJAM_STATE_RPC:-${JAMSCRIPT_ADAPTER_URL}}"
export MINIJAM_FORMAL_RELAYER_URI="${MINIJAM_FORMAL_RELAYER_URI:-0x9292929292929292929292929292929292929292929292929292929292929292}"

cleanup() {
  local status=$?
  trap - EXIT INT TERM
  if [[ -n "${adapter_pid:-}" ]]; then
    kill "${adapter_pid}" 2>/dev/null || true
    wait "${adapter_pid}" 2>/dev/null || true
  fi
  "${MINIJAM_ROOT}/scripts/stage0-native.sh" down || true
  if [[ "${status}" -ne 0 ]]; then
    echo "JamScript MiniJAM network E2E: FAIL" >&2
    echo "MiniJAM logs: ${E2E_RUNTIME}/logs" >&2
    echo "JamScript artifacts: ${E2E_RUNTIME}" >&2
    for log in "${E2E_RUNTIME}"/logs/*.log; do
      [[ -f "${log}" ]] || continue
      echo "----- ${log} (last 100 lines) -----" >&2
      tail -n 100 "${log}" >&2 || true
    done
  elif [[ "${JAMSCRIPT_E2E_KEEP_DATA:-0}" != "1" ]]; then
    rm -rf "${E2E_RUNTIME}/data" "${E2E_RUNTIME}/run"
  fi
  exit "${status}"
}
trap cleanup EXIT INT TERM

[[ -x "${MINIJAM_ROOT}/scripts/stage0-native.sh" ]] || {
  echo "MiniJAM checkout not found: ${MINIJAM_ROOT}" >&2
  exit 1
}
[[ -f "${LOCK_FILE}" ]] || {
  echo "MiniJAM lock file not found: ${LOCK_FILE}" >&2
  exit 1
}

locked_revision="$(sed -n 's/^revision = "\([^"]*\)"/\1/p' "${LOCK_FILE}")"
actual_revision="$(git -C "${MINIJAM_ROOT}" rev-parse HEAD)"
if [[ "${actual_revision}" != "${locked_revision}" ]]; then
  if [[ "${CI:-}" == "true" ]]; then
    echo "MiniJAM checkout ${actual_revision} does not match locked revision ${locked_revision}" >&2
    exit 1
  fi
  echo "warning: MiniJAM checkout ${actual_revision} does not match locked revision ${locked_revision}" >&2
fi

mkdir -p "${E2E_RUNTIME}/logs"
rm -f "${E2E_RUNTIME}"/logs/*.log

echo "[prepare] MiniJAM revision: ${actual_revision}"
"${MINIJAM_ROOT}/scripts/stage0-native.sh" deps
"${MINIJAM_ROOT}/scripts/stage0-native.sh" build

(cd "${JAMSCRIPT_ROOT}" && cargo build --locked --bin jamscript)
(cd "${JAMSCRIPT_ROOT}" && cargo build --locked --bin managed-state-network-adapter)
npm --prefix "${JAMSCRIPT_ROOT}/packages/client" ci --no-audit
npm --prefix "${JAMSCRIPT_ROOT}/packages/client" run build

echo "[network] starting isolated MiniJAM network"
"${MINIJAM_ROOT}/scripts/stage0-native.sh" reset
"${MINIJAM_ROOT}/scripts/stage0-native.sh" up
curl -fsS "${MINIJAM_FORMAL_RPC_URL}/health/ready" >/dev/null
curl -fsS \
  -H "content-type: application/json" \
  --data '{"jsonrpc":"2.0","id":1,"method":"system_health","params":[]}' \
  "${MINIJAM_NODE_RPC}" >/dev/null
echo "[network] MiniJAM node ready"
echo "[network] Formal Work RPC ready"
echo "[network] Workers ready"

genesis_hash="$(
  curl -fsS \
    -H "content-type: application/json" \
    --data '{"jsonrpc":"2.0","id":1,"method":"chain_getBlockHash","params":[0]}' \
    "${MINIJAM_NODE_RPC}" |
    node -e 'let b="";process.stdin.on("data",c=>b+=c);process.stdin.on("end",()=>{const r=JSON.parse(b);if(r.error)throw new Error(JSON.stringify(r.error));process.stdout.write(r.result);});'
)"
[[ "${genesis_hash}" =~ ^0x[0-9a-fA-F]{64}$ ]] || {
  echo "invalid genesis hash from MiniJAM node: ${genesis_hash}" >&2
  exit 1
}

placeholder_blob="${MINIJAM_ROOT}/examples/services/counter/artifacts/counter-c.blob"
placeholder_code_hash="$(
  cd "${JAMSCRIPT_ROOT}/packages/client"
  node --input-type=module -e 'import fs from "node:fs"; import { blake2AsHex } from "@polkadot/util-crypto"; process.stdout.write(blake2AsHex(fs.readFileSync(process.argv[1]), 256));' "${placeholder_blob}"
)"
provision_json="$(
  node "${JAMSCRIPT_ROOT}/packages/client/tests/support/provision-service.mjs" \
    --command create \
    --base-url http://127.0.0.1:8080 \
    --blob "${placeholder_blob}" \
    --code-hash "${placeholder_code_hash}"
)"
service_id="$(
  node --input-type=module -e 'let b="";process.stdin.on("data",c=>b+=c);process.stdin.on("end",()=>process.stdout.write(String(JSON.parse(b).serviceId)));' <<<"${provision_json}"
)"
echo "[provision] placeholder service created: ${service_id}"

rm -rf "${E2E_PROJECT}"
mkdir -p "${E2E_RUNTIME}"
cp -R "${JAMSCRIPT_ROOT}/examples/game-replay" "${E2E_PROJECT}"
sed -i \
  -e "s/^service_id = .*/service_id = ${service_id}/" \
  -e "s/^genesis_hash = .*/genesis_hash = \"${genesis_hash}\"/" \
  "${E2E_PROJECT}/jamscript.toml"

(cd "${JAMSCRIPT_ROOT}" && cargo run --locked --bin jamscript -- check "${E2E_PROJECT}")
(cd "${JAMSCRIPT_ROOT}" && cargo run --locked --bin jamscript -- build "${E2E_PROJECT}" --output "${ARTIFACTS}")
code_hash="$(
  node --input-type=module -e 'let b="";process.stdin.on("data",c=>b+=c);process.stdin.on("end",()=>process.stdout.write(JSON.parse(b).code_hash));' < "${ARTIFACTS}/build.json"
)"
service_key="$(
  node --input-type=module -e 'let b="";process.stdin.on("data",c=>b+=c);process.stdin.on("end",()=>{const v=JSON.parse(b);process.stdout.write(v.serviceKey ?? v.service_key);});' < "${ARTIFACTS}/build.json"
)"
echo "[build] JamScript service built: ${ARTIFACTS}/service.blob"

node "${JAMSCRIPT_ROOT}/packages/client/tests/support/provision-service.mjs" \
  --command upgrade \
  --base-url http://127.0.0.1:8080 \
  --service-id "${service_id}" \
  --blob "${ARTIFACTS}/service.blob" \
  --code-hash "${code_hash}" >/dev/null
echo "[provision] finalized code hash verified for service ${service_id}"

JAMSCRIPT_E2E_SERVICE_ID="${service_id}" \
JAMSCRIPT_E2E_SERVICE_KEY="${service_key}" \
JAMSCRIPT_E2E_CODE_HASH="${code_hash}" \
JAMSCRIPT_E2E_GENESIS_HASH="${genesis_hash}" \
JAMSCRIPT_E2E_TEST_METHODS=true \
"${JAMSCRIPT_ROOT}/target/debug/managed-state-network-adapter" \
  >"${E2E_RUNTIME}/logs/jamscript-adapter.log" 2>&1 &
adapter_pid=$!
for _ in $(seq 1 60); do
  if curl -fsS "${JAMSCRIPT_ADAPTER_URL}/health/ready" >/dev/null 2>&1; then
    break
  fi
  sleep 1
done
curl -fsS "${JAMSCRIPT_ADAPTER_URL}/health/ready" >/dev/null
echo "[network] JamScript managed-state Builder/Provider RPC ready"

JAMSCRIPT_E2E_ARTIFACTS="${ARTIFACTS}" \
JAMSCRIPT_E2E_SERVICE_ID="${service_id}" \
JAMSCRIPT_E2E_SERVICE_KEY="${service_key}" \
JAMSCRIPT_E2E_CODE_HASH="${code_hash}" \
JAMSCRIPT_E2E_GENESIS_HASH="${genesis_hash}" \
JAMSCRIPT_E2E_LOG_DIR="${E2E_RUNTIME}/logs" \
npm --prefix "${JAMSCRIPT_ROOT}/packages/client" run test:network
