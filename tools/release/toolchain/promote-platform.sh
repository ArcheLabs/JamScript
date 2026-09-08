#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../../.." && pwd -P)"
PLATFORM="${1:?usage: promote-platform.sh linux-x86_64|macos-arm64 bundle.tar.zst release-tag}"
ARCHIVE="${2:?usage: promote-platform.sh linux-x86_64|macos-arm64 bundle.tar.zst release-tag}"
TAG="${3:?usage: promote-platform.sh linux-x86_64|macos-arm64 bundle.tar.zst release-tag}"
MANIFEST="${ROOT}/toolchains/distribution-v1.toml"

case "${PLATFORM}" in
  linux-x86_64|macos-arm64) ;;
  *) echo "unsupported promotion platform: ${PLATFORM}" >&2; exit 1 ;;
esac
ASSET="jamscript-toolchain-scriptc-m2-v1-${PLATFORM}.tar.zst"
test "$(basename -- "${ARCHIVE}")" = "${ASSET}"
"${ROOT}/tools/release/toolchain/verify-bundle.sh" "${ARCHIVE}"
if command -v sha256sum >/dev/null 2>&1; then
  sha="$(sha256sum "${ARCHIVE}" | awk '{print $1}')"
else
  sha="$(shasum -a 256 "${ARCHIVE}" | awk '{print $1}')"
fi
if [[ "$(uname -s):$(uname -m)" == "Darwin:arm64" ]]; then
  size="$(stat -f '%z' "${ARCHIVE}")"
else
  size="$(stat -c '%s' "${ARCHIVE}")"
fi
url="https://github.com/ArcheLabs/JamScript/releases/download/${TAG}/${ASSET}"
python3 - "${MANIFEST}" "${PLATFORM}" "${url}" "${sha}" "${size}" <<'PY'
import pathlib
import re
import sys

path, platform, url, sha, size = sys.argv[1:]
manifest = pathlib.Path(path)
text = manifest.read_text(encoding="utf-8")
section_pattern = re.compile(
    rf"(^\[platforms\.{re.escape(platform)}\]\n)(.*?)(?=^\[|\Z)",
    re.MULTILINE | re.DOTALL,
)
match = section_pattern.search(text)
if not match:
    raise SystemExit(f"missing platform section: {platform}")
body = match.group(2)
for name, value in (("url", url), ("sha256", sha)):
    body, count = re.subn(rf'^{name} = ".*"$', f'{name} = "{value}"', body, count=1, flags=re.MULTILINE)
    if count != 1:
        raise SystemExit(f"missing {name} in platform section: {platform}")
body, count = re.subn(r"^size = [0-9]+$", f"size = {size}", body, count=1, flags=re.MULTILINE)
if count != 1:
    raise SystemExit(f"missing size in platform section: {platform}")
body, count = re.subn(r"^published = false$", "published = true", body, count=1, flags=re.MULTILINE)
if count != 1:
    raise SystemExit(f"platform is already promoted or has no unpublished record: {platform}")
manifest.write_text(text[:match.start()] + match.group(1) + body + text[match.end():], encoding="utf-8")
PY
echo "Promoted ${url}"
echo "platform=${PLATFORM}"
echo "sha256=${sha}"
echo "size=${size}"
