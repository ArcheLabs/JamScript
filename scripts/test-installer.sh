#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

version='v0.1.0-rc.1'
case "$(uname -s):$(uname -m)" in
  Linux:x86_64) platform='linux-x86_64' ;;
  Darwin:arm64) platform='macos-arm64' ;;
  *) printf 'installer tests require Linux x86_64 or macOS arm64\n' >&2; exit 1 ;;
esac
asset="jamscript-${version}-${platform}.tar.gz"
asset_dir="${tmp}/assets"
fixture_dir="${tmp}/fixture"
home_dir="${tmp}/home"
bin_dir="${tmp}/bin"
log_file="${tmp}/jams.log"
fake_bin="${tmp}/fake-bin"
mkdir -p "$asset_dir" "$fixture_dir" "$home_dir" "$bin_dir"
canonical_bin_dir="$(cd "$bin_dir" && pwd -P)"
test_path="$PATH"

sha256_file() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | awk '{print $1}'
  else
    shasum -a 256 "$1" | awk '{print $1}'
  fi
}

write_fixture_cli() {
  cat > "${fixture_dir}/jams" <<'FIXTURE'
#!/usr/bin/env bash
set -euo pipefail
printf '%s\n' "$*" >> "${JAMSCRIPT_TEST_LOG:?}"
case "$*" in
  'toolchain install')
    printf 'Toolchain verified at test-fixture\n'
    ;;
  'doctor')
    if [[ "${JAMSCRIPT_TEST_DOCTOR_FAIL:-}" == '1' ]]; then
      printf 'fixture doctor failure\n' >&2
      exit 1
    fi
    printf 'Canonical build readiness: PASS\n'
    ;;
esac
FIXTURE
  chmod 0755 "${fixture_dir}/jams"
}

make_archive() {
  local mode="${1:-success}"
  local checksum_mode="${2:-correct}"
  rm -rf "$asset_dir" "$fixture_dir"
  mkdir -p "$asset_dir" "$fixture_dir"
  write_fixture_cli
  printf 'fixture license\n' > "${fixture_dir}/LICENSE"
  printf 'fixture readme\n' > "${fixture_dir}/README.md"
  if [[ "$mode" == 'legacy-jamscript' ]]; then
    printf '#!/usr/bin/env bash\nexit 0\n' > "${fixture_dir}/jamscript"
    chmod 0755 "${fixture_dir}/jamscript"
  fi
  if [[ "$mode" == 'missing-jams' ]]; then
    rm -f "${fixture_dir}/jams"
  fi
  (cd "$fixture_dir" && tar -czf "${asset_dir}/${asset}" .)
  case "$checksum_mode" in
    correct)
      printf '%s  %s\n' "$(sha256_file "${asset_dir}/${asset}")" "$asset" > "${asset_dir}/SHA256SUMS"
      ;;
    missing)
      : > "${asset_dir}/SHA256SUMS"
      ;;
    incorrect)
      printf '%064d  %s\n' 0 "$asset" > "${asset_dir}/SHA256SUMS"
      ;;
    *)
      printf 'unknown checksum test mode: %s\n' "$checksum_mode" >&2
      exit 1
      ;;
  esac
}

run_install() {
  HOME="$home_dir" \
  PATH="$test_path" \
  JAMSCRIPT_INSTALL_TEST=1 \
  JAMSCRIPT_INSTALL_TEST_ASSET_DIR="$asset_dir" \
  JAMSCRIPT_TEST_LOG="$log_file" \
  JAMSCRIPT_TEST_DOCTOR_FAIL="${JAMSCRIPT_TEST_DOCTOR_FAIL:-}" \
  bash "$ROOT/install.sh" --version "$version" --bin-dir "$bin_dir" "$@"
}

run_expect_failure() {
  if "$@" > "${tmp}/failure.out" 2>&1; then
    printf 'expected command to fail: %s\n' "$*" >&2
    exit 1
  fi
}

assert_log_contains() {
  grep -qx "$1" "$log_file"
}

# I1, I10, I11, I12: supported-native success, executable installation, and CLI calls.
make_archive
: > "$log_file"
run_install > "${tmp}/success.out"
test -x "${bin_dir}/jams"
assert_log_contains 'toolchain install'
assert_log_contains 'doctor'
if PATH="$test_path" command -v jams >/dev/null 2>&1; then
  ! grep -Fq "  export PATH=\"${canonical_bin_dir}:\$PATH\"" "${tmp}/success.out"
else
  grep -Fqx "  export PATH=\"${canonical_bin_dir}:\$PATH\"" "${tmp}/success.out"
fi

# The default destination is HOME/.local/bin.
HOME="$home_dir" PATH="$test_path" JAMSCRIPT_INSTALL_TEST=1 JAMSCRIPT_INSTALL_TEST_ASSET_DIR="$asset_dir" \
  JAMSCRIPT_TEST_LOG="$log_file" bash "$ROOT/install.sh" --version "$version" >/dev/null
test -x "${home_dir}/.local/bin/jams"

mkdir -p "$fake_bin"

# I2: unsupported OS.
cat > "${fake_bin}/uname" <<'FAKE'
#!/usr/bin/env bash
if [[ "$1" == '-s' ]]; then printf 'Darwin\n'; else printf 'x86_64\n'; fi
FAKE
chmod 0755 "${fake_bin}/uname"
run_expect_failure env PATH="$fake_bin:$test_path" HOME="$home_dir" \
  JAMSCRIPT_INSTALL_TEST=1 JAMSCRIPT_INSTALL_TEST_ASSET_DIR="$asset_dir" \
  JAMSCRIPT_TEST_LOG="$log_file" bash "$ROOT/install.sh" --version "$version" --bin-dir "$bin_dir"

# I3: unsupported architecture.
cat > "${fake_bin}/uname" <<'FAKE'
#!/usr/bin/env bash
if [[ "$1" == '-s' ]]; then printf 'Linux\n'; else printf 'aarch64\n'; fi
FAKE
chmod 0755 "${fake_bin}/uname"
run_expect_failure env PATH="$fake_bin:$test_path" HOME="$home_dir" \
  JAMSCRIPT_INSTALL_TEST=1 JAMSCRIPT_INSTALL_TEST_ASSET_DIR="$asset_dir" \
  JAMSCRIPT_TEST_LOG="$log_file" bash "$ROOT/install.sh" --version "$version" --bin-dir "$bin_dir"
rm -f "${fake_bin}/uname"

# I4 and I5: required and validated release versions.
run_expect_failure env HOME="$home_dir" bash "$ROOT/install.sh" --bin-dir "$bin_dir"
run_expect_failure env HOME="$home_dir" bash "$ROOT/install.sh" --version 'v0.1.0/rc.1' --bin-dir "$bin_dir"

# I6: checksum entry is required.
make_archive success missing
run_expect_failure run_install

# I7: checksum mismatch is rejected.
make_archive success incorrect
printf 'old CLI\n' > "${bin_dir}/jams"
chmod 0755 "${bin_dir}/jams"
run_expect_failure run_install
grep -qx 'old CLI' "${bin_dir}/jams"

# I8: archive structure requires jams.
make_archive missing-jams
run_expect_failure run_install

# I9: legacy jamscript is forbidden.
make_archive legacy-jamscript
run_expect_failure run_install

# I13: doctor failure leaves the verified CLI in place and fails overall.
make_archive
JAMSCRIPT_TEST_DOCTOR_FAIL=1 run_expect_failure run_install
test -x "${bin_dir}/jams"

# I14: reinstall is idempotent.
make_archive
: > "$log_file"
run_install >/dev/null
run_install >/dev/null
test "$(grep -c '^toolchain install$' "$log_file")" -eq 2
test "$(grep -c '^doctor$' "$log_file")" -eq 2

# I15: a download/source failure preserves the existing CLI.
printf 'stable CLI\n' > "${bin_dir}/jams"
chmod 0755 "${bin_dir}/jams"
rm -f "${asset_dir}/${asset}"
run_expect_failure run_install
grep -qx 'stable CLI' "${bin_dir}/jams"

# I16: custom bin directory.
custom_bin="${tmp}/custom-bin"
make_archive
HOME="$home_dir" PATH="$test_path" JAMSCRIPT_INSTALL_TEST=1 JAMSCRIPT_INSTALL_TEST_ASSET_DIR="$asset_dir" \
  JAMSCRIPT_TEST_LOG="$log_file" bash "$ROOT/install.sh" --version "$version" --bin-dir "$custom_bin" >/dev/null
test -x "${custom_bin}/jams"

printf 'INSTALLER_TESTS=PASS\n'
