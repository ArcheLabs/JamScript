#!/usr/bin/env bash
set -euo pipefail

JAMSCRIPT_ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)"
MINIJAM_ROOT="${JAMSCRIPT_MINIJAM_SDK:-${JAMSCRIPT_ROOT}/../minijam-client}"
MINIJAM_ROOT="$(cd -- "${MINIJAM_ROOT}" && pwd -P)"
E2E_RUNTIME="${MINIJAM_ROOT}/.local/jamscript-network-e2e"
E2E_PROJECT="${E2E_RUNTIME}/dynamic-state-scriptc"
ARTIFACTS="${E2E_PROJECT}/dist"
E2E_PROJECT_B="${E2E_RUNTIME}/dynamic-state-scriptc-b"
ARTIFACTS_B="${E2E_PROJECT_B}/dist"
LOCK_FILE="${JAMSCRIPT_ROOT}/toolchains/minijam.lock"
minijam_result=FAIL

export JAMSCRIPT_MINIJAM_SDK="${MINIJAM_ROOT}"
# The checked-in distribution manifest is intentionally unpublished in a
# source checkout. This integration gate uses the local toolchain by default;
# CI/release jobs can set JAMSCRIPT_DEV_TOOLCHAIN=0 to exercise the published
# bundle path instead.
export JAMSCRIPT_DEV_TOOLCHAIN="${JAMSCRIPT_DEV_TOOLCHAIN:-1}"
export MINIJAM_ENABLE_FORMAL_RPC=true
export MINIJAM_NATIVE_RUNTIME_ROOT="${E2E_RUNTIME}"
export MINIJAM_FORMAL_RPC_BIND="${MINIJAM_FORMAL_RPC_BIND:-127.0.0.1:8090}"
export MINIJAM_FORMAL_RPC_URL="${MINIJAM_FORMAL_RPC_URL:-http://127.0.0.1:8090}"
export JAMSCRIPT_BACKEND_BIND="${JAMSCRIPT_BACKEND_BIND:-127.0.0.1:8091}"
export JAMSCRIPT_BACKEND_URL="${JAMSCRIPT_BACKEND_URL:-http://127.0.0.1:8091}"
export JAMSCRIPT_BACKEND_DATA="${JAMSCRIPT_BACKEND_DATA:-${E2E_RUNTIME}/backend-data}"
export MINIJAM_NODE_RPC="${MINIJAM_NODE_RPC:-http://127.0.0.1:9944}"
export JAMSCRIPT_NODE_RPC="${MINIJAM_NODE_RPC}"
export JAMSCRIPT_FORMAL_RPC="${MINIJAM_FORMAL_RPC_URL}"
export MINIJAM_FORMAL_RELAYER_URI="${MINIJAM_FORMAL_RELAYER_URI:-0x9292929292929292929292929292929292929292929292929292929292929292}"

nvm_script="${JAMSCRIPT_NVM_SH:-}"
if [[ -z "${nvm_script}" ]]; then
  task_home="$(cd ~ && pwd -P)"
  nvm_script="${task_home}/.nvm/nvm.sh"
fi
if [[ -s "${nvm_script}" ]]; then
  # shellcheck disable=SC1090
  source "${nvm_script}"
  nvm use 24.15.0 >/dev/null
fi
[[ "$(node --version)" == "v24.15.0" ]] || {
  echo "ScriptC M2 requires Node v24.15.0; set JAMSCRIPT_NVM_SH or activate that Node version" >&2
  exit 1
}

cleanup() {
  local status=$?
  trap - EXIT INT TERM
  if [[ -n "${backend_pid:-}" ]]; then
    kill "${backend_pid}" 2>/dev/null || true
    wait "${backend_pid}" 2>/dev/null || true
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
  echo "REAL_MINIJAM_E2E=${minijam_result}"
  echo "REAL_MINIJAM_MULTI_SERVICE_E2E=${minijam_result}"
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

[[ -x "${MINIJAM_ROOT}/scripts/check-minijam-boundary.sh" ]] || {
  echo "MiniJAM boundary check is unavailable in ${MINIJAM_ROOT}" >&2
  exit 1
}
(cd "${MINIJAM_ROOT}" && "${MINIJAM_ROOT}/scripts/check-minijam-boundary.sh")

mkdir -p "${E2E_RUNTIME}/logs"
rm -f "${E2E_RUNTIME}"/logs/*.log

echo "[prepare] MiniJAM revision: ${actual_revision}"
"${MINIJAM_ROOT}/scripts/stage0-native.sh" deps
"${MINIJAM_ROOT}/scripts/stage0-native.sh" build

(cd "${JAMSCRIPT_ROOT}" && cargo build --locked --bin jams)
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

echo "[network] starting one dynamic PVM backend"
(cd "${JAMSCRIPT_ROOT}" && cargo build --locked --bin jamscript-service-backend)
JAMSCRIPT_BACKEND_BIND="${JAMSCRIPT_BACKEND_BIND}" \
JAMSCRIPT_NODE_RPC="${MINIJAM_NODE_RPC}" \
JAMSCRIPT_FORMAL_RPC="${MINIJAM_FORMAL_RPC_URL}" \
JAMSCRIPT_BACKEND_DATA="${JAMSCRIPT_BACKEND_DATA}" \
  "${JAMSCRIPT_ROOT}/target/debug/jamscript-service-backend" \
  >"${E2E_RUNTIME}/logs/jamscript-backend.log" 2>&1 &
backend_pid=$!
for _ in $(seq 1 60); do
  if curl -fsS "${JAMSCRIPT_BACKEND_URL}/healthz" >/dev/null 2>&1; then
    break
  fi
  sleep 1
done
curl -fsS "${JAMSCRIPT_BACKEND_URL}/readinessz" >/dev/null
echo "[network] dynamic PVM backend ready"
backend_binary_hash="$(sha256sum "${JAMSCRIPT_ROOT}/target/debug/jamscript-service-backend" | awk '{print $1}')"

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

prepare_project() {
  local project="$1"
  local artifacts="$2"
  local package_name="$3"
  local service_key="$4"
  rm -rf "${project}"
  cp -R "${JAMSCRIPT_ROOT}/examples/dynamic-state-scriptc" "${project}"
  sed -i \
    -e "s/^name = \"dynamic-state-scriptc\"/name = \"${package_name}\"/" \
    -e "s/\"serviceKey\": \"0x[0-9a-fA-F]*\"/\"serviceKey\": \"${service_key}\"/" \
    -e "s/\"name\": \"dynamic-state-scriptc\"/\"name\": \"${package_name}\"/" \
    -e "s/^genesis_hash = .*/genesis_hash = \"${genesis_hash}\"/" \
    "${project}/jamscript.toml" "${project}/.jamscript/service.json"
  cat >> "${project}/jamscript.toml" <<EOF

[networks.local]
kind = "minijam"
deployment_rpc = "${MINIJAM_FORMAL_RPC_URL}"
node_rpc = "${MINIJAM_NODE_RPC}"
backend_rpc = "${JAMSCRIPT_BACKEND_URL}"
genesis_hash = "${genesis_hash}"
EOF
  (cd "${JAMSCRIPT_ROOT}" && cargo run --locked --bin jams -- check "${project}")
  (cd "${JAMSCRIPT_ROOT}" && cargo run --locked --bin jams -- build "${project}" --output "${artifacts}")
}

mkdir -p "${E2E_RUNTIME}"
prepare_project "${E2E_PROJECT}" "${ARTIFACTS}" "dynamic-state-scriptc" "0x4444444444444444444444444444444444444444444444444444444444444444"
prepare_project "${E2E_PROJECT_B}" "${ARTIFACTS_B}" "dynamic-state-scriptc-b" "0x5555555555555555555555555555555555555555555555555555555555555555"
code_hash="$(
  node --input-type=module -e 'let b="";process.stdin.on("data",c=>b+=c);process.stdin.on("end",()=>process.stdout.write(JSON.parse(b).code_hash));' < "${ARTIFACTS}/build.json"
)"
service_key="$(
  node --input-type=module -e 'let b="";process.stdin.on("data",c=>b+=c);process.stdin.on("end",()=>{const v=JSON.parse(b);process.stdout.write(v.serviceKey ?? v.service_key);});' < "${ARTIFACTS}/build.json"
)"
echo "[build] JamScript service built: ${ARTIFACTS}/service.blob"

code_hash_b="$(
  node --input-type=module -e 'let b="";process.stdin.on("data",c=>b+=c);process.stdin.on("end",()=>process.stdout.write(JSON.parse(b).code_hash));' < "${ARTIFACTS_B}/build.json"
)"
service_key_b="$(
  node --input-type=module -e 'let b="";process.stdin.on("data",c=>b+=c);process.stdin.on("end",()=>{const v=JSON.parse(b);process.stdout.write(v.serviceKey ?? v.service_key);});' < "${ARTIFACTS_B}/build.json"
)"
echo "[build] second JamScript service built: ${ARTIFACTS_B}/service.blob"

deployment_json="$(
  cd "${JAMSCRIPT_ROOT}"
  cargo run --locked --bin jams -- deploy "${E2E_PROJECT}" \
    --network local --artifact "${ARTIFACTS}" --json
)"
service_id="$(
  node --input-type=module -e 'let b="";process.stdin.on("data",c=>b+=c);process.stdin.on("end",()=>process.stdout.write(String(JSON.parse(b).serviceId)));' <<<"${deployment_json}"
)"
code_hash="$(
  node --input-type=module -e 'let b="";process.stdin.on("data",c=>b+=c);process.stdin.on("end",()=>process.stdout.write(JSON.parse(b).codeHash));' <<<"${deployment_json}"
)"
echo "[deploy] JamScript Service ${service_id} created through minijam_createServiceV1"

deployment_json_b="$(
  cd "${JAMSCRIPT_ROOT}"
  cargo run --locked --bin jams -- deploy "${E2E_PROJECT_B}" \
    --network local --artifact "${ARTIFACTS_B}" --json
)"
service_id_b="$(
  node --input-type=module -e 'let b="";process.stdin.on("data",c=>b+=c);process.stdin.on("end",()=>process.stdout.write(String(JSON.parse(b).serviceId)));' <<<"${deployment_json_b}"
)"
code_hash_b="$(
  node --input-type=module -e 'let b="";process.stdin.on("data",c=>b+=c);process.stdin.on("end",()=>process.stdout.write(JSON.parse(b).codeHash));' <<<"${deployment_json_b}"
)"
echo "[deploy] JamScript Service ${service_id_b} created through minijam_createServiceV1"

service_count="$(
  curl -fsS -H "content-type: application/json" \
    --data '{"jsonrpc":"2.0","id":1,"method":"jamscript_listServicesV1","params":{}}' \
    "${JAMSCRIPT_BACKEND_URL}" |
    node --input-type=module -e 'let b="";process.stdin.on("data",c=>b+=c);process.stdin.on("end",()=>{const r=JSON.parse(b);if(r.error)throw new Error(JSON.stringify(r.error));process.stdout.write(String(r.result.length));});'
)"
[[ "${service_count}" -ge 2 ]] || {
  echo "dynamic backend registered fewer than two Services: ${service_count}" >&2
  exit 1
}
[[ "$(sha256sum "${JAMSCRIPT_ROOT}/target/debug/jamscript-service-backend" | awk '{print $1}')" == "${backend_binary_hash}" ]] || {
  echo "backend binary changed during the multi-Service deployment" >&2
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
JAMSCRIPT_E2E_BACKEND_URL="${JAMSCRIPT_BACKEND_URL}" \
JAMSCRIPT_E2E_LOG_DIR="${E2E_RUNTIME}/logs" \
npm --prefix "${JAMSCRIPT_ROOT}/packages/client" run test:network
minijam_result=PASS
kill -0 "${backend_pid}"
echo "NO_BACKEND_RECOMPILE=PASS"
echo "NO_BACKEND_RESTART=PASS"
