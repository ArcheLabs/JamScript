#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../../.." && pwd -P)"
LLVM_ROOT="${1:?usage: verify-llvm-macos.sh llvm-root [lock-file]}"
LOCK="${2:-${ROOT}/toolchains/llvm/macos-arm64.lock}"
PARSER="${ROOT}/tools/release/toolchain/llvm-lock.py"

test "$(uname -s)" = "Darwin" || { echo "macOS LLVM verification requires Darwin" >&2; exit 1; }
test "$(uname -m)" = "arm64" || { echo "macOS LLVM verification requires native arm64" >&2; exit 1; }
command -v file >/dev/null 2>&1 || { echo "file is required to verify Apple Silicon binaries" >&2; exit 1; }

sha256_file() {
  if command -v shasum >/dev/null 2>&1; then
    shasum -a 256 "$1" | awk '{print $1}'
  else
    sha256sum "$1" | awk '{print $1}'
  fi
}
get_lock() { python3 "${PARSER}" "${LOCK}" --get "$1"; }

test -d "${LLVM_ROOT}" || { echo "LLVM root is missing: ${LLVM_ROOT}" >&2; exit 1; }
clang="${LLVM_ROOT}/$(get_lock clang_relpath)"
llvm_ar="${LLVM_ROOT}/$(get_lock llvm_ar_relpath)"
lld="${LLVM_ROOT}/$(get_lock lld_relpath)"
readelf="${LLVM_ROOT}/bin/llvm-readelf"
for tool in "${clang}" "${llvm_ar}" "${lld}" "${readelf}"; do
  test -x "${tool}" || { echo "locked LLVM tool is missing: ${tool}" >&2; exit 1; }
  description="$(file -b "${tool}")"
  grep -Eiq 'Mach-O.*(arm64|arm64e)' <<<"${description}" || {
    echo "LLVM binary is not native arm64: ${tool} (${description})" >&2
    exit 1
  }
done

test "$(${clang} --version | sed -n '1s/.*clang version \([0-9.]*\).*/\1/p')" = "$(get_lock llvm_version)"
for pair in "clang:${clang}" "llvm_ar:${llvm_ar}" "ld_lld:${lld}"; do
  name="${pair%%:*}"
  path="${pair#*:}"
  actual="$(sha256_file "${path}")"
  expected="$(get_lock "${name}_sha256")"
  if [[ "${expected}" == "$(printf '0%.0s' {1..64})" ]]; then
    echo "${name}_SHA256=${actual} (measured; macOS lock promotion pending)"
  else
    test "${actual}" = "${expected}" || { echo "${name} SHA-256 mismatch" >&2; exit 1; }
    echo "${name}_SHA256=${actual}"
  fi
done
echo "LLVM_MACOS_ARM64_VERIFICATION=PASS"
