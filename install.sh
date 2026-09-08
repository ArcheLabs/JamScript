#!/usr/bin/env bash
set -euo pipefail

readonly INSTALLER_REPOSITORY="ArcheLabs/JamScript"

usage() {
  cat <<'USAGE'
Usage: install.sh --version VERSION [--bin-dir DIR]

Install the JamScript CLI and its managed toolchain.

Options:
  --version VERSION  Immutable JamScript release tag (required)
  --bin-dir DIR      CLI destination (default: $HOME/.local/bin)
  --help             Show this help
USAGE
}

fail() {
  printf 'INSTALLATION FAILED: %s\n' "$*" >&2
  exit 1
}

version=''
if [[ -z "${HOME:-}" ]]; then
  fail 'HOME is not set; cannot determine the default install directory'
fi
bin_dir="${HOME}/.local/bin"

while (($# > 0)); do
  case "$1" in
    --version)
      (($# >= 2)) || fail '--version requires a value'
      version="$2"
      shift 2
      ;;
    --bin-dir)
      (($# >= 2)) || fail '--bin-dir requires a value'
      bin_dir="$2"
      shift 2
      ;;
    --help|-h)
      usage
      exit 0
      ;;
    *)
      fail "unknown argument: $1"
      ;;
  esac
done

[[ -n "$version" ]] || fail '--version is required'
[[ "$version" =~ ^v[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?$ ]] || \
  fail "invalid release version: $version"
[[ "$version" != *..* ]] || fail "invalid release version: $version"

os="$(uname -s)"
arch="$(uname -m)"
case "${os}:${arch}" in
  Linux:x86_64) platform='linux-x86_64' ;;
  Darwin:arm64) platform='macos-arm64' ;;
  *) fail "unsupported platform: ${os} ${arch}; supported platforms are Linux x86_64 and macOS Apple Silicon (arm64)" ;;
esac

required_tools=(bash curl tar gzip mktemp mkdir install mv rm uname grep awk)
missing_tools=()
for tool in "${required_tools[@]}"; do
  if ! command -v "$tool" >/dev/null 2>&1; then
    missing_tools+=("$tool")
  fi
done
if ((${#missing_tools[@]} > 0)); then
  printf 'Missing required bootstrap tool: %s\n' "${missing_tools[*]}" >&2
  printf 'Install %s using your operating system package manager, then rerun this command.\n' \
    "${missing_tools[*]}" >&2
  exit 1
fi
if ! command -v sha256sum >/dev/null 2>&1 && ! command -v shasum >/dev/null 2>&1; then
  fail 'neither sha256sum nor shasum is available for checksum verification'
fi

test_asset_dir="${JAMSCRIPT_INSTALL_TEST_ASSET_DIR:-}"
if [[ -n "$test_asset_dir" && "${JAMSCRIPT_INSTALL_TEST:-}" != '1' ]]; then
  fail 'JAMSCRIPT_INSTALL_TEST_ASSET_DIR is only available with JAMSCRIPT_INSTALL_TEST=1'
fi

asset="jamscript-${version}-${platform}.tar.gz"
release_base="https://github.com/${INSTALLER_REPOSITORY}/releases/download/${version}"
tmp="$(mktemp -d)"
new_path=''
cleanup() {
  rm -rf "$tmp"
  if [[ -n "$new_path" ]]; then
    rm -f "$new_path"
  fi
}
trap cleanup EXIT

archive_path="${tmp}/${asset}"
checksums_path="${tmp}/SHA256SUMS"
if [[ -n "$test_asset_dir" ]]; then
  test -f "${test_asset_dir}/${asset}" || fail "test asset is missing: ${asset}"
  test -f "${test_asset_dir}/SHA256SUMS" || fail 'test asset is missing: SHA256SUMS'
  install -m 0644 "${test_asset_dir}/${asset}" "$archive_path"
  install -m 0644 "${test_asset_dir}/SHA256SUMS" "$checksums_path"
else
  curl --fail --location --retry 3 --silent --show-error \
    --output "$archive_path" "${release_base}/${asset}"
  curl --fail --location --retry 3 --silent --show-error \
    --output "$checksums_path" "${release_base}/SHA256SUMS"
fi

checksum="$(awk -v target="$asset" '
  NF == 2 && length($1) == 64 && $1 ~ /^[[:xdigit:]]+$/ {
    name = $2
    sub(/^\*/, "", name)
    if (name == target) {
      count++
      value = $1
    }
  }
  END {
    if (count != 1) {
      exit 1
    }
    print value
  }
' "$checksums_path")" || fail "SHA256SUMS must contain exactly one valid entry for ${asset}"

printf '%s  %s\n' "$checksum" "$asset" > "${tmp}/checksum-entry"
sha256_file() {
  local path="$1"
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$path" | awk '{print $1}'
  elif command -v shasum >/dev/null 2>&1; then
    shasum -a 256 "$path" | awk '{print $1}'
  else
    fail 'neither sha256sum nor shasum is available for checksum verification'
  fi
}
actual_checksum="$(sha256_file "$archive_path")"
[[ "$actual_checksum" == "$checksum" ]] || fail "SHA-256 mismatch for ${asset}"

extract="${tmp}/extract"
mkdir -p "$extract"
tar -xzf "$archive_path" -C "$extract"
test -x "$extract/jams" || fail 'release archive does not contain an executable jams'
test -f "$extract/LICENSE" || fail 'release archive does not contain LICENSE'
test -f "$extract/README.md" || fail 'release archive does not contain README.md'
test ! -e "$extract/jamscript" || fail 'legacy jamscript executable unexpectedly present'

mkdir -p "$bin_dir"
bin_dir="$(cd "$bin_dir" && pwd -P)"
installed_jams="${bin_dir}/jams"
new_path="${bin_dir}/.jams.new.$$"
install -m 0755 "$extract/jams" "$new_path"
mv -f "$new_path" "$installed_jams"
new_path=''

printf 'JamScript installer\nRelease:  %s\nPlatform: %s\n\n' "$version" "$platform"
printf 'CLI verified and installed at %s\n\n' "$installed_jams"
printf 'Installing managed toolchain...\n'
if ! "$installed_jams" toolchain install; then
  printf '\nJamScript CLI was installed at %s, but managed toolchain installation failed.\n' \
    "$installed_jams" >&2
  printf 'Installation is incomplete.\n\nRetry:\n  %s toolchain install\n  %s doctor\n' \
    "$installed_jams" "$installed_jams" >&2
  exit 1
fi

printf 'Running doctor...\n'
if ! "$installed_jams" doctor; then
  printf '\nJamScript CLI was installed at %s, but doctor failed.\n' "$installed_jams" >&2
  printf 'Installation is incomplete.\n\nRetry:\n  %s toolchain install\n  %s doctor\n' \
    "$installed_jams" "$installed_jams" >&2
  exit 1
fi

printf '\nJamScript installation complete.\n\nCLI:\n  %s\n\nRelease:\n  %s\n\nPlatform:\n  %s\n\nManaged toolchain:\n  verified\n\nCanonical build readiness:\n  PASS\n' \
  "$installed_jams" "$version" "$platform"

resolved_jams="$(command -v jams 2>/dev/null || true)"
if [[ "$resolved_jams" != "$installed_jams" ]]; then
  printf '\nJamScript is installed, but %s is not currently on PATH.\n\n' "$bin_dir"
  printf 'For this shell:\n  export PATH="%s:$PATH"\n\n' "$bin_dir"
  printf 'Add the same line to your shell profile if needed.\n'
fi

printf '\nTry:\n  jams --help\n  jams new hello\n'
