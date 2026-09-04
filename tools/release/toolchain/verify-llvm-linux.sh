#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../../.." && pwd -P)"
LLVM_ROOT="${1:?usage: verify-llvm-linux.sh LLVM_ROOT [LOCK]}"
LOCK="${2:-${ROOT}/toolchains/llvm/linux-x86_64.lock}"
LOCK_PARSER="${ROOT}/tools/release/toolchain/llvm-lock.py"

test -d "${LLVM_ROOT}"
test -x "${LLVM_ROOT}/bin/clang"
test -x "${LLVM_ROOT}/bin/llvm-ar"
test -x "${LLVM_ROOT}/bin/ld.lld"

llvm_version="$(python3 "${LOCK_PARSER}" "${LOCK}" --get llvm_version)"
distribution="$(python3 "${LOCK_PARSER}" "${LOCK}" --get distribution)"
archive_sha256="$(python3 "${LOCK_PARSER}" "${LOCK}" --get archive_sha256)"
for tool in clang llvm-ar ld.lld; do
  case "${tool}" in
    clang) expected="$(python3 "${LOCK_PARSER}" "${LOCK}" --get clang_sha256)" ;;
    llvm-ar) expected="$(python3 "${LOCK_PARSER}" "${LOCK}" --get llvm_ar_sha256)" ;;
    ld.lld) expected="$(python3 "${LOCK_PARSER}" "${LOCK}" --get ld_lld_sha256)" ;;
  esac
  actual="$(sha256sum "${LLVM_ROOT}/bin/${tool}" | awk '{print $1}')"
  case "${tool}" in
    clang) label=LLVM_CLANG_SHA256 ;;
    llvm-ar) label=LLVM_LLVM_AR_SHA256 ;;
    ld.lld) label=LLVM_LD_LLD_SHA256 ;;
  esac
  test "${actual}" = "${expected}" || {
    echo "${label}=FAIL expected=${expected} actual=${actual}" >&2
    exit 1
  }
  echo "${label}=PASS ${actual}"
done

clang_line="$(LD_LIBRARY_PATH="${LLVM_ROOT}/lib${LD_LIBRARY_PATH:+:${LD_LIBRARY_PATH}}" "${LLVM_ROOT}/bin/clang" --version | sed -n '1p')"
echo "${clang_line}"
clang_actual="$(sed -n 's/.*clang version \([0-9][0-9.]*\).*/\1/p' <<<"${clang_line}")"
test "${clang_actual}" = "${llvm_version}" || {
  echo "LLVM_VERSION=FAIL expected=${llvm_version} actual=${clang_actual}" >&2
  exit 1
}
echo "LLVM_VERSION=PASS ${llvm_version}"
echo "LLVM_DISTRIBUTION=${distribution}"
echo "LLVM_ARCHIVE_SHA256=${archive_sha256}"

for tool in clang llvm-ar ld.lld; do
  ldd_output="$(LD_LIBRARY_PATH="${LLVM_ROOT}/lib${LD_LIBRARY_PATH:+:${LD_LIBRARY_PATH}}" ldd "${LLVM_ROOT}/bin/${tool}" 2>&1)" || {
    echo "LLVM_RUNTIME_CLOSURE=FAIL ${tool}" >&2
    echo "${ldd_output}" >&2
    exit 1
  }
  if grep -q 'not found' <<<"${ldd_output}"; then
    echo "LLVM_RUNTIME_CLOSURE=FAIL ${tool}: unresolved dependency" >&2
    echo "${ldd_output}" >&2
    exit 1
  fi
done
echo "LLVM_RUNTIME_CLOSURE=PASS"
echo "LLVM_IDENTITY=PASS"
