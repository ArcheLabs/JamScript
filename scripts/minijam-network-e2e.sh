#!/usr/bin/env bash
set -euo pipefail

JAMSCRIPT_ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)"
MINIJAM_ROOT="${JAMSCRIPT_MINIJAM_SDK:-${JAMSCRIPT_ROOT}/../minijam-client}"
MINIJAM_ROOT="$(cd -- "${MINIJAM_ROOT}" && pwd -P)"
E2E_RUNTIME="${MINIJAM_ROOT}/.local/jamscript-network-e2e"
E2E_PROJECT="${E2E_RUNTIME}/dynamic-state-scriptc"
ARTIFACTS="${E2E_PROJECT}/dist"
LOCK_FILE="${JAMSCRIPT_ROOT}/toolchains/minijam.lock"
minijam_result=FAIL

export JAMSCRIPT_MINIJAM_SDK="${MINIJAM_ROOT}"
export MINIJAM_NATIVE_RUNTIME_ROOT="${E2E_RUNTIME}"
export MINIJAM_FORMAL_RPC_BIND="${MINIJAM_FORMAL_RPC_BIND:-127.0.0.1:8090}"
export MINIJAM_FORMAL_RPC_URL="${MINIJAM_FORMAL_RPC_URL:-http://127.0.0.1:8090}"
export JAMSCRIPT_ADAPTER_BIND="${JAMSCRIPT_ADAPTER_BIND:-127.0.0.1:8091}"
export JAMSCRIPT_ADAPTER_URL="${JAMSCRIPT_ADAPTER_URL:-http://127.0.0.1:8091}"
export MINIJAM_NODE_RPC="${MINIJAM_NODE_RPC:-http://127.0.0.1:9944}"
export MINIJAM_WORK_RPC="${MINIJAM_WORK_RPC:-${JAMSCRIPT_ADAPTER_URL}}"
export MINIJAM_STATE_RPC="${MINIJAM_STATE_RPC:-${JAMSCRIPT_ADAPTER_URL}}"
export MINIJAM_NATIVE_LOCAL_RUNTIME="${E2E_RUNTIME}"

cleanup() {
  local status=$?
  trap - EXIT INT TERM
  if [[ -n "${adapter_pid:-}" ]]; then
    kill "${adapter_pid}" 2>/dev/null || true
    wait "${adapter_pid}" 2>/dev/null || true
  fi
  MINIJAM_LOCAL_PURGE=0 "${MINIJAM_ROOT}/scripts/stage1-native-local-down.sh" || true
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
    rm -rf "${E2E_RUNTIME}"
  fi
  echo "REAL_MINIJAM_E2E=${minijam_result}"
  exit "${status}"
}
trap cleanup EXIT INT TERM

[[ -x "${MINIJAM_ROOT}/scripts/stage1-native-local-up.sh" ]] || {
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

(cd "${JAMSCRIPT_ROOT}" && cargo build --locked --bin jams)
if [[ "${JAMSCRIPT_SKIP_CLIENT_BUILD:-0}" == "1" ]]; then
  test -s "${JAMSCRIPT_ROOT}/packages/client/dist/index.js" || {
    echo "client build is required when JAMSCRIPT_SKIP_CLIENT_BUILD=1" >&2
    exit 1
  }
else
  npm --prefix "${JAMSCRIPT_ROOT}/packages/client" ci --no-audit
  npm --prefix "${JAMSCRIPT_ROOT}/packages/client" run build
fi

echo "[network] starting isolated MiniJAM network"
MINIJAM_LOCAL_PURGE=1 "${MINIJAM_ROOT}/scripts/stage1-native-local-down.sh" >/dev/null 2>&1 || true
"${MINIJAM_ROOT}/scripts/stage1-native-local-up.sh"
curl -fsS "${MINIJAM_FORMAL_RPC_URL}/health/ready" >/dev/null
curl -fsS \
  -H "content-type: application/json" \
  --data '{"jsonrpc":"2.0","id":1,"method":"system_health","params":[]}' \
  "${MINIJAM_NODE_RPC}" >/dev/null
echo "[network] MiniJAM node ready"
echo "[network] Formal transaction RPC ready"
echo "[network] Worker ready"

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

rm -rf "${E2E_PROJECT}"
mkdir -p "${E2E_RUNTIME}"
cp -R "${JAMSCRIPT_ROOT}/examples/dynamic-state-scriptc" "${E2E_PROJECT}"
cat >> "${E2E_PROJECT}/jamscript.toml" <<EOF

[networks.local]
kind = "minijam"
deployment_rpc = "${MINIJAM_FORMAL_RPC_URL}"
node_rpc = "${MINIJAM_NODE_RPC}"
genesis_hash = "${genesis_hash}"
EOF
sed -i \
  -e "s/^genesis_hash = .*/genesis_hash = \"${genesis_hash}\"/" \
  "${E2E_PROJECT}/jamscript.toml"

(cd "${JAMSCRIPT_ROOT}" && cargo run --locked --bin jams -- check "${E2E_PROJECT}")
if [[ "${JAMSCRIPT_SKIP_SERVICE_BUILD:-0}" == "1" ]]; then
  prebuilt="${JAMSCRIPT_PREBUILT_ARTIFACTS:-}"
  [[ -n "${prebuilt}" && -d "${prebuilt}" ]] || {
    echo "JAMSCRIPT_PREBUILT_ARTIFACTS is required when JAMSCRIPT_SKIP_SERVICE_BUILD=1" >&2
    exit 1
  }
  mkdir -p "${ARTIFACTS}"
  cp -a "${prebuilt}/." "${ARTIFACTS}/"
else
  npm --prefix "${JAMSCRIPT_ROOT}/toolchains/scriptc" ci --ignore-scripts --no-audit --no-fund
  (
    cd "${JAMSCRIPT_ROOT}"
    JAMSCRIPT_DEV_TOOLCHAIN=1 \
      cargo run --locked --bin jams -- build "${E2E_PROJECT}" --output "${ARTIFACTS}"
  )
fi
code_hash="$(
  node --input-type=module -e 'let b="";process.stdin.on("data",c=>b+=c);process.stdin.on("end",()=>process.stdout.write(JSON.parse(b).code_hash));' < "${ARTIFACTS}/build.json"
)"
service_key="$(
  node --input-type=module -e 'let b="";process.stdin.on("data",c=>b+=c);process.stdin.on("end",()=>{const v=JSON.parse(b);process.stdout.write(v.serviceKey ?? v.service_key);});' < "${ARTIFACTS}/build.json"
)"
echo "[build] JamScript service built: ${ARTIFACTS}/service.blob"

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

builder_native_sources="$(
  node --input-type=module -e '
    import fs from "node:fs";
    import path from "node:path";
    const [metadataPath, project] = process.argv.slice(1);
    const metadata = JSON.parse(fs.readFileSync(metadataPath, "utf8"));
    process.stdout.write(metadata.native_modules.flatMap(module => module.sources.map(source => path.resolve(project, source))).join(path.delimiter));
  ' "${ARTIFACTS}/builder.json" "${E2E_PROJECT}"
)"
builder_native_includes="$(
  node --input-type=module -e '
    import fs from "node:fs";
    import path from "node:path";
    const [metadataPath, project] = process.argv.slice(1);
    const metadata = JSON.parse(fs.readFileSync(metadataPath, "utf8"));
    process.stdout.write(metadata.native_modules.flatMap(module => module.include_dirs.map(include => path.resolve(project, include))).join(path.delimiter));
  ' "${ARTIFACTS}/builder.json" "${E2E_PROJECT}"
)"
scriptc_runtime_sources="${JAMSCRIPT_ROOT}/toolchains/scriptc/node_modules/@scriptc/runtime/src/scr_library.c:${JAMSCRIPT_ROOT}/toolchains/scriptc/node_modules/@scriptc/runtime/src/scr_number.c:${JAMSCRIPT_ROOT}/toolchains/scriptc/node_modules/@scriptc/runtime/src/scr_string.c:${JAMSCRIPT_ROOT}/toolchains/scriptc/node_modules/@scriptc/runtime/src/scr_array.c:${JAMSCRIPT_ROOT}/toolchains/scriptc/node_modules/@scriptc/runtime/src/scr_bytes.c:${JAMSCRIPT_ROOT}/toolchains/scriptc/node_modules/@scriptc/runtime/src/scr_closure.c:${JAMSCRIPT_ROOT}/toolchains/scriptc/node_modules/@scriptc/runtime/src/scr_cycle.c:${JAMSCRIPT_ROOT}/toolchains/scriptc/node_modules/@scriptc/runtime/src/scr_error.c:${JAMSCRIPT_ROOT}/toolchains/scriptc/node_modules/@scriptc/runtime/src/scr_exception.c:${JAMSCRIPT_ROOT}/toolchains/scriptc/node_modules/@scriptc/runtime/src/scr_json.c:${JAMSCRIPT_ROOT}/toolchains/scriptc/node_modules/@scriptc/runtime/src/scr_object.c:${JAMSCRIPT_ROOT}/toolchains/scriptc/node_modules/@scriptc/runtime/src/scr_union.c:${JAMSCRIPT_ROOT}/crates/jamscript-runtime-scriptc/src/scr_lib_cleanup.c"
builder_native_sources="${builder_native_sources:+${builder_native_sources}:}${ARTIFACTS}/scriptc/scriptc_service.lib.c:${ARTIFACTS}/scriptc/scriptc_service_adapter.c:${scriptc_runtime_sources}"
builder_native_includes="${builder_native_includes:+${builder_native_includes}:}${JAMSCRIPT_SCRIPTC_RUNTIME_INCLUDE:-${JAMSCRIPT_ROOT}/toolchains/scriptc/node_modules/@scriptc/runtime/src}"
JAMSCRIPT_BUILDER_APPLICATION_RS="${ARTIFACTS}/generated_builder_application.rs" \
JAMSCRIPT_BUILDER_NATIVE_SOURCES="${builder_native_sources}" \
JAMSCRIPT_BUILDER_NATIVE_INCLUDES="${builder_native_includes}" \
  cargo build --locked --manifest-path "${JAMSCRIPT_ROOT}/Cargo.toml" --bin managed-state-network-adapter

JAMSCRIPT_E2E_SERVICE_ID="${service_id}" \
JAMSCRIPT_E2E_SERVICE_KEY="${service_key}" \
JAMSCRIPT_E2E_CODE_HASH="${code_hash}" \
JAMSCRIPT_E2E_GENESIS_HASH="${genesis_hash}" \
JAMSCRIPT_E2E_TEST_METHODS=true \
JAMSCRIPT_PROVIDER_STORE="${E2E_PROJECT}/provider-recovery.log" \
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

if [[ "$(node --input-type=module -e 'process.stdout.write(process.platform)')" == "win32" ]] && command -v wslpath >/dev/null 2>&1; then
  client_root_win="$(wslpath -w "${JAMSCRIPT_ROOT}/packages/client")"
  client_artifacts_win="$(wslpath -w "${ARTIFACTS}")"
  client_wsl_env="${WSLENV:-}"
  for client_env_name in \
    JAMSCRIPT_E2E_ARTIFACTS JAMSCRIPT_E2E_SERVICE_ID JAMSCRIPT_E2E_SERVICE_KEY \
    JAMSCRIPT_E2E_CODE_HASH JAMSCRIPT_E2E_GENESIS_HASH JAMSCRIPT_E2E_LOG_DIR \
    MINIJAM_NODE_RPC MINIJAM_WORK_RPC MINIJAM_STATE_RPC; do
    if [[ ":${client_wsl_env}:" != *":${client_env_name}:"* ]]; then
      client_wsl_env="${client_wsl_env:+${client_wsl_env}:}${client_env_name}"
    fi
  done
  WSLENV="${client_wsl_env}" \
  JAMSCRIPT_E2E_ARTIFACTS="${client_artifacts_win}" \
  JAMSCRIPT_E2E_SERVICE_ID="${service_id}" \
  JAMSCRIPT_E2E_SERVICE_KEY="${service_key}" \
  JAMSCRIPT_E2E_CODE_HASH="${code_hash}" \
  JAMSCRIPT_E2E_GENESIS_HASH="${genesis_hash}" \
  JAMSCRIPT_E2E_LOG_DIR="${E2E_RUNTIME}/logs" \
    node "${client_root_win}/tests/minijam-network.e2e.mjs"
else
  JAMSCRIPT_E2E_ARTIFACTS="${ARTIFACTS}" \
  JAMSCRIPT_E2E_SERVICE_ID="${service_id}" \
  JAMSCRIPT_E2E_SERVICE_KEY="${service_key}" \
  JAMSCRIPT_E2E_CODE_HASH="${code_hash}" \
  JAMSCRIPT_E2E_GENESIS_HASH="${genesis_hash}" \
  JAMSCRIPT_E2E_LOG_DIR="${E2E_RUNTIME}/logs" \
    npm --prefix "${JAMSCRIPT_ROOT}/packages/client" run test:network
fi
minijam_result=PASS
