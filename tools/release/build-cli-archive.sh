#!/usr/bin/env bash
set -euo pipefail

SOURCE_ROOT="${1:?usage: build-cli-archive.sh source-root output-dir release-version [platform]}"
OUT="${2:?usage: build-cli-archive.sh source-root output-dir release-version [platform]}"
VERSION="${3:?usage: build-cli-archive.sh source-root output-dir release-version [platform]}"
PLATFORM="${4:-linux-x86_64}"
SOURCE_DATE_EPOCH="${SOURCE_DATE_EPOCH:-0}"

case "${PLATFORM}" in
  linux-x86_64) archive="jamscript-${VERSION}-linux-x86_64.tar.zst" ;;
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
find "${stage}" -type f -exec touch -d "@${SOURCE_DATE_EPOCH}" {} +
(cd "${stage}" && tar --sort=name --numeric-owner --owner=0 --group=0 --mtime="@${SOURCE_DATE_EPOCH}" --zstd -cf "${OUT}/${archive}" .)
archive_entries="$(tar --zstd -tf "${OUT}/${archive}")"
printf '%s\n' "${archive_entries}" | sed 's#^\./##' | grep -qx 'jams'
if printf '%s\n' "${archive_entries}" | sed 's#^\./##' | grep -qx 'jamscript'; then
  echo "legacy jamscript executable unexpectedly present" >&2
  exit 1
fi
sha256sum "${OUT}/${archive}"
echo "CLI_ARCHIVE=${OUT}/${archive}"
