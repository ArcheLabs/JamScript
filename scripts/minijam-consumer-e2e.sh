#!/usr/bin/env bash
set -euo pipefail

JAMSCRIPT_ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)"
E2E_RUNTIME="${JAMSCRIPT_CONSUMER_E2E_RUNTIME:-${JAMSCRIPT_ROOT}/target/jamscript-network-e2e}"
E2E_PROJECT="${E2E_RUNTIME}/dynamic-state-scriptc-a"
E2E_PROJECT_B="${E2E_RUNTIME}/dynamic-state-scriptc-b"
ARTIFACTS="${E2E_PROJECT}/dist"
ARTIFACTS_B="${E2E_PROJECT_B}/dist"
LOG_DIR="${E2E_RUNTIME}/logs"
BACKEND_LOG="${LOG_DIR}/jamscript-backend.log"
if [[ -n "${JAMSCRIPT_BACKEND_DATA:-}" ]]; then
  BACKEND_DATA="${JAMSCRIPT_BACKEND_DATA}"
  BACKEND_DATA_OWNED=0
else
  BACKEND_DATA="${E2E_RUNTIME}/backend-data"
  BACKEND_DATA_OWNED=1
fi
NODE_RPC="${JAMSCRIPT_NODE_RPC:-http://127.0.0.1:9944}"
FORMAL_RPC="${JAMSCRIPT_FORMAL_RPC_URL:-http://127.0.0.1:8090}"
BACKEND_BIND="${JAMSCRIPT_BACKEND_BIND:-127.0.0.1:8091}"
BACKEND_URL="${JAMSCRIPT_BACKEND_URL:-http://127.0.0.1:8091}"
DEPLOY_TIMEOUT="${JAMSCRIPT_E2E_DEPLOY_TIMEOUT:-240s}"
NPM_REGISTRY="${JAMSCRIPT_NPM_REGISTRY:-}"
backend_pid=""

# A source checkout uses its local rustup/Cargo toolchain by default. Release
# validation can explicitly select the published managed distribution with 0.
export JAMSCRIPT_DEV_TOOLCHAIN="${JAMSCRIPT_DEV_TOOLCHAIN:-1}"
if [[ "${JAMSCRIPT_DEV_TOOLCHAIN}" == "1" ]]; then
  echo "JAMSCRIPT_TOOLCHAIN_MODE=developer"
else
  echo "JAMSCRIPT_TOOLCHAIN_MODE=managed"
fi

for command in cargo curl jq node npm sha256sum; do
  command -v "${command}" >/dev/null 2>&1 || {
    echo "${command} is required" >&2
    exit 127
  }
done

nvm_script="${JAMSCRIPT_NVM_SH:-}"
if [[ -z "${nvm_script}" ]]; then
  task_home="$(cd ~ && pwd -P)"
  nvm_script="${task_home}/.nvm/nvm.sh"
fi
[[ -s "${nvm_script}" ]] || {
  echo "NVM is required to select Node v24.15.0; set JAMSCRIPT_NVM_SH" >&2
  exit 1
}
# shellcheck disable=SC1090
source "${nvm_script}"
nvm use 24.15.0 >/dev/null
[[ "$(node --version)" == "v24.15.0" ]] || {
  echo "consumer E2E requires Node v24.15.0" >&2
  exit 1
}

is_local_endpoint() {
  case "$1" in
    http://localhost|http://localhost/*|http://localhost:*|https://localhost|https://localhost/*|https://localhost:*) return 0 ;;
    http://127.0.0.1|http://127.0.0.1/*|http://127.0.0.1:*|https://127.0.0.1|https://127.0.0.1/*|https://127.0.0.1:*) return 0 ;;
    http://\[::1\]|http://\[::1\]/*|http://\[::1\]:*|https://\[::1\]|https://\[::1\]/*|https://\[::1\]:*) return 0 ;;
    *) return 1 ;;
  esac
}

if [[ "${JAMSCRIPT_ALLOW_REMOTE_MUTATION:-0}" != "1" ]]; then
  for endpoint in "${NODE_RPC}" "${FORMAL_RPC}" "${BACKEND_URL}"; do
    if ! is_local_endpoint "${endpoint}"; then
      echo "remote mutation endpoint requires JAMSCRIPT_ALLOW_REMOTE_MUTATION=1: ${endpoint}" >&2
      exit 1
    fi
  done
fi

rpc_call() {
  local endpoint="$1" method="$2" params="${3:-[]}"
  curl -fsS --max-time 5 -H 'content-type: application/json' \
    --data "$(jq -cn --arg method "${method}" --argjson params "${params}" \
      '{id: 1, jsonrpc: "2.0", method: $method, params: $params}')" \
    "${endpoint}"
}

run_npm() {
  if [[ -n "${NPM_REGISTRY}" ]]; then
    npm --prefix "${JAMSCRIPT_ROOT}/packages/client" --registry "${NPM_REGISTRY}" "$@"
  else
    npm --prefix "${JAMSCRIPT_ROOT}/packages/client" "$@"
  fi
}

block_number() {
  local hash="${1:-}" params='[]' value
  [[ -z "${hash}" ]] || params="$(jq -cn --arg hash "${hash}" '[$hash]')"
  value="$(rpc_call "${NODE_RPC}" chain_getHeader "${params}" | jq -er '.result.number')"
  case "${value}" in
    0x*) printf '%d\n' "$((16#${value#0x}))" ;;
    0X*) printf '%d\n' "$((16#${value#0X}))" ;;
    *) printf '%d\n' "${value}" ;;
  esac
}

formal_ready() {
  curl -fsS --max-time 5 "${FORMAL_RPC}/health/ready" |
    jq -e '.status == "ready"' >/dev/null
}

node_healthy() {
  rpc_call "${NODE_RPC}" system_health |
    jq -e '.result != null and .error == null' >/dev/null
}

wait_for_target_progress() {
  local initial_best="$1" initial_finalized="$2"
  local deadline=$((SECONDS + ${JAMSCRIPT_TARGET_PROGRESS_TIMEOUT_SECONDS:-180}))
  local best finalized_hash finalized
  local best_pass=0 finality_pass=0
  while (( SECONDS < deadline )); do
    best="$(block_number 2>/dev/null || true)"
    if [[ "${best}" =~ ^[0-9]+$ ]] && (( best > initial_best )) && (( best_pass == 0 )); then
      best_pass=1
      echo "JAMSCRIPT_TARGET_BEST_BLOCK_ADVANCES=PASS"
    fi
    finalized_hash="$(rpc_call "${NODE_RPC}" chain_getFinalizedHead 2>/dev/null | jq -er '.result | strings' 2>/dev/null || true)"
    if [[ -n "${finalized_hash}" ]]; then
      finalized="$(block_number "${finalized_hash}" 2>/dev/null || true)"
      if [[ "${finalized}" =~ ^[0-9]+$ ]] && (( finalized > initial_finalized )) && (( finality_pass == 0 )); then
        finality_pass=1
        echo "JAMSCRIPT_TARGET_FINALITY=PASS"
      fi
    fi
    (( best_pass == 1 && finality_pass == 1 )) && return 0
    sleep 2
  done
  echo "target chain did not advance beyond best/finalized ${initial_best}/${initial_finalized}" >&2
  return 1
}

cleanup() {
  local status=$?
  trap - EXIT INT TERM
  if [[ -n "${backend_pid}" ]]; then
    kill "${backend_pid}" 2>/dev/null || true
    wait "${backend_pid}" 2>/dev/null || true
  fi
  if [[ "${status}" -ne 0 ]]; then
    echo "JamScript MiniJAM consumer E2E: FAIL" >&2
    echo "consumer logs: ${LOG_DIR}" >&2
    if [[ -f "${BACKEND_LOG}" ]]; then
      tail -n 100 "${BACKEND_LOG}" >&2 || true
    fi
  fi
  if [[ "${JAMSCRIPT_E2E_KEEP_DATA:-0}" != "1" ]]; then
    rm -rf -- "${E2E_PROJECT}" "${E2E_PROJECT_B}"
    if (( BACKEND_DATA_OWNED == 1 )); then
      rm -rf -- "${BACKEND_DATA}"
    fi
  fi
  exit "${status}"
}
trap cleanup EXIT INT TERM

mkdir -p "${LOG_DIR}"

echo "[preflight] checking external Formal RPC"
formal_ready || {
  echo "external Formal RPC is not ready: ${FORMAL_RPC}" >&2
  exit 1
}
echo "JAMSCRIPT_TARGET_FORMAL_RPC=PASS"

echo "[preflight] checking external node RPC"
node_healthy || {
  echo "external node RPC is not healthy: ${NODE_RPC}" >&2
  exit 1
}
echo "JAMSCRIPT_TARGET_NODE_RPC=PASS"

initial_best="$(block_number)"
initial_finalized_hash="$(rpc_call "${NODE_RPC}" chain_getFinalizedHead | jq -er '.result')"
initial_finalized="$(block_number "${initial_finalized_hash}")"
wait_for_target_progress "${initial_best}" "${initial_finalized}"

genesis_hash="$(
  rpc_call "${NODE_RPC}" chain_getBlockHash '[0]' |
    jq -er '.result | strings'
)"
[[ "${genesis_hash}" =~ ^0x[0-9a-fA-F]{64}$ ]] || {
  echo "invalid genesis hash from external node: ${genesis_hash}" >&2
  exit 1
}
echo "JAMSCRIPT_TARGET_GENESIS=PASS"

(cd "${JAMSCRIPT_ROOT}" && cargo build --locked --bin jams --bin jamscript-service-backend)
if [[ "${JAMSCRIPT_SKIP_NPM_INSTALL:-0}" == "1" ]]; then
  echo "JAMSCRIPT_NPM_INSTALL=SKIPPED"
else
  run_npm ci --no-audit
  echo "JAMSCRIPT_NPM_INSTALL=PASS"
fi
run_npm run build

start_backend() {
  JAMSCRIPT_BACKEND_BIND="${BACKEND_BIND}" \
  JAMSCRIPT_NODE_RPC="${NODE_RPC}" \
  JAMSCRIPT_FORMAL_RPC="${FORMAL_RPC}" \
  JAMSCRIPT_BACKEND_DATA="${BACKEND_DATA}" \
    "${JAMSCRIPT_ROOT}/target/debug/jamscript-service-backend" \
    >"${BACKEND_LOG}" 2>&1 &
  backend_pid=$!
  for _ in $(seq 1 60); do
    if curl -fsS --max-time 3 "${BACKEND_URL}/healthz" >/dev/null 2>&1; then
      break
    fi
    kill -0 "${backend_pid}" 2>/dev/null || break
    sleep 1
  done
  curl -fsS --max-time 5 "${BACKEND_URL}/readinessz" >/dev/null
}

start_backend
echo "JAMSCRIPT_BACKEND=PASS"
backend_binary="${JAMSCRIPT_ROOT}/target/debug/jamscript-service-backend"
backend_binary_hash="$(sha256sum "${backend_binary}" | awk '{print $1}')"

prepare_project() {
  local project="$1" package_name="$2" service_key="$3" instance_id="$4"
  rm -rf -- "${project}"
  cp -R "${JAMSCRIPT_ROOT}/examples/dynamic-state-scriptc" "${project}"
  sed -i \
    -e "s/^name = \"dynamic-state-scriptc\"/name = \"${package_name}\"/" \
    -e "s/\"serviceKey\": \"0x[0-9a-fA-F]*\"/\"serviceKey\": \"${service_key}\"/" \
    -e "s/\"instanceId\": \"0x[0-9a-fA-F]*\"/\"instanceId\": \"${instance_id}\"/" \
    -e "s/\"name\": \"dynamic-state-scriptc\"/\"name\": \"${package_name}\"/" \
    -e "s/^genesis_hash = .*/genesis_hash = \"${genesis_hash}\"/" \
    "${project}/jamscript.toml" "${project}/.jamscript/service.json"
  cat >> "${project}/jamscript.toml" <<EOF

[networks.local]
kind = "minijam"
deployment_rpc = "${FORMAL_RPC}"
node_rpc = "${NODE_RPC}"
backend_rpc = "${BACKEND_URL}"
genesis_hash = "${genesis_hash}"
EOF
  (cd "${JAMSCRIPT_ROOT}" && cargo run --locked --bin jams -- check "${project}")
  (cd "${JAMSCRIPT_ROOT}" && cargo run --locked --bin jams -- build "${project}" --output "${project}/dist")
}

mkdir -p "${E2E_RUNTIME}"
prepare_project "${E2E_PROJECT}" "dynamic-state-scriptc-a" \
  "0x4444444444444444444444444444444444444444444444444444444444444444" \
  "0x6666666666666666666666666666666666666666666666666666666666666666"
prepare_project "${E2E_PROJECT_B}" "dynamic-state-scriptc-b" \
  "0x5555555555555555555555555555555555555555555555555555555555555555" \
  "0x7777777777777777777777777777777777777777777777777777777777777777"

code_hash="$(jq -er '.code_hash' "${ARTIFACTS}/build.json")"
service_key="$(jq -er '.serviceKey // .service_key' "${ARTIFACTS}/build.json")"
code_hash_b="$(jq -er '.code_hash' "${ARTIFACTS_B}/build.json")"
service_key_b="$(jq -er '.serviceKey // .service_key' "${ARTIFACTS_B}/build.json")"

deployment_json="$(
  cd "${JAMSCRIPT_ROOT}"
  cargo run --locked --bin jams -- deploy "${E2E_PROJECT}" \
    --network local --artifact "${ARTIFACTS}" --timeout "${DEPLOY_TIMEOUT}" --json
)"
deployment_json_b="$(
  cd "${JAMSCRIPT_ROOT}"
  cargo run --locked --bin jams -- deploy "${E2E_PROJECT_B}" \
    --network local --artifact "${ARTIFACTS_B}" --timeout "${DEPLOY_TIMEOUT}" --json
)"

validate_deployment() {
  local payload="$1" expected_hash="$2" label="$3"
  DEPLOYMENT_JSON="${payload}" EXPECTED_CODE_HASH="${expected_hash}" node --input-type=module -e '
    const deployment = JSON.parse(process.env.DEPLOYMENT_JSON);
    const expected = process.env.EXPECTED_CODE_HASH.toLowerCase();
    if (deployment.status !== "PASS") throw new Error(`${process.argv[1]} deployment status was not PASS`);
    if (!Number.isInteger(deployment.serviceId)) throw new Error(`${process.argv[1]} serviceId is invalid`);
    if (String(deployment.codeHash).toLowerCase() !== expected) throw new Error(`${process.argv[1]} codeHash mismatch`);
    if (deployment.finalized !== true) throw new Error(`${process.argv[1]} deployment is not finalized`);
    if (deployment.backendRegistration?.status !== "PASS") throw new Error(`${process.argv[1]} backend registration failed`);
    process.stdout.write(String(deployment.serviceId));
  ' "${label}"
}

service_id="$(validate_deployment "${deployment_json}" "${code_hash}" "Service A")"
service_id_b="$(validate_deployment "${deployment_json_b}" "${code_hash_b}" "Service B")"
echo "JAMSCRIPT_SERVICE_A=PASS"
echo "JAMSCRIPT_SERVICE_A_DEPLOY=PASS"
echo "JAMSCRIPT_SERVICE_B=PASS"
echo "JAMSCRIPT_SERVICE_B_DEPLOY=PASS"

[[ "${service_id}" != "${service_id_b}" ]] || {
  echo "the two deployments received the same service ID: ${service_id}" >&2
  exit 1
}
echo "JAMSCRIPT_SERVICE_IDS_DIFFER=PASS"
echo "JAMSCRIPT_MULTI_SERVICE_IDS=PASS"

registry_json="$(rpc_call "${BACKEND_URL}" jamscript_listServicesV1 '{}')"
SERVICE_REGISTRY_JSON="${registry_json}" SERVICE_ID_A="${service_id}" SERVICE_ID_B="${service_id_b}" \
  node --input-type=module -e '
    const response = JSON.parse(process.env.SERVICE_REGISTRY_JSON);
    const services = response.result;
    if (!Array.isArray(services) || services.length < 2) throw new Error("backend registry has fewer than two services");
    const ids = new Set(services.map((service) => String(service.serviceId)));
    if (!ids.has(process.env.SERVICE_ID_A) || !ids.has(process.env.SERVICE_ID_B)) throw new Error("backend registry is missing a deployment");
  '
echo "JAMSCRIPT_BACKEND_MULTI_SERVICE=PASS"

[[ -d "${ARTIFACTS}" && -d "${ARTIFACTS_B}" ]] || {
  echo "consumer artifacts disappeared before client validation" >&2
  exit 1
}
JAMSCRIPT_E2E_ARTIFACTS="${ARTIFACTS}" \
JAMSCRIPT_E2E_SERVICE_ID="${service_id}" \
JAMSCRIPT_E2E_SERVICE_KEY="${service_key}" \
JAMSCRIPT_E2E_CODE_HASH="${code_hash}" \
JAMSCRIPT_E2E_ARTIFACTS_B="${ARTIFACTS_B}" \
JAMSCRIPT_E2E_SERVICE_ID_B="${service_id_b}" \
JAMSCRIPT_E2E_SERVICE_KEY_B="${service_key_b}" \
JAMSCRIPT_E2E_CODE_HASH_B="${code_hash_b}" \
JAMSCRIPT_E2E_GENESIS_HASH="${genesis_hash}" \
JAMSCRIPT_E2E_BACKEND_URL="${BACKEND_URL}" \
JAMSCRIPT_E2E_LOG_DIR="${LOG_DIR}" \
  run_npm run test:network

kill -0 "${backend_pid}" 2>/dev/null || {
  echo "backend stopped during consumer E2E" >&2
  exit 1
}
[[ "$(sha256sum "${backend_binary}" | awk '{print $1}')" == "${backend_binary_hash}" ]] || {
  echo "backend binary changed during the multi-service deployment" >&2
  exit 1
}
echo "NO_BACKEND_RECOMPILE=PASS"
echo "NO_BACKEND_RESTART=PASS"
echo "JAMSCRIPT_EXTERNAL_NETWORK_E2E=PASS"
echo "JAMSCRIPT_EXTERNAL_NETWORK_MULTI_SERVICE_E2E=PASS"
echo "REAL_MINIJAM_E2E=PASS"
echo "REAL_MINIJAM_MULTI_SERVICE_E2E=PASS"
