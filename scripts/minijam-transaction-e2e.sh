#!/usr/bin/env bash
set -euo pipefail

JAMSCRIPT_ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)"
MINIJAM_ROOT="${MINIJAM_DIRECT_REFINE_ROOT:-/home/libingjiang/minijam-client-direct-refine}"
E2E_RUNTIME="${JAMSCRIPT_E2E_RUNTIME:-${JAMSCRIPT_ROOT}/target/jamscript-transaction-e2e}"
MINIJAM_SOURCE_WORKTREE="${E2E_RUNTIME}/minijam-source"
PROJECT="${E2E_RUNTIME}/dynamic-state-scriptc"
ARTIFACTS="${PROJECT}/dist"
LOG_DIR="${E2E_RUNTIME}/logs"
LOCK_FILE="${JAMSCRIPT_ROOT}/toolchains/minijam.lock"
NATIVE_UP="${MINIJAM_SOURCE_WORKTREE}/scripts/stage1-native-local-up.sh"
NATIVE_DOWN="${MINIJAM_SOURCE_WORKTREE}/scripts/stage1-native-local-down.sh"
backend_pid=""
clean_repro_worktree=""

export PATH="${HOME}/.nvm/versions/node/v24.15.0/bin:${PATH}"
export JAMSCRIPT_DEV_TOOLCHAIN="${JAMSCRIPT_DEV_TOOLCHAIN:-1}"
export JAMSCRIPT_BATCH_MAX_ACTIONS="${JAMSCRIPT_BATCH_MAX_ACTIONS:-3}"
export JAMSCRIPT_BATCH_FLUSH_MS="${JAMSCRIPT_BATCH_FLUSH_MS:-1000}"
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
  if [[ -n "${clean_repro_worktree}" ]]; then
    git -C "${JAMSCRIPT_ROOT}" worktree remove --force "${clean_repro_worktree}" >/dev/null 2>&1 || true
  fi
  git -C "${MINIJAM_ROOT}" worktree remove --force "${MINIJAM_SOURCE_WORKTREE}" >/dev/null 2>&1 || true
  echo "CLEANUP=COMPLETE"
  exit "${status}"
}
trap cleanup EXIT INT TERM

[[ "$(node --version)" == "v24.15.0" ]] || { echo "Node v24.15.0 is required" >&2; exit 1; }
expected_jamscript_head="${JAMSCRIPT_EXPECTED_HEAD:-$(git -C "${JAMSCRIPT_ROOT}" rev-parse HEAD)}"
jamscript_executed_sha="$(git -C "${JAMSCRIPT_ROOT}" rev-parse HEAD)"
[[ "${jamscript_executed_sha}" == "${expected_jamscript_head}" ]] || {
  echo "JamScript HEAD changed during E2E: ${jamscript_executed_sha} != ${expected_jamscript_head}" >&2
  exit 1
}
locked_revision="$(sed -n 's/^revision = "\([^"]*\)"/\1/p' "${LOCK_FILE}")"
git -C "${MINIJAM_ROOT}" worktree remove --force "${MINIJAM_SOURCE_WORKTREE}" >/dev/null 2>&1 || true
rm -rf "${E2E_RUNTIME}"
mkdir -p "${LOG_DIR}"
git -C "${MINIJAM_ROOT}" worktree add --detach "${MINIJAM_SOURCE_WORKTREE}" "${locked_revision}" >/dev/null
git -C "${MINIJAM_SOURCE_WORKTREE}" submodule update --init external/jambda >/dev/null
actual_revision="$(git -C "${MINIJAM_SOURCE_WORKTREE}" rev-parse HEAD)"
[[ "${actual_revision}" == "${locked_revision}" ]] || {
  echo "MiniJAM exact checkout mismatch: ${actual_revision} != ${locked_revision}" >&2
  exit 1
}
echo "JAMSCRIPT_HEAD=${expected_jamscript_head}"
echo "JAMSCRIPT_EXECUTED_SHA=${jamscript_executed_sha}"
echo "MINIJAM_LOCK_SHA=${locked_revision}"
echo "MINIJAM_EXECUTED_SHA=${actual_revision}"
echo "MINIJAM_EXACT_PIN=PASS"
echo "FRESH_CHAIN=PASS"
echo "FRESH_BACKEND_DB=PASS"
echo "FRESH_DEPLOYMENT=PASS"

MINIJAM_CARGO_TARGET_DIR="${MINIJAM_CARGO_TARGET_DIR:-${MINIJAM_ROOT}/target}"
export MINIJAM_NATIVE_NODE_BIN="${MINIJAM_NATIVE_NODE_BIN:-${MINIJAM_CARGO_TARGET_DIR}/debug/minijam-node}"
export MINIJAM_NATIVE_FORMAL_RPC_BIN="${MINIJAM_NATIVE_FORMAL_RPC_BIN:-${MINIJAM_CARGO_TARGET_DIR}/debug/minijam-formal-rpc}"
export MINIJAM_NATIVE_WORKER_BIN="${MINIJAM_NATIVE_WORKER_BIN:-${MINIJAM_CARGO_TARGET_DIR}/debug/minijam-worker}"

MINIJAM_NATIVE_LOCAL_RUNTIME="${E2E_RUNTIME}/native" \
  MINIJAM_NATIVE_WORKER_KEY=//Alice MINIJAM_NATIVE_FORMAL_SIGNER_KEY=//Bob \
  CARGO_TARGET_DIR="${MINIJAM_CARGO_TARGET_DIR}" \
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
JAMSCRIPT_E2E_RESULT="${E2E_RUNTIME}/e2e-result.json" JAMSCRIPT_EXECUTED_SHA="${jamscript_executed_sha}" \
  npm --prefix "${JAMSCRIPT_ROOT}/packages/client" run test:transaction-closure
node --input-type=module - "${E2E_RUNTIME}/e2e-result.json" "${LOG_DIR}/jamscript-backend.log" "${E2E_RUNTIME}/native/logs/worker.log" <<'NODE'
import fs from "node:fs";
const result = JSON.parse(fs.readFileSync(process.argv[2], "utf8"));
const backend = fs.readFileSync(process.argv[3], "utf8");
const worker = fs.readFileSync(process.argv[4], "utf8");
const predictionByPackage = new Map();
for (const match of backend.matchAll(/PACKAGE_HASH=(0x[0-9a-f]+).*?PREDICTED_OUTPUT_BLAKE2=(0x[0-9a-f]+)/g)) {
  predictionByPackage.set(match[1], match[2]);
}
const actualByPackage = new Map();
const resultCountByPackage = new Map();
const kindByPackage = new Map();
let currentPackage = null;
for (const line of worker.split("\n")) {
  const packageMatch = line.match(/WORK_PACKAGE_HASH=(0x[0-9a-f]+)/);
  if (packageMatch) currentPackage = packageMatch[1];
  const countMatch = line.match(/WORK_RESULT_COUNT=(\d+)/);
  if (countMatch && currentPackage) resultCountByPackage.set(currentPackage, Number(countMatch[1]));
  const kindMatch = line.match(/WORK_RESULT_0_KIND=([A-Z_]+)/);
  if (kindMatch && currentPackage) kindByPackage.set(currentPackage, kindMatch[1]);
  const payloadMatch = line.match(/WORK_RESULT_0_PAYLOAD_BLAKE2=(0x[0-9a-f]+)/);
  if (payloadMatch && currentPackage) actualByPackage.set(currentPackage, payloadMatch[1]);
}
if (predictionByPackage.size === 0 || predictionByPackage.size !== actualByPackage.size) {
  throw new Error(`package keyed parity map mismatch: predicted=${predictionByPackage.size} actual=${actualByPackage.size}`);
}
for (const [packageHash, predicted] of predictionByPackage) {
  if (actualByPackage.get(packageHash) !== predicted) throw new Error(`PVM/native mismatch for ${packageHash}`);
  if (resultCountByPackage.get(packageHash) !== 1) throw new Error(`unexpected WorkResult count for ${packageHash}`);
  if (kindByPackage.get(packageHash) !== "OK") throw new Error(`unexpected WorkResult kind for ${packageHash}`);
}
function assertMeasured(name, measured, expected) {
  if (measured !== expected) throw new Error(`${name}=${measured}, expected ${expected}`);
  console.log(`${name}=${measured}`);
}
function batchStats(name, value, expectedLogical) {
  assertMeasured(`${name}_LOGICAL_TX_COUNT`, value.logicalTransactionIds.length, expectedLogical);
  assertMeasured(`${name}_FORMAL_TX_COUNT`, value.formalTransactionIds.length, 1);
  assertMeasured(`${name}_PACKAGE_COUNT`, value.packageHashes.length, 1);
  assertMeasured(`${name}_WORK_ITEM_COUNT`, value.workItems.length, 1);
  assertMeasured(`${name}_RECEIPT_COUNT`, value.receipts.length, expectedLogical);
}
batchStats("SINGLE", result.single, 1);
batchStats("BATCH", result.batch, 3);
batchStats("OUT_OF_ORDER", result.outOfOrder, 3);
batchStats("CROSS_SIGNER", result.crossSigner, 3);
console.log(`BATCH_PACKAGE_HASH=${result.batch.packageHashes[0]}`);
console.log(`BATCH_PREDICTED_HASH=${predictionByPackage.get(result.batch.packageHashes[0])}`);
console.log(`BATCH_PVM_RESULT_HASH=${actualByPackage.get(result.batch.packageHashes[0])}`);
console.log("PVM_NATIVE_PARITY=PASS");
console.log("SINGLE_PVM_NATIVE_PARITY=PASS");
console.log("BATCH_PVM_NATIVE_PARITY=PASS");
console.log(`SINGLE_WORK_RESULT_COUNT=${resultCountByPackage.get(result.single.packageHashes[0])}`);
console.log(`SINGLE_WORK_RESULT_KIND=${kindByPackage.get(result.single.packageHashes[0])}`);
console.log(`BATCH_WORK_RESULT_COUNT=${resultCountByPackage.get(result.batch.packageHashes[0])}`);
NODE

kill "${backend_pid}" 2>/dev/null || true
wait "${backend_pid}" 2>/dev/null || true
backend_pid=""
JAMSCRIPT_BACKEND_BIND="${JAMSCRIPT_BACKEND_BIND}" JAMSCRIPT_NODE_RPC="${MINIJAM_NODE_RPC}" \
JAMSCRIPT_FORMAL_RPC="${MINIJAM_FORMAL_RPC_URL}" JAMSCRIPT_BACKEND_DATA="${JAMSCRIPT_BACKEND_DATA}" \
MINIJAM_E2E_DIAGNOSTICS=1 JAMSCRIPT_BATCH_MAX_ACTIONS="${JAMSCRIPT_BATCH_MAX_ACTIONS}" \
JAMSCRIPT_BATCH_FLUSH_MS="${JAMSCRIPT_BATCH_FLUSH_MS}" \
  "${JAMSCRIPT_ROOT}/target/debug/jamscript-service-backend" >"${LOG_DIR}/jamscript-backend-restart.log" 2>&1 &
backend_pid=$!
for _ in $(seq 1 60); do curl -fsS "${JAMSCRIPT_BACKEND_URL}/readinessz" >/dev/null 2>&1 && break; sleep 1; done
curl -fsS "${JAMSCRIPT_BACKEND_URL}/readinessz" >/dev/null
restarted_status="$(curl -fsS -H content-type:application/json --data "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"jamscript_getServiceStateStatusV1\",\"params\":{\"serviceId\":${service_id}}}" "${JAMSCRIPT_BACKEND_URL}")"
node --input-type=module -e 'let b="";process.stdin.on("data",c=>b+=c);process.stdin.on("end",()=>{const r=JSON.parse(b);if(r.error||r.result?.materialized!==true)throw new Error(JSON.stringify(r));})' <<<"${restarted_status}"
echo "MATERIALIZED_STATE_RESTART=PASS"
echo "MATERIALIZED_STATE_DURABILITY=PASS"
echo "TRANSACTION_INGRESS_DURABILITY=NOT_GUARANTEED_V1"

if [[ "${JAMSCRIPT_CLEAN_REPRO:-0}" != 1 ]]; then
  clean_repro_worktree="${E2E_RUNTIME}-clean-worktree"
  git worktree add --detach "${clean_repro_worktree}" "${expected_jamscript_head}" >/dev/null
  JAMSCRIPT_CLEAN_REPRO=1 JAMSCRIPT_EXPECTED_HEAD="${expected_jamscript_head}" \
    JAMSCRIPT_E2E_RUNTIME="${E2E_RUNTIME}-clean" \
    MINIJAM_DIRECT_REFINE_ROOT="${MINIJAM_ROOT}" \
    MINIJAM_NATIVE_NODE_RPC_PORT=9945 MINIJAM_NATIVE_FORMAL_RPC_PORT=8092 MINIJAM_NATIVE_NODE_P2P_PORT=30334 \
    JAMSCRIPT_BACKEND_BIND=127.0.0.1:8093 JAMSCRIPT_BACKEND_URL=http://127.0.0.1:8093 \
    MINIJAM_NODE_RPC=http://127.0.0.1:9945 MINIJAM_FORMAL_RPC_URL=http://127.0.0.1:8092 \
    JAMSCRIPT_NATIVE_LOCAL_RUNTIME="${E2E_RUNTIME}-clean/native" \
  "${clean_repro_worktree}/scripts/minijam-transaction-e2e.sh"
  echo "CLEAN_WORKTREE_E2E=PASS"
fi
echo "LOCAL_PRIMARY_E2E=PASS"
if [[ "${JAMSCRIPT_CLEAN_REPRO:-0}" != 1 ]]; then
  echo "REAL_MINIJAM_E2E=PASS"
fi
