#!/usr/bin/env bash
set -euo pipefail

JAMSCRIPT_ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)"
RUN_ROOT="$(mktemp -d "${TMPDIR:-/tmp}/jamscript-matrix-pvm.XXXXXX")"
trap 'rm -rf -- "${RUN_ROOT}"' EXIT

JAMS="${JAMSCRIPT_TEST_CLI:-${JAMSCRIPT_ROOT}/target/release/jams}"
[[ -x "${JAMS}" ]] || JAMS="${JAMSCRIPT_ROOT}/target/debug/jams"
[[ -x "${JAMS}" ]] || { echo "build target/release/jams or target/debug/jams first" >&2; exit 2; }
[[ "${JAMSCRIPT_DEV_TOOLCHAIN:-}" == "1" ]] || {
  echo "this gate must use the contributor ScriptC toolchain" >&2
  exit 2
}

node "${JAMSCRIPT_ROOT}/packages/client/scripts/bundle-matrix-scriptc-probe.mjs" \
  "${RUN_ROOT}/project"
"${JAMS}" build "${RUN_ROOT}/project" --output "${RUN_ROOT}/dist"
test -s "${RUN_ROOT}/dist/service.pvm"

log="${RUN_ROOT}/matrix-pvm.log"
cargo run --locked --quiet \
  --manifest-path "${JAMSCRIPT_ROOT}/tools/pvm-scriptc-ed25519-ffi/Cargo.toml" \
  --bin matrix_adapter -- "${RUN_ROOT}/dist/service.pvm" | tee "${log}"
for marker in \
  MATRIX_ADAPTER_PVM_VALID=PASS \
  MATRIX_ADAPTER_PVM_INVALID_M_TO_S=PASS \
  MATRIX_ADAPTER_PVM_INVALID_S_TO_D=PASS \
  MATRIX_ADAPTER_PVM_WRONG_SUBJECT=PASS \
  MATRIX_ADAPTER_PVM_WRONG_CONTROLLER=PASS \
  MATRIX_ADAPTER_PVM_MALFORMED=PASS \
  MATRIX_ADAPTER_PVM=PASS \
  MATRIX_ADAPTER_PVM_FATAL=false; do
  grep -Fxq "${marker}" "${log}"
done
