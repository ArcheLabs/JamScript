#!/usr/bin/env bash
set -euo pipefail

JAMSCRIPT_ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)"
RUN_ROOT="$(mktemp -d "${TMPDIR:-/tmp}/jamscript-matrix-normal-action.XXXXXX")"
trap 'rm -rf -- "${RUN_ROOT}"' EXIT

JAMS="${JAMSCRIPT_TEST_CLI:-${JAMSCRIPT_ROOT}/target/debug/jams}"
[[ -x "${JAMS}" ]] || { echo "build target/debug/jams first" >&2; exit 2; }
[[ "${JAMSCRIPT_DEV_TOOLCHAIN:-}" == "1" ]] || {
  echo "this gate must use the pinned contributor ScriptC/LLVM toolchain" >&2
  exit 2
}

"${JAMS}" build \
  "${JAMSCRIPT_ROOT}/tests/scriptc-matrix-normal-action" \
  --output "${RUN_ROOT}/dist"
test -s "${RUN_ROOT}/dist/service.pvm"

locus_artifact=()
if [[ -n "${LOCUS_ROOT:-}" ]]; then
  LOCUS_ROOT="$(cd -- "${LOCUS_ROOT}" && pwd -P)"
  [[ -f "${LOCUS_ROOT}/scripts/bundle-service.mjs" ]] || {
    echo "LOCUS_ROOT must point to the Locus source checkout" >&2
    exit 2
  }
  node "${LOCUS_ROOT}/scripts/bundle-service.mjs" \
    "${LOCUS_ROOT}" "${RUN_ROOT}/locus-project"
  "${JAMS}" build "${RUN_ROOT}/locus-project" --output "${RUN_ROOT}/locus-dist"
  test -s "${RUN_ROOT}/locus-dist/service.pvm"
  locus_artifact+=("${RUN_ROOT}/locus-dist/service.pvm")
fi

log="${RUN_ROOT}/matrix-normal-action.log"
cargo run --locked --quiet \
  --manifest-path "${JAMSCRIPT_ROOT}/tools/pvm-scriptc-matrix-normal-action/Cargo.toml" \
  -- "${RUN_ROOT}/dist/service.pvm" "${locus_artifact[@]}" | tee "${log}"
markers=( \
  ED25519_DIRECT_PLAN=PASS \
  ED25519_DIRECT_ACTION=APPLIED \
  SIGNED_ACTION_V2_ED25519_PVM=PASS \
  MATRIX_ED25519_DIRECT_OWNERSHIP_PVM=PASS \
  MATRIX_DELEGATED_OWNERSHIP_PVM=PASS \
  MATRIX_MISSING_CONTROLLER=STRUCTURED_ABORT \
  MATRIX_REVOKED_CONTROLLER=STRUCTURED_ABORT \
  MATRIX_UNAUTHORIZED_CONTROLLER=ABORT_5001 \
  MATRIX_REVOKED_CONTROLLER=ABORT_5001 \
  MATRIX_UNAUTHORIZED_FAILURE_STRUCTURED=PASS \
  MATRIX_NORMAL_ACTION_PVM=PASS \
  UNCLASSIFIED_PVM_TRAP=0 )
if ((${#locus_artifact[@]})); then
  markers+=( \
    LOCUS_DIRECT_TRANSFER=PASS \
    MATRIX_TRANSFER_PLAN=PASS \
    MATRIX_TRANSFER_RECEIPT=APPLIED \
    MATRIX_TRANSFER_STATE=PASS \
    MATRIX_TRANSFER=PASS \
    MATRIX_TRANSFER_REAL_PVM=PASS \
    MATRIX_CREATE_POOL_PLAN=PASS \
    MATRIX_CREATE_POOL_RECEIPT=APPLIED \
    MATRIX_CREATE_POOL_STATE=PASS \
    MATRIX_CREATE_POOL_LP_POSITION=PASS \
    MATRIX_CREATE_POOL=PASS \
    MATRIX_SWAP_PLAN=PASS \
    MATRIX_SWAP_RECEIPT=APPLIED \
    MATRIX_SWAP_RESERVES=PASS \
    MATRIX_SWAP_BALANCES=PASS \
    MATRIX_SWAP=PASS )
fi
for marker in "${markers[@]}"; do
  grep -Fxq "${marker}" "${log}"
done
