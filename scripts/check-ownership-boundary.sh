#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)"
core_paths=(
  "${ROOT}/crates/jamscript-crypto"
  "${ROOT}/crates/service-runtime-guest"
  "${ROOT}/crates/jamscript-backend-scriptc"
  "${ROOT}/crates/jamscript-parser"
  "${ROOT}/toolchains/scriptc/m2"
)
if rg -n -i 'matrix|cross.?signing|MatrixControlClaim' "${core_paths[@]}"; then
  echo "CORE_PROVIDER_SPECIFIC_MATCHES=FAIL" >&2
  exit 1
fi
rg -q 'verify_ed25519' "${ROOT}/crates/jamscript-crypto/src/lib.rs"
rg -q 'jamscript_verify_ed25519' "${ROOT}/crates/service-runtime-guest/src/lib.rs"
rg -q 'verifyEd25519' "${ROOT}/toolchains/scriptc/m2/runtime.ts"
if rg -n 'verifyMatrixCrossSigning|jamscript_verify_matrix_cross_signing|RELEASE_MATRIX_' \
  "${ROOT}/.github/workflows/release.yml" "${ROOT}/.github/workflows/backend-release.yml"; then
  echo "RELEASE_MATRIX_SPECIFIC_GATE_PRESENT=true" >&2
  exit 1
fi
echo "CORE_PROVIDER_SPECIFIC_MATCHES=0"
echo "GENERIC_ED25519_CORE_API=PASS"
echo "RELEASE_MATRIX_SPECIFIC_GATE_PRESENT=false"
