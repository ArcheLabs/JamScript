#!/usr/bin/env bash
set -euo pipefail

readonly INSTALLER_REPOSITORY="ArcheLabs/JamScript"
readonly RELEASES_API="https://api.github.com/repos/${INSTALLER_REPOSITORY}/releases?per_page=50"

usage() {
  cat <<'USAGE'
Usage: install.sh [--version VERSION] [--bin-dir DIR]

Install the JamScript CLI, matching backend, and managed toolchain.

Options:
  --version VERSION  Install a specific JamScript release tag (default: latest published release)
  --bin-dir DIR      Binary destination (default: $HOME/.local/bin)
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
test_backend_asset_dir="${JAMSCRIPT_INSTALL_TEST_BACKEND_ASSET_DIR:-}"
test_latest_version="${JAMSCRIPT_INSTALL_TEST_LATEST_VERSION:-}"
if [[ -n "$test_asset_dir" && "${JAMSCRIPT_INSTALL_TEST:-}" != '1' ]]; then
  fail 'JAMSCRIPT_INSTALL_TEST_ASSET_DIR is only available with JAMSCRIPT_INSTALL_TEST=1'
fi
if [[ -n "$test_backend_asset_dir" && "${JAMSCRIPT_INSTALL_TEST:-}" != '1' ]]; then
  fail 'JAMSCRIPT_INSTALL_TEST_BACKEND_ASSET_DIR is only available with JAMSCRIPT_INSTALL_TEST=1'
fi
if [[ -n "$test_latest_version" && "${JAMSCRIPT_INSTALL_TEST:-}" != '1' ]]; then
  fail 'JAMSCRIPT_INSTALL_TEST_LATEST_VERSION is only available with JAMSCRIPT_INSTALL_TEST=1'
fi

resolve_latest_version() {
  if [[ -n "$test_latest_version" ]]; then
    printf '%s\n' "$test_latest_version"
    return
  fi

  local releases
  releases="$(curl --fail --location --retry 3 --silent --show-error \
    -H 'Accept: application/vnd.github+json' \
    -H 'X-GitHub-Api-Version: 2022-11-28' \
    "$RELEASES_API")" || fail 'could not query JamScript releases'

  printf '%s\n' "$releases" |
    grep -Eo '"tag_name"[[:space:]]*:[[:space:]]*"v[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?"' |
    awk -F'"' 'NR == 1 { value = $4 } END { print value }'
}

if [[ -z "$version" ]]; then
  version="$(resolve_latest_version)"
  [[ -n "$version" ]] || fail 'could not determine the latest JamScript release'
fi

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

cli_asset="jamscript-${version}-${platform}.tar.gz"
backend_tag="backend-${version}"
backend_asset="jamscript-backend-${version}-${platform}.tar.gz"
release_base="https://github.com/${INSTALLER_REPOSITORY}/releases/download/${version}"
backend_release_base="https://github.com/${INSTALLER_REPOSITORY}/releases/download/${backend_tag}"

tmp="$(mktemp -d)"
new_jams_path=''
new_backend_path=''
cleanup() {
  rm -rf "$tmp"
  [[ -z "$new_jams_path" ]] || rm -f "$new_jams_path"
  [[ -z "$new_backend_path" ]] || rm -f "$new_backend_path"
}
trap cleanup EXIT

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

verify_checksum() {
  local archive_path="$1"
  local checksums_path="$2"
  local asset="$3"
  local checksum actual_checksum

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

  actual_checksum="$(sha256_file "$archive_path")"
  [[ "$actual_checksum" == "$checksum" ]] || fail "SHA-256 mismatch for ${asset}"
}

cli_archive_path="${tmp}/${cli_asset}"
cli_checksums_path="${tmp}/cli-SHA256SUMS"
backend_archive_path="${tmp}/${backend_asset}"
backend_checksums_path="${tmp}/backend-SHA256SUMS"

if [[ -n "$test_asset_dir" ]]; then
  test -f "${test_asset_dir}/${cli_asset}" || fail "test asset is missing: ${cli_asset}"
  test -f "${test_asset_dir}/SHA256SUMS" || fail 'test asset is missing: SHA256SUMS'
  install -m 0644 "${test_asset_dir}/${cli_asset}" "$cli_archive_path"
  install -m 0644 "${test_asset_dir}/SHA256SUMS" "$cli_checksums_path"
else
  curl --fail --location --retry 3 --silent --show-error \
    --output "$cli_archive_path" "${release_base}/${cli_asset}"
  curl --fail --location --retry 3 --silent --show-error \
    --output "$cli_checksums_path" "${release_base}/SHA256SUMS"
fi
verify_checksum "$cli_archive_path" "$cli_checksums_path" "$cli_asset"

if [[ -n "$test_backend_asset_dir" ]]; then
  test -f "${test_backend_asset_dir}/${backend_asset}" || fail "test backend asset is missing: ${backend_asset}"
  test -f "${test_backend_asset_dir}/SHA256SUMS" || fail 'test backend asset is missing: SHA256SUMS'
  install -m 0644 "${test_backend_asset_dir}/${backend_asset}" "$backend_archive_path"
  install -m 0644 "${test_backend_asset_dir}/SHA256SUMS" "$backend_checksums_path"
elif [[ "${JAMSCRIPT_INSTALL_TEST:-}" == '1' ]]; then
  : # Installer fixture tests may intentionally exercise only the CLI path.
else
  curl --fail --location --retry 3 --silent --show-error \
    --output "$backend_archive_path" "${backend_release_base}/${backend_asset}"
  curl --fail --location --retry 3 --silent --show-error \
    --output "$backend_checksums_path" "${backend_release_base}/SHA256SUMS"
fi

if [[ -f "$backend_archive_path" ]]; then
  verify_checksum "$backend_archive_path" "$backend_checksums_path" "$backend_asset"
fi

cli_extract="${tmp}/cli-extract"
mkdir -p "$cli_extract"
tar -xzf "$cli_archive_path" -C "$cli_extract"
test -x "$cli_extract/jams" || fail 'release archive does not contain an executable jams'
test -f "$cli_extract/LICENSE" || fail 'release archive does not contain LICENSE'
test -f "$cli_extract/README.md" || fail 'release archive does not contain README.md'
test ! -e "$cli_extract/jamscript" || fail 'legacy jamscript executable unexpectedly present'
test ! -e "$cli_extract/jamscript-service-backend" || fail 'backend executable must not be in the JamScript CLI archive'

backend_binary=''
if [[ -f "$backend_archive_path" ]]; then
  backend_extract="${tmp}/backend-extract"
  mkdir -p "$backend_extract"
  tar -xzf "$backend_archive_path" -C "$backend_extract"
  backend_binary="${backend_extract}/bin/jamscript-service-backend"
  test -x "$backend_binary" || fail 'backend release archive does not contain bin/jamscript-service-backend'
fi

mkdir -p "$bin_dir"
bin_dir="$(cd "$bin_dir" && pwd -P)"
installed_jams="${bin_dir}/jams"
installed_backend="${bin_dir}/jamscript-service-backend"

new_jams_path="${bin_dir}/.jams.new.$$"
install -m 0755 "$cli_extract/jams" "$new_jams_path"

if [[ -n "$backend_binary" ]]; then
  new_backend_path="${bin_dir}/.jamscript-service-backend.new.$$"
  install -m 0755 "$backend_binary" "$new_backend_path"
fi

mv -f "$new_jams_path" "$installed_jams"
new_jams_path=''

if [[ -n "$backend_binary" ]]; then
  mv -f "$new_backend_path" "$installed_backend"
  new_backend_path=''
fi

printf 'JamScript installer\nRelease:  %s\nPlatform: %s\n\n' "$version" "$platform"
printf 'CLI verified and installed at %s\n' "$installed_jams"
if [[ -n "$backend_binary" ]]; then
  printf 'Backend verified and installed at %s\n' "$installed_backend"
fi

printf '\nInstalling managed toolchain...\n'
if ! "$installed_jams" toolchain install; then
  printf '\nJamScript CLI was installed at %s, but managed toolchain installation failed.\n' \
    "$installed_jams" >&2
  printf 'Installation is incomplete.\n\nRetry:\n  %s toolchain install\n  %s toolchain verify\n' \
    "$installed_jams" "$installed_jams" >&2
  exit 1
fi

printf '\nJamScript installation complete.\n\nCLI:\n  %s\nRelease:\n  %s\nPlatform:\n  %s\n\nManaged toolchain:\n  installed and verified\n' \
  "$installed_jams" "$version" "$platform"
if [[ -n "$backend_binary" ]]; then
  printf 'Backend:\n  %s\n' "$installed_backend"
fi

resolved_jams="$(command -v jams 2>/dev/null || true)"
if [[ "$resolved_jams" != "$installed_jams" ]]; then
  printf '\nJamScript is nstalled, but %s is not currently on PATH.\n\n' "$bin_dir"
  printf 'For this shell:\n  export PATH="%s:$PATH"\n\n' "$bin_dir"
  printf 'Add the same line to your shell profile if needed.\n'
fi

printf '\nTry:\n  jams --help\n  jams new hello\n  jams build\n'
