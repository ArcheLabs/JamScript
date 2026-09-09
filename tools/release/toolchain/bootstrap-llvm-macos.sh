#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../../.." && pwd -P)"
OUT="${1:?usage: bootstrap-llvm-macos.sh output-directory [lock-file]}"
LOCK="${2:-${ROOT}/toolchains/llvm/macos-arm64.lock}"
PARSER="${ROOT}/tools/release/toolchain/llvm-lock.py"

test "$(uname -s)" = "Darwin" || { echo "macOS LLVM bootstrap requires Darwin" >&2; exit 1; }
test "$(uname -m)" = "arm64" || { echo "macOS LLVM bootstrap requires a native arm64 runner" >&2; exit 1; }
command -v curl >/dev/null 2>&1 || { echo "curl is required to bootstrap LLVM" >&2; exit 1; }
command -v tar >/dev/null 2>&1 || { echo "tar is required to bootstrap LLVM" >&2; exit 1; }
command -v file >/dev/null 2>&1 || { echo "file is required to verify Apple Silicon binaries" >&2; exit 1; }

sha256_file() {
  if command -v shasum >/dev/null 2>&1; then
    shasum -a 256 "$1" | awk '{print $1}'
  elif command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | awk '{print $1}'
  else
    echo "neither shasum nor sha256sum is available" >&2
    exit 1
  fi
}

get_lock() { python3 "${PARSER}" "${LOCK}" --get "$1"; }
archive_url="$(get_lock archive_url)"
archive_name="$(get_lock archive_filename)"
archive_sha="$(get_lock archive_sha256)"
clang_relpath="$(get_lock clang_relpath)"
llvm_ar_relpath="$(get_lock llvm_ar_relpath)"
lld_relpath="$(get_lock lld_relpath)"

mkdir -p "${OUT}"
download="${OUT}/${archive_name}"
unpack="${OUT}/unpack"
rm -rf -- "${unpack}"
mkdir -p "${unpack}"
if [[ ! -f "${download}" ]]; then
  curl --fail --location --retry 3 --silent --show-error --output "${download}" "${archive_url}"
fi
test "$(sha256_file "${download}")" = "${archive_sha}" || {
  echo "LLVM archive SHA-256 mismatch" >&2
  exit 1
}
tar -xJf "${download}" -C "${unpack}"

if [[ -x "${unpack}/bin/clang" ]]; then
  llvm_root="${unpack}"
else
  roots=("${unpack}"/*)
  test "${#roots[@]}" -eq 1 && test -d "${roots[0]}" && llvm_root="${roots[0]}" || {
    echo "unable to discover the root of the official macOS LLVM archive" >&2
    exit 1
  }
fi

clang="${llvm_root}/${clang_relpath}"
llvm_ar="${llvm_root}/${llvm_ar_relpath}"
lld="${llvm_root}/${lld_relpath}"
readelf="${llvm_root}/bin/llvm-readelf"
for tool in "${clang}" "${llvm_ar}" "${lld}" "${readelf}"; do
  test -x "${tool}" || { echo "locked LLVM tool is missing: ${tool}" >&2; exit 1; }
  description="$(file -b "${tool}")"
  grep -Eiq 'Mach-O.*(arm64|arm64e)' <<<"${description}" || {
    echo "macOS LLVM tool is not an Apple Silicon Mach-O binary: ${tool} (${description})" >&2
    exit 1
  }
done

recorded_hashes=()
clang_measured=""
llvm_ar_measured=""
ld_lld_measured=""
for pair in "clang:${clang}" "llvm_ar:${llvm_ar}" "ld_lld:${lld}"; do
  name="${pair%%:*}"
  path="${pair#*:}"
  actual="$(sha256_file "${path}")"
  expected="$(get_lock "${name}_sha256")"
  case "${name}" in
    clang) clang_measured="${actual}" ;;
    llvm_ar) llvm_ar_measured="${actual}" ;;
    ld_lld) ld_lld_measured="${actual}" ;;
  esac
  if [[ "${expected}" == "$(printf '0%.0s' {1..64})" ]]; then
    recorded_hashes+=("${name}_sha256=${actual}")
  elif [[ "${expected}" != "${actual}" ]]; then
    echo "locked LLVM binary SHA-256 mismatch: ${name}" >&2
    exit 1
  fi
done

cat > "${OUT}/llvm.env" <<EOF
JAMSCRIPT_LLVM_ROOT=${llvm_root}
JAMSCRIPT_CLANG=${clang}
JAMSCRIPT_LLVM_AR=${llvm_ar}
JAMSCRIPT_LLVM_LD=${lld}
LLVM_ROOT=${llvm_root}
LLVM_ARCHIVE=${download}
LLVM_ARCHIVE_SHA256=${archive_sha}
LLVM_CLANG_SHA256_MEASURED=${clang_measured}
LLVM_AR_SHA256_MEASURED=${llvm_ar_measured}
LLVM_LD_LLD_SHA256_MEASURED=${ld_lld_measured}
LLVM_BINARY_HASHES_MEASURED=${recorded_hashes[*]-}
EOF
echo "LLVM_ARCHIVE_IDENTITY=PASS"
echo "LLVM_RUNNER_ARCHITECTURE=PASS"
echo "LLVM_BINARY_IDENTITY=PASS"
echo "LLVM_ROOT=${llvm_root}"
