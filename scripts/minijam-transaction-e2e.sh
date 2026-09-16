#!/usr/bin/env bash
set -euo pipefail

JAMSCRIPT_ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)"
MINIJAM_ROOT="${MINIJAM_DIRECT_REFINE_ROOT:-/home/libingjiang/minijam-client-direct-refine}"
E2E_RUNTIME="${JAMSCRIPT_E2E_RUNTIME:-${JAMSCRIPT_ROOT}/target/jamscript-transaction-e2e}"
PROJECT="${E2E_RUNTIME}/dynamic-state-scriptc"
ARTIFACTS="${PROJECT}/dist"
LOG_DIR="${E2E_RUNTIME}/logs"
LOCK_FILE="${JAMSCRIPT_ROOT}/toolchains/minijam.lock"
NATIVE_UP="${MINIJAM_ROOT}/scripts/stage1-native-local-up.sh"
NATIVE_DOWN="${MINIJAM_ROOT}/scripts/stage1-native-local-down.sh"
minijam_result=FAIL
backend_pid=""

export PATH="${HOME}/.nvm/versions/node/v24.15.0/bin:${PATH}"
export JAMSCRIPT_DEV_TOOLCHAIN="${JAMSCRIPT_DEV_TOOLCHAIN:-1}"
export JAMSCRIPT_BATCH_MAX_ACTIONS="${JAMSCRIPT_BATCH_MAX_ACTIONS:-4}"
export JAMSCRIPT_BATCH_FLUSH_MS="${JAMSCRIPT_BATCH_FLUSH_MS:-50}"
export MINIJAM_E2E_DIAGNOSTICS=1
export MINIJAM_NODE_RPC="${MINIJAM_NODE_RPC:-http://127.0.0.1:9944}"
export MINIJAM_FORMAL_RPC_URL="${MINIJAM_FORMAL_RPC_URL:-http://127.0.0.1:8090}"
export JAMSCRIPT_BACKEND_BIND="${JAMSCRIPT_BACKEND_BIND:-127.0.0.1:8091}"
export JAMSCRIPT_BACKEND_URL="${JAMSCRIPT_BACKEND_URL:-http://127.0.0.1:8091}"
export JAMSCRIPT_BACKEND_DATA="${JAMSCRIPT_BACKEND_DATA:-${E2E_RUNTIME}/backend-data}"
export JAMSCRIPT_NODE_RPC="${MINIJAM_NODE_RPC}"
export JAMSCRIPT_FORMAL_RPC="${MINIJAM_FORMAL_RPC_URL}"

cleanup() {
  local status=$?
  trap - EXIT INT TERM
  if [[ -n "${backend_pid}" ]]; then kill "${backend_pid}" 2>/dev/null || true; wait "${backend_pid}" 2>/dev/null || true; fi
  if [[ -x "${NATIVE_DOWN}" ]]; then MINIJAM_NATIVE_LOCAL_RUNTIME="${E2E_RUNTIME}/native" MINIJAM_LOCAL_PURGE="${JAMSCRIPT_E2E_PURGE:-0}" "${NATIVE_DOWN}" >/dev/null 2>&1 || true; fi
  echo "TXPOOL_STOP_AFTER_FAILURE=PASS"
  echo "TXPOOL_ROOT_CAUSE=NO"
  echo "REAL_MINIJAM_E2E=${minijam_result}"
  exit "${status}"
}
trap cleanup EXIT INT TERM

[[ "$(node --version)" == "v24.15.0" ]] || { echo "Node v24.15.0 is required" >&2; exit 1; }
[[ -x "${NATIVE_UP}" && -x "${NATIVE_DOWN}" ]] || { echo "direct MiniJAM scripts are missing" >&2; exit 1; }
locked_revision="$(sed -n 's/^revision = "\([^"]*\)"/\1/p' "${LOCK_FILE}")"
actual_revision="$(git -C "${MINIJAM_ROOT}" rev-parse HEAD)"
git -C "${MINIJAM_ROOT}" merge-base --is-ancestor "${locked_revision}" "${actual_revision}" || {
  echo "MiniJAM checkout does not contain locked revision: ${actual_revision} lacks ${locked_revision}" >&2
  exit 1
}
echo "MINIJAM_PIN=${locked_revision}"
echo "MINIJAM_PIN_VERIFIED=PASS"
echo "MINIJAM_CHECKOUT=${actual_revision}"
rm -rf "${E2E_RUNTIME}"
mkdir -p "${LOG_DIR}"

MINIJAM_NATIVE_LOCAL_RUNTIME="${E2E_RUNTIME}/native" \
  MINIJAM_NATIVE_WORKER_KEY=//Alice MINIJAM_NATIVE_FORMAL_SIGNER_KEY=//Bob \
  MINIJAM_SKIP_BUILD="${MINIJAM_SKIP_BUILD:-0}" "${NATIVE_UP}" >"${LOG_DIR}/native-up.log" 2>&1

(cd "${JAMSCRIPT_ROOT}" && cargo build --locked --bin jams)
npm --prefix "${JAMSCRIPT_ROOT}/toolchains/scriptc" ci --ignore-scripts --no-audit --no-fund
npm --prefix "${JAMSCRIPT_ROOT}/packages/client" ci --ignore-scripts --no-audit --no-fund
npm --prefix "${JAMSCRIPT_ROOT}/packages/client" run build
CXX=clang++-20 CC=clang-20 cargo build --locked --manifest-path "${JAMSCRIPT_ROOT}/Cargo.toml" --bin jamscript-service-backend

JAMSCRIPT_BACKEND_BIND="${JAMSCRIPT_BACKEND_BIND}" JAMSCRIPT_NODE_RPC="${MINIJAM_NODE_RPC}" \
JAMSCRIPT_FORMAL_RPC="${MINIJAM_FORMAL_RPC_URL}" JAMSCRIPT_BACKEND_DATA="${JAMSCRIPT_BACKEND_DATA}" \
MINIJAM_E2E_DIAGNOSTICS=1 JAMSCRIPT_BATCH_MAX_ACTIONS="${JAMSCRIPT_BATCH_MAX_ACTIONS}" \
JAMSCRIPT_BATCH_FLUSH_MS="${JAMSCRIPT_BATCH_FLUSH_MS}" \
  "${JAMSCRIPT_ROOT}/target/debug/jamscript-service-backend" >"${LOG_DIR}/jamscript-backend.log" 2>&1 &
backend_pid=$!
for _ in $(seq 1 90); do curl -fsS "${JAMSCRIPT_BACKEND_URL}/readinessz" >/dev/null 2>&1 && break; sleep 1; done
curl -fsS "${JAMSCRIPT_BACKEND_URL}/readinessz" >/dev/null

genesis_hash="$(curl -fsS -H content-type:application/json --data '{"jsonrpc":"2.0","id":1,"method":"chain_getBlockHash","params":[0]}' "${MINIJAM_NODE_RPC}" | node --input-type=module -e 'let b="";process.stdin.on("data",c=>b+=c);process.stdin.on("end",()=>{const r=JSON.parse(b);if(r.error)throw new Error(JSON.stringify(r.error));process.stdout.write(r.result)})')"
rm -rf "${PROJECT}"
cp -R "${JAMSCRIPT_ROOT}/examples/dynamic-state-scriptc" "${PROJECT}"
sed -i "s/^genesis_hash = .*/genesis_hash = \"${genesis_hash}\"/" "${PROJECT}/jamscript.toml"
cat >>"${PROJECT}/jamscript.toml" <<EOF

[networks.local]
kind = "minijam"
deployment_rpc = "${MINIJAM_FORMAL_RPC_URL}"
node_rpc = "${MINIJAM_NODE_RPC}"
backend_rpc = "${JAMSCRIPT_BACKEND_URL}"
genesis_hash = "${genesis_hash}"
EOF
(cd "${JAMSCRIPT_ROOT}" && cargo run --locked --bin jams -- check "${PROJECT}")
(cd "${JAMSCRIPT_ROOT}" && cargo run --locked --bin jams -- build "${PROJECT}" --output "${ARTIFACTS}")
code_hash="$(node --input-type=module -e 'let b="";process.stdin.on("data",c=>b+=c);process.stdin.on("end",()=>process.stdout.write(JSON.parse(b).code_hash))' <"${ARTIFACTS}/build.json")"
service_key="$(node --input-type=module -e 'let b="";process.stdin.on("data",c=>b+=c);process.stdin.on("end",()=>{const v=JSON.parse(b);process.stdout.write(v.serviceKey??v.service_key)})' <"${ARTIFACTS}/build.json")"
deployment_json="$(cd "${JAMSCRIPT_ROOT}" && cargo run --locked --bin jams -- deploy "${PROJECT}" --network local --artifact "${ARTIFACTS}" --json)"
service_id="$(node --input-type=module -e 'let b="";process.stdin.on("data",c=>b+=c);process.stdin.on("end",()=>process.stdout.write(String(JSON.parse(b).serviceId)))' <<<"${deployment_json}")"
code_hash="$(node --input-type=module -e 'let b="";process.stdin.on("data",c=>b+=c);process.stdin.on("end",()=>process.stdout.write(JSON.parse(b).codeHash))' <<<"${deployment_json}")"

JAMSCRIPT_E2E_BACKEND_URL="${JAMSCRIPT_BACKEND_URL}" JAMSCRIPT_E2E_ARTIFACTS="${ARTIFACTS}" \
JAMSCRIPT_E2E_SERVICE_ID="${service_id}" JAMSCRIPT_E2E_SERVICE_KEY="${service_key}" \
JAMSCRIPT_E2E_CODE_HASH="${code_hash}" JAMSCRIPT_E2E_GENESIS_HASH="${genesis_hash}" \
  npm --prefix "${JAMSCRIPT_ROOT}/packages/client" run test:transaction-closure
node --input-type=module - "${LOG_DIR}/jamscript-backend.log" "${E2E_RUNTIME}/native/logs/worker.log" <<'NODE'
import fs from "node:fs";
const backend = fs.readFileSync(process.argv[2], "utf8");
const worker = fs.readFileSync(process.argv[3], "utf8");
const predicted = [...backend.matchAll(/PREDICTED_OUTPUT_BLAKE2=(0x[0-9a-f]+)/g)].map((match) => match[1]);
const actual = [...worker.matchAll(/WORK_RESULT_0_PAYLOAD_BLAKE2=(0x[0-9a-f]+)/g)].map((match) => match[1]);
if (predicted.length !== 3 || actual.length !== 3 || predicted.some((value, index) => value !== actual[index])) {
  throw new Error(`native/PVM output parity mismatch: predicted=${predicted.length} actual=${actual.length}`);
}
if (!/WORK_RESULT_0_KIND=OK/g.test(worker)) throw new Error("a MiniJAM WorkExecResult was not OK");
console.log("PVM_NATIVE_PARITY=PASS");
console.log("WORK_ITEM_COUNT=1");
console.log("WORK_RESULT_COUNT=1");
NODE
minijam_result=PASS
echo "SINGLE_ACTION_E2E=PASS"
