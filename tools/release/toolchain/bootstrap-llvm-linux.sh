#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../../.." && pwd -P)"
OUT="${1:?usage: bootstrap-llvm-linux.sh OUT [LOCK]}"
LOCK="${2:-${ROOT}/toolchains/llvm/linux-x86_64.lock}"
PARSER="${ROOT}/tools/release/toolchain/llvm-lock.py"

test "$(uname -m)" = "x86_64"
if [[ -e "${OUT}" ]]; then
  echo "LLVM bootstrap output already exists: ${OUT}" >&2
  exit 1
fi
archive_filename="$(python3 "${PARSER}" "${LOCK}" --get archive_filename)"
archive_url="$(python3 "${PARSER}" "${LOCK}" --get archive_url)"
archive_sha256="$(python3 "${PARSER}" "${LOCK}" --get archive_sha256)"
mkdir -p "${OUT}/download" "${OUT}/unpack"
archive="${OUT}/download/${archive_filename}"
curl --fail --location --retry 5 --retry-delay 2 --silent --show-error \
  --output "${archive}" "${archive_url}"
actual_archive_sha256="$(sha256sum "${archive}" | awk '{print $1}')"
test "${actual_archive_sha256}" = "${archive_sha256}" || {
  echo "LLVM_ARCHIVE_INTEGRITY=FAIL expected=${archive_sha256} actual=${actual_archive_sha256}" >&2
  exit 1
}
echo "LLVM_ARCHIVE_INTEGRITY=PASS ${actual_archive_sha256}"

# Reject absolute/traversal members before extraction. Links are allowed inside
# this disposable extraction root and are resolved by the identity verifier.
while IFS= read -r member; do
  member="${member%/}"
  [[ -n "${member}" && "${member}" != /* && "${member}" != *\\* && "${member}" != ../* && "${member}" != */../* && "${member}" != .. ]] || {
    echo "unsafe LLVM archive member: ${member}" >&2
    exit 1
  }
done < <(tar -tJf "${archive}")
tar -xJf "${archive}" -C "${OUT}/unpack"

mapfile -t roots < <(find "${OUT}/unpack" -mindepth 1 -maxdepth 1 -type d -print)
test "${#roots[@]}" -eq 1
LLVM_ROOT="${roots[0]}"
mv -- "${LLVM_ROOT}" "${OUT}/llvm"
LLVM_ROOT="${OUT}/llvm"
rmdir -- "${OUT}/unpack"

"${ROOT}/tools/release/toolchain/verify-llvm-linux.sh" "${LLVM_ROOT}" "${LOCK}"

cat > "${OUT}/llvm.env" <<EOF
LLVM_ROOT=${LLVM_ROOT}
JAMSCRIPT_LLVM_ROOT=${LLVM_ROOT}
JAMSCRIPT_CLANG=${LLVM_ROOT}/bin/clang
JAMSCRIPT_LLVM_AR=${LLVM_ROOT}/bin/llvm-ar
JAMSCRIPT_LLVM_LD=${LLVM_ROOT}/bin/ld.lld
LLVM_VERSION=$(python3 "${PARSER}" "${LOCK}" --get llvm_version)
LLVM_DISTRIBUTION=$(python3 "${PARSER}" "${LOCK}" --get distribution)
LLVM_ARCHIVE_SHA256=${archive_sha256}
EOF
echo "LLVM_BOOTSTRAP=PASS"
echo "LLVM_ROOT=${LLVM_ROOT}"
echo "JAMSCRIPT_CLANG=${LLVM_ROOT}/bin/clang"
echo "JAMSCRIPT_LLVM_AR=${LLVM_ROOT}/bin/llvm-ar"
echo "JAMSCRIPT_LLVM_LD=${LLVM_ROOT}/bin/ld.lld"
