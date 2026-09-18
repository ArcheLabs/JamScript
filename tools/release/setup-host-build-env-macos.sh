#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd -P)"
OUT="${1:?usage: setup-host-build-env-macos.sh OUTPUT_ENV}"
LLVM_OUT="${RUNNER_TEMP:-/tmp}/jamscript-llvm-macos"

for variable in LD_LIBRARY_PATH DYLD_LIBRARY_PATH DYLD_FALLBACK_LIBRARY_PATH; do
  if [[ -n "${!variable:-}" ]]; then
    echo "HOST_BUILD_ENV=FAIL ${variable}_PREEXISTING" >&2
    exit 1
  fi
done

brew install zstd jq ripgrep
rm -rf -- "${LLVM_OUT}"
"${ROOT}/tools/release/toolchain/bootstrap-llvm-macos.sh" "${LLVM_OUT}"
set -a
source "${LLVM_OUT}/llvm.env"
set +a
"${ROOT}/tools/release/toolchain/verify-llvm-macos.sh" "${JAMSCRIPT_LLVM_ROOT}"

LLVM_CONFIG_PATH="${JAMSCRIPT_LLVM_ROOT}/bin/llvm-config"
test -x "${LLVM_CONFIG_PATH}" || {
  echo "HOST_BUILD_ENV=FAIL LLVM_CONFIG_MISSING" >&2
  exit 1
}
echo "LLVM_CONFIG_DISCOVERY=PASS"
LIBCLANG_PATH="$(find "${JAMSCRIPT_LLVM_ROOT}" -type f \( -name 'libclang.dylib' -o -name 'libclang.*.dylib' \) -exec dirname {} \; | sort -u | head -n 1)"
if [[ -z "${LIBCLANG_PATH}" ]]; then
  LIBCLANG_PATH="$(find "${JAMSCRIPT_LLVM_ROOT}" -type f -name 'libclang*.dylib' -exec dirname {} \; | sort -u | head -n 1)"
fi
test -n "${LIBCLANG_PATH}"
find "${LIBCLANG_PATH}" -maxdepth 1 -type f -name 'libclang*.dylib' -print -quit | grep -q .
HOST_RPATH_FLAG="-Clink-arg=-Wl,-rpath,${LIBCLANG_PATH}"
echo "LIBCLANG_DISCOVERY=PASS"

cat > "${OUT}" <<EOF
JAMSCRIPT_LLVM_ROOT=${JAMSCRIPT_LLVM_ROOT}
JAMSCRIPT_CLANG=${JAMSCRIPT_CLANG}
JAMSCRIPT_LLVM_AR=${JAMSCRIPT_LLVM_AR}
JAMSCRIPT_LLVM_LD=${JAMSCRIPT_LLVM_LD}
LLVM_CONFIG_PATH=${LLVM_CONFIG_PATH}
LIBCLANG_PATH=${LIBCLANG_PATH}
JAMSCRIPT_HOST_RPATH_FLAG=${HOST_RPATH_FLAG}
EOF
chmod 0644 "${OUT}"
echo "HOST_BUILD_ENV=PASS"
echo "JAMSCRIPT_CLANG=${JAMSCRIPT_CLANG}"
echo "LIBCLANG_PATH=${LIBCLANG_PATH}"
