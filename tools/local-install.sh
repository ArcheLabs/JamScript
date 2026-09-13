#!/usr/bin/env bash
set -euo pipefail

# M0 is deliberately a Linux x86_64 producer flow. The resulting CLI and
# managed bundle are consumed independently of this checkout.
ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)"
LOCAL_BIN="${JAMSCRIPT_LOCAL_BIN_DIR:-${HOME:?HOME is required}/.local/bin}"
BUILD_ROOT="$(mktemp -d "${TMPDIR:-/tmp}/jamscript-local-install.XXXXXX")"
trap 'rm -rf -- "${BUILD_ROOT}"' EXIT

fail() {
  printf 'LOCAL_INSTALL=FAIL: %s\n' "$*" >&2
  exit 1
}

[[ "$(uname -s)" == Linux && "$(uname -m)" == x86_64 ]] || \
  fail 'M0 local installation currently supports Linux x86_64 only'

for tool in cargo rustup python3 tar install mv; do
  command -v "${tool}" >/dev/null 2>&1 || \
    fail "required producer tool is missing: ${tool}"
done

NVM_SCRIPT="${NVM_DIR:-${HOME}/.nvm}/nvm.sh"
if [[ -s "${NVM_SCRIPT}" ]]; then
  # shellcheck disable=SC1090
  source "${NVM_SCRIPT}"
  nvm use 24.15.0 >/dev/null
fi
NODE_BIN="${SCRIPTC_NODE:-$(command -v node || true)}"
[[ -n "${NODE_BIN}" && -x "${NODE_BIN}" ]] || \
  fail 'Node 24.15.0 is required; install it with NVM or set SCRIPTC_NODE'
[[ "$(${NODE_BIN} --version | tr -d '\r\n')" == v24.15.0 ]] || \
  fail "expected Node v24.15.0, got $(${NODE_BIN} --version 2>/dev/null || echo unavailable)"

RUST_TOOLCHAIN='nightly-2026-05-02'
RUSTC_BIN="$(rustup which rustc --toolchain "${RUST_TOOLCHAIN}")"
CARGO_BIN="$(rustup which cargo --toolchain "${RUST_TOOLCHAIN}")"

CC_BIN="${JAMSCRIPT_LOCAL_CC:-/usr/bin/clang-20}"
CXX_BIN="${JAMSCRIPT_LOCAL_CXX:-/usr/bin/clang++-20}"
CLANG_BIN="${JAMSCRIPT_CLANG:-/usr/lib/llvm-20/bin/clang}"
LLVM_AR_BIN="${JAMSCRIPT_LLVM_AR:-/usr/lib/llvm-20/bin/llvm-ar}"
LLD_BIN="${JAMSCRIPT_LLVM_LD:-/usr/lib/llvm-20/bin/ld.lld}"
LLVM_ROOT="${JAMSCRIPT_LLVM_ROOT:-$(cd -- "$(dirname -- "${CLANG_BIN}")/.." && pwd -P)}"
for path in "${CC_BIN}" "${CXX_BIN}" "${CLANG_BIN}" "${LLVM_AR_BIN}" "${LLD_BIN}" \
  "${LLVM_ROOT}/bin/llvm-readelf"; do
  [[ -x "${path}" ]] || fail "required LLVM tool is missing: ${path}"
done

export CC="${CC_BIN}"
export CXX="${CXX_BIN}"
CARGO_TARGET_DIR="${BUILD_ROOT}/cargo-target"

printf 'Building release CLI and backend...\n'
(cd "${ROOT}" && \
  CARGO_TARGET_DIR="${CARGO_TARGET_DIR}" \
  cargo +"${RUST_TOOLCHAIN}" build --release --locked \
    --bin jams --bin jamscript-service-backend)

CLI_BINARY="${CARGO_TARGET_DIR}/release/jams"
BACKEND_BINARY="${CARGO_TARGET_DIR}/release/jamscript-service-backend"
[[ -x "${CLI_BINARY}" ]] || fail "release CLI was not built"
[[ -x "${BACKEND_BINARY}" ]] || fail "release backend was not built"

printf 'Building managed toolchain bundle...\n'
TOOLCHAIN_OUT="${BUILD_ROOT}/toolchain"
(
  cd "${ROOT}"
  SCRIPTC_NODE="${NODE_BIN}" \
  JAMSCRIPT_DEV_TOOLCHAIN=1 \
  JAMSCRIPT_CLANG="${CLANG_BIN}" \
  JAMSCRIPT_LLVM_AR="${LLVM_AR_BIN}" \
  JAMSCRIPT_LLVM_LD="${LLD_BIN}" \
  JAMSCRIPT_LLVM_ROOT="${LLVM_ROOT}" \
  JAMSCRIPT_RUSTC="${RUSTC_BIN}" \
  JAMSCRIPT_CARGO="${CARGO_BIN}" \
  tools/release/toolchain/build-linux.sh "${TOOLCHAIN_OUT}"
)
TOOLCHAIN_ARCHIVE="${TOOLCHAIN_OUT}/jamscript-toolchain-scriptc-m2-v1-linux-x86_64.tar.zst"
[[ -f "${TOOLCHAIN_ARCHIVE}" ]] || fail "managed toolchain bundle was not built"

mkdir -p "${LOCAL_BIN}"
LOCAL_BIN="$(cd "${LOCAL_BIN}" && pwd -P)"
NEW_CLI="${LOCAL_BIN}/.jams.new.$$"
NEW_BACKEND="${LOCAL_BIN}/.jamscript-service-backend.new.$$"
cleanup_install() {
  rm -f -- "${NEW_CLI}" "${NEW_BACKEND}"
}
trap 'cleanup_install; rm -rf -- "${BUILD_ROOT}"' EXIT
install -m 0755 "${CLI_BINARY}" "${NEW_CLI}"
install -m 0755 "${BACKEND_BINARY}" "${NEW_BACKEND}"
mv -f -- "${NEW_CLI}" "${LOCAL_BIN}/jams"
mv -f -- "${NEW_BACKEND}" "${LOCAL_BIN}/jamscript-service-backend"

printf 'Installing and verifying managed toolchain...\n'
unset JAMSCRIPT_DEV_TOOLCHAIN JAMSCRIPT_TOOLCHAIN_RELEASE_ENGINEERING \
  JAMSCRIPT_TOOLCHAIN_BUNDLE JAMSCRIPT_RELEASE_TEST
PATH="${LOCAL_BIN}:${PATH}" "${LOCAL_BIN}/jams" toolchain install --archive "${TOOLCHAIN_ARCHIVE}"
PATH="${LOCAL_BIN}:${PATH}" "${LOCAL_BIN}/jams" toolchain verify

printf '\nJamScript local installation complete.\n\nCLI:\n  %s\nBackend:\n  %s\nManaged toolchain:\n  installed and verified\n\nTry:\n  jams --help\n  jams new hello\n  jams build\n' \
  "${LOCAL_BIN}/jams" "${LOCAL_BIN}/jamscript-service-backend"
if [[ "$(command -v jams 2>/dev/null || true)" != "${LOCAL_BIN}/jams" ]]; then
  printf '\nFor this shell:\n  export PATH="%s:$PATH"\n' "${LOCAL_BIN}"
fi
printf 'LOCAL_INSTALL=PASS\n'
