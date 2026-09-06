#!/usr/bin/env bash
set -euo pipefail

# JamScript Release Kill Test 001. Bootstrap may use curl; after the release
# bytes are present, compiler work runs with a restricted PATH and offline.

usage() {
  cat >&2 <<'EOF'
usage: release-kill-test-001.sh --release-url URL --release-version TAG [options]
       release-kill-test-001.sh --asset-dir DIR --release-version TAG [options]

options:
  --target TRIPLE       default: linux-x86_64
  --fixture-dir DIR     use an external fixture instead of the built-in one
  --result-json PATH    default: release-kill-test-001.json
  --work-dir DIR        preserve the isolated test directory
EOF
  exit 2
}

release_url=""
asset_dir=""
release_version=""
target="linux-x86_64"
fixture_dir=""
result_json="release-kill-test-001.json"
work_dir=""

while (($#)); do
  case "$1" in
    --release-url) release_url="${2:?missing URL}"; shift 2 ;;
    --asset-dir) asset_dir="${2:?missing asset directory}"; shift 2 ;;
    --release-version) release_version="${2:?missing release version}"; shift 2 ;;
    --target) target="${2:?missing target}"; shift 2 ;;
    --fixture-dir) fixture_dir="${2:?missing fixture directory}"; shift 2 ;;
    --result-json) result_json="${2:?missing result path}"; shift 2 ;;
    --work-dir) work_dir="${2:?missing work directory}"; shift 2 ;;
    -h|--help) usage ;;
    *) echo "unknown argument: $1" >&2; usage ;;
  esac
done

[[ -n "${release_version}" ]] || { echo "--release-version is required" >&2; usage; }
[[ "${release_version}" =~ ^v0\.1\.0(-rc\.[0-9]+)?$ ]] || { echo "release version is not a v0.1 semver tag" >&2; exit 1; }
[[ -n "${release_url}" || -n "${asset_dir}" ]] || { echo "--release-url or --asset-dir is required" >&2; usage; }
[[ -z "${release_url}" || -z "${asset_dir}" ]] || { echo "choose one of --release-url and --asset-dir" >&2; usage; }
[[ "${target}" == "linux-x86_64" ]] || { echo "Release Kill Test 001 currently supports linux-x86_64" >&2; exit 1; }
command -v jq >/dev/null 2>&1 || { echo "jq is required during release bootstrap" >&2; exit 1; }

if [[ -z "${work_dir}" ]]; then
  work_dir="$(mktemp -d "${TMPDIR:-/tmp}/jamscript-release-kill-001.XXXXXX")"
else
  mkdir -p "${work_dir}"
fi
bootstrap="${work_dir}/bootstrap"
install="${work_dir}/install"
home="${work_dir}/home"
forbidden_cargo_home="${work_dir}/forbidden-cargo-home"
forbidden_rustup_home="${work_dir}/forbidden-rustup-home"
cache="${work_dir}/cache"
output_a="${work_dir}/output-a"
output_b="${work_dir}/output-b"
mkdir -p "${bootstrap}" "${install}" "${home}" "${forbidden_cargo_home}" "${forbidden_rustup_home}" "${cache}"

cli_asset="jamscript-${release_version}-${target}.tar.zst"
toolchain_asset="jamscript-toolchain-scriptc-m2-v1-${target}.tar.zst"
if [[ -n "${release_url}" ]]; then
  release_url="${release_url%/}"
  curl --fail --location --retry 3 --silent --show-error "${release_url}/release-manifest.json" -o "${bootstrap}/release-manifest.json"
  curl --fail --location --retry 3 --silent --show-error "${release_url}/${cli_asset}" -o "${bootstrap}/${cli_asset}"
  curl --fail --location --retry 3 --silent --show-error "${release_url}/${toolchain_asset}" -o "${bootstrap}/${toolchain_asset}"
  curl --fail --location --retry 3 --silent --show-error "${release_url}/SHA256SUMS" -o "${bootstrap}/SHA256SUMS"
else
  cp -L "${asset_dir}/release-manifest.json" "${bootstrap}/release-manifest.json"
  cp -L "${asset_dir}/${cli_asset}" "${bootstrap}/${cli_asset}"
  cp -L "${asset_dir}/${toolchain_asset}" "${bootstrap}/${toolchain_asset}"
  cp -L "${asset_dir}/SHA256SUMS" "${bootstrap}/SHA256SUMS"
fi
echo "K0_BOOTSTRAP=PASS"

(cd "${bootstrap}" && sha256sum -c SHA256SUMS)
test "$(jq -er '.releaseVersion' "${bootstrap}/release-manifest.json")" = "${release_version}"
test "$(jq -er '.targets[0].triple' "${bootstrap}/release-manifest.json")" = "${target}"
test "$(jq -er '.targets[0].cli.name' "${bootstrap}/release-manifest.json")" = "${cli_asset}"
test "$(jq -er '.targets[0].toolchain.name' "${bootstrap}/release-manifest.json")" = "${toolchain_asset}"
manifest_cli_sha="$(jq -er '.targets[0].cli.sha256' "${bootstrap}/release-manifest.json")"
manifest_toolchain_sha="$(jq -er '.targets[0].toolchain.sha256' "${bootstrap}/release-manifest.json")"
test "$(sha256sum "${bootstrap}/${cli_asset}" | awk '{print $1}')" = "${manifest_cli_sha}"
test "$(sha256sum "${bootstrap}/${toolchain_asset}" | awk '{print $1}')" = "${manifest_toolchain_sha}"
echo "K1_MANIFEST=PASS"
echo "K2_CLI_CHECKSUM=PASS"
echo "K4_TOOLCHAIN_CHECKSUM=PASS"

tar --zstd -xf "${bootstrap}/${cli_asset}" -C "${install}"
test -x "${install}/jamscript"
echo "K3_CLI_DOWNLOAD=PASS"
tar --zstd -tf "${bootstrap}/${toolchain_asset}" >/dev/null
echo "K5_TOOLCHAIN_DOWNLOAD=PASS"

if [[ -z "${fixture_dir}" ]]; then
  fixture_dir="${work_dir}/fixture"
  mkdir -p "${fixture_dir}/.jamscript"
  cat >"${fixture_dir}/hello.ts" <<'EOF'
import { action, publicAction, u64 } from "jam";
export const hello = action({
  auth: publicAction(),
  input: { value: u64 },
  execute(_ctx, input) { return input.value + 1; },
});
EOF
  cat >"${fixture_dir}/jamscript.toml" <<'EOF'
[package]
name = "release-consumer-hello"
version = "0.1.0"
entry = "hello.ts"
language = "0.2"

[compiler]
backend = "scriptc"

[management]
mode = "immutable"
EOF
  cat >"${fixture_dir}/.jamscript/service.json" <<'EOF'
{
  "version": 2,
  "serviceKey": "0x4444444444444444444444444444444444444444444444444444444444444444",
  "instanceId": "0x5555555555555555555555555555555555555555555555555555555555555555",
  "name": "release-consumer-hello"
}
EOF
  printf '%s\n' 'PVM_EXECUTION=PASS' >"${fixture_dir}/expected-output.txt"
fi
test -f "${fixture_dir}/jamscript.toml"
test -f "${fixture_dir}/hello.ts"
expected_output="${fixture_dir}/expected-output.txt"
if [[ ! -f "${expected_output}" ]]; then
  expected_output="${work_dir}/expected-output.txt"
  printf '%s\n' 'PVM_EXECUTION=PASS' >"${expected_output}"
fi

host_tools="${work_dir}/host-tools"
mkdir -p "${host_tools}"
for tool in awk bash basename cat cp dirname find grep mkdir mktemp readelf rm sed sha256sum stat tar tee tr touch zstd; do
  tool_path="$(type -P "${tool}" || true)"
  test -x "${tool_path}"
  ln -s "${tool_path}" "${host_tools}/${tool}"
done
export PATH="${host_tools}"
for forbidden in rustc cargo rustup node npm clang llvm-config zig; do
  if command -v "${forbidden}" >/dev/null 2>&1; then
    echo "forbidden host tool is visible: ${forbidden}" >&2
    exit 1
  fi
done
echo "K6_INSTALL=PASS"

export HOME="${home}"
export CARGO_HOME="${forbidden_cargo_home}"
export RUSTUP_HOME="${forbidden_rustup_home}"
export XDG_CACHE_HOME="${cache}"
export JAMSCRIPT_TOOLCHAIN_HOME="${home}/jamscript-toolchains"
export JAMSCRIPT_TOOLCHAIN_BUNDLE="${bootstrap}/${toolchain_asset}"
export JAMSCRIPT_RELEASE_TEST=1
export JAMSCRIPT_OFFLINE=1
unset JAMSCRIPT_DEV_TOOLCHAIN JAMSCRIPT_TOOLCHAIN_RELEASE_ENGINEERING
unset JAMSCRIPT_MINIJAM_SDK JAMSCRIPT_CLANG JAMSCRIPT_LLVM_ROOT JAMSCRIPT_LLVM_AR JAMSCRIPT_LLVM_LD

"${install}/jamscript" toolchain install
doctor_json="${work_dir}/doctor.json"
"${install}/jamscript" doctor --json >"${doctor_json}"
grep -q '"canonical_build_readiness": "PASS"' "${doctor_json}"
grep -q '"host_dependency_leakage": "PASS"' "${doctor_json}"
grep -q "${JAMSCRIPT_TOOLCHAIN_HOME}" "${doctor_json}"
echo "K7_DOCTOR=PASS"
echo "K8_MANAGED_PATHS=PASS"

"${install}/jamscript" build "${fixture_dir}" --offline --output "${output_a}"
"${install}/jamscript" build "${fixture_dir}" --offline --output "${output_b}"
for output in "${output_a}" "${output_b}"; do
  test -s "${output}/service.pvm"
  test -s "${output}/service.polkavm"
  test -s "${output}/service.blob"
  grep -q '"canonical_toolchain": true' "${output}/build.json"
done
echo "K9_OFFLINE=PASS"
echo "K10_BUILD=PASS"
echo "K11_PVM_ARTIFACT=PASS"

run_result="${work_dir}/pvm-result.bin"
run_result_b="${work_dir}/pvm-result-b.bin"
run_log="${work_dir}/pvm-run.log"
run_log_b="${work_dir}/pvm-run-b.log"
"${install}/jamscript" run "${output_a}/service.pvm" --result "${run_result}" >"${run_log}"
grep -q '^PVM_EXECUTION=PASS$' "${run_log}"
test -s "${run_result}"
"${install}/jamscript" run "${output_a}/service.pvm" --result "${run_result_b}" >"${run_log_b}"
grep -q '^PVM_EXECUTION=PASS$' "${run_log_b}"
test -s "${run_result_b}"
cmp -s "${run_result}" "${run_result_b}"
echo "K12_PVM_EXECUTION=PASS"
expected_line="$(sed -n '1p' "${expected_output}")"
test -n "${expected_line}"
grep -Fxq "${expected_line}" "${run_log}"
echo "K13_OUTPUT_AND_RESULT=PASS"

hash_a="$(sha256sum "${output_a}/service.pvm" | awk '{print $1}')"
hash_b="$(sha256sum "${output_b}/service.pvm" | awk '{print $1}')"
test "${hash_a}" = "${hash_b}"
echo "K14_REBUILD=PASS"
echo "K15_DETERMINISTIC_HASH=PASS"

cat >"${result_json}" <<EOF
{
  "protocol": "JAMSCRIPT_RELEASE_KILL_001",
  "status": "PASS",
  "release": "${release_version}",
  "target": "${target}",
  "managed_paths_only": true,
  "offline_build": true,
  "execution": true,
  "output_match": true,
  "gates": {
    "R1_clean_consumer_e2e": true,
    "R2_managed_toolchain_published": $([[ -n "${release_url}" ]] && echo true || echo false),
    "R3_immutable_github_release": $([[ -n "${release_url}" ]] && echo true || echo false),
    "R4_published_artifact_validation": $([[ -n "${release_url}" ]] && echo true || echo false)
  },
  "artifact_sha256": "${hash_a}"
}
EOF
echo "RELEASE_KILL_TEST_001=PASS"
