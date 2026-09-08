#!/usr/bin/env bash
set -euo pipefail

SOURCE_ROOT="${1:?usage: build-cli-archive.sh source-root output-dir release-version [platform]}"
OUT="${2:?usage: build-cli-archive.sh source-root output-dir release-version [platform]}"
VERSION="${3:?usage: build-cli-archive.sh source-root output-dir release-version [platform]}"
PLATFORM="${4:-linux-x86_64}"
SOURCE_DATE_EPOCH="${SOURCE_DATE_EPOCH:-0}"

case "${PLATFORM}" in
  linux-x86_64|macos-arm64) archive="jamscript-${VERSION}-${PLATFORM}.tar.gz" ;;
  *) echo "unsupported CLI release platform: ${PLATFORM}" >&2; exit 1 ;;
esac

mkdir -p "${OUT}"
target_dir="${OUT}/cargo-target"
stage="${OUT}/cli-stage"
rm -rf "${target_dir}" "${stage}"
mkdir -p "${stage}"
(cd "${SOURCE_ROOT}" && CARGO_TARGET_DIR="${target_dir}" cargo build --release --locked --bin jams)
test -x "${target_dir}/release/jams"
test ! -e "${target_dir}/release/jamscript"
cp -L "${target_dir}/release/jams" "${stage}/jams"
chmod 0755 "${stage}/jams"
cp -L "${SOURCE_ROOT}/LICENSE" "${stage}/LICENSE"
cp -L "${SOURCE_ROOT}/README.md" "${stage}/README.md"
# The CLI bootstrap archive intentionally uses gzip: both stock GNU tar and
# stock BSD tar can unpack it without a third-party zstd dependency.
(cd "${stage}" && tar -czf "${OUT}/${archive}" .)
archive_entries="$(tar -tzf "${OUT}/${archive}")"
printf '%s\n' "${archive_entries}" | sed 's#^\./##' | grep -qx 'jams'
if printf '%s\n' "${archive_entries}" | sed 's#^\./##' | grep -qx 'jamscript'; then
  echo "legacy jamscript executable unexpectedly present" >&2
  exit 1
fi
if command -v sha256sum >/dev/null 2>&1; then
  sha256sum "${OUT}/${archive}"
else
  shasum -a 256 "${OUT}/${archive}"
fi
echo "CLI_ARCHIVE=${OUT}/${archive}"
