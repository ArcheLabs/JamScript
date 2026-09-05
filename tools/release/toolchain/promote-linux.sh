#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../../.." && pwd -P)"
ARCHIVE="${1:?usage: promote-linux.sh bundle.tar.zst release-tag}"
TAG="${2:?usage: promote-linux.sh bundle.tar.zst release-tag}"
MANIFEST="${ROOT}/toolchains/distribution-v1.toml"
ASSET="jamscript-toolchain-scriptc-m2-v1-linux-x86_64.tar.zst"
test "$(basename -- "${ARCHIVE}")" = "${ASSET}"
"${ROOT}/tools/release/toolchain/verify-bundle.sh" "${ARCHIVE}"
sha="$(sha256sum "${ARCHIVE}" | awk '{print $1}')"
size="$(stat -c '%s' "${ARCHIVE}")"
url="https://github.com/ArcheLabs/JamScript/releases/download/${TAG}/${ASSET}"
python3 - "${MANIFEST}" "${url}" "${sha}" "${size}" <<'PY'
import pathlib
import sys

path, url, sha, size = sys.argv[1:]
text = pathlib.Path(path).read_text()
text = text.replace('url = "https://github.com/ArcheLabs/JamScript/releases/download/toolchain-scriptc-m2-v1/jamscript-toolchain-scriptc-m2-v1-linux-x86_64.tar.zst"', f'url = "{url}"')
text = text.replace('sha256 = "0000000000000000000000000000000000000000000000000000000000000000"', f'sha256 = "{sha}"')
text = text.replace('size = 1', f'size = {size}')
text = text.replace('published = false', 'published = true')
pathlib.Path(path).write_text(text)
PY
echo "Promoted ${url}"
echo "sha256=${sha}"
echo "size=${size}"
