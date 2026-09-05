#!/usr/bin/env bash
set -euo pipefail

ARCHIVE="${1:?usage: consumer-smoke.sh bundle.tar.zst jamscript-cli fixture.tar install-home output-dir}"
CLI_BINARY="${2:?usage: consumer-smoke.sh bundle.tar.zst jamscript-cli fixture.tar install-home output-dir}"
FIXTURE_ARCHIVE="${3:?usage: consumer-smoke.sh bundle.tar.zst jamscript-cli fixture.tar install-home output-dir}"
INSTALL_HOME="${4:?usage: consumer-smoke.sh bundle.tar.zst jamscript-cli fixture.tar install-home output-dir}"
OUTPUT="${5:?usage: consumer-smoke.sh bundle.tar.zst jamscript-cli fixture.tar install-home output-dir}"
RUN_ROOT="${RUNNER_TEMP:-${TMPDIR:-/tmp}}/jamscript-consumer"
PROJECT_ROOT="${RUN_ROOT}/project"
HOME_ROOT="${RUN_ROOT}/home"
CARGO_HOME_ROOT="${RUN_ROOT}/cargo-home"
RUSTUP_HOME_ROOT="${RUN_ROOT}/rustup-home"
NODE_CACHE_ROOT="${RUN_ROOT}/node-cache"
HOST_TOOLS_ROOT="${RUN_ROOT}/host-tools"
LOG="${RUN_ROOT}/smoke.log"
BUNDLE_SHA256="$(sha256sum "${ARCHIVE}" | awk '{print $1}')"

for input in "${ARCHIVE}" "${CLI_BINARY}" "${FIXTURE_ARCHIVE}"; do
  test -f "${input}" || { echo "consumer input is missing: ${input}" >&2; exit 1; }
done
test -x "${CLI_BINARY}"
for directory in "${RUN_ROOT}" "${INSTALL_HOME}" "${OUTPUT}"; do
  if [[ -e "${directory}" ]] && [[ -n "$(find "${directory}" -mindepth 1 -print -quit 2>/dev/null)" ]]; then
    echo "consumer directory is not fresh: ${directory}" >&2
    exit 1
  fi
done
mkdir -p "${RUN_ROOT}" "${INSTALL_HOME}" "${OUTPUT}" \
  "${PROJECT_ROOT}" "${HOME_ROOT}" "${CARGO_HOME_ROOT}" \
  "${RUSTUP_HOME_ROOT}" "${NODE_CACHE_ROOT}" "${HOST_TOOLS_ROOT}"
: > "${LOG}"

run_step() {
  local label="$1"
  shift
  echo "=== ${label} ===" | tee -a "${LOG}"
  set +e
  "$@" 2>&1 | tee -a "${LOG}"
  local command_status="${PIPESTATUS[0]}"
  set -e
  if [[ "${command_status}" != 0 ]]; then
    echo "CONSUMER_FIRST_FAILURE=${label} exit=${command_status}" | tee -a "${LOG}"
    echo "::error title=Offline consumer first failure::${label} exited with ${command_status}; see ${LOG}" >&2
    return "${command_status}"
  fi
}

# The consumer has no source checkout, package manager cache, Rust toolchain,
# or host compiler on PATH. All compiler commands are selected by the managed
# ToolchainManager paths and inherit only the bundle's native library root.
export HOME="${HOME_ROOT}"
export XDG_CACHE_HOME="${RUN_ROOT}/cache"
export CARGO_HOME="${CARGO_HOME_ROOT}"
export RUSTUP_HOME="${RUSTUP_HOME_ROOT}"
export npm_config_cache="${NODE_CACHE_ROOT}"
for host_tool in bash cp find grep mkdir python3 readelf sha256sum stat tar tee; do
  host_tool_path="$(type -P "${host_tool}" || true)"
  test -x "${host_tool_path}" || {
    echo "consumer prerequisite is missing: ${host_tool}" >&2
    exit 1
  }
  ln -s -- "${host_tool_path}" "${HOST_TOOLS_ROOT}/${host_tool}"
done
export PATH="${HOST_TOOLS_ROOT}"
for forbidden_tool in cargo rustc \
  clang cc gcc g++ c++ \
  ld ld.lld llvm-ar ar \
  node; do
  if command -v "${forbidden_tool}" >/dev/null 2>&1; then
    echo "forbidden host tool is visible on consumer PATH: ${forbidden_tool}" >&2
    exit 1
  fi
done
unset JAMSCRIPT_DEV_TOOLCHAIN JAMSCRIPT_TOOLCHAIN_RELEASE_ENGINEERING
unset JAMSCRIPT_MINIJAM_SDK JAMSCRIPT_CLANG JAMSCRIPT_LLVM_ROOT JAMSCRIPT_LLVM_AR JAMSCRIPT_LLVM_LD
unset CARGO_TARGET_DIR RUSTC RUSTFLAGS LIBRARY_PATH CPATH PKG_CONFIG_PATH
unset NODE_PATH NPM_CONFIG_PREFIX npm_config_prefix
export JAMSCRIPT_TOOLCHAIN_HOME="${INSTALL_HOME}"
export JAMSCRIPT_OFFLINE=1
export CARGO_NET_OFFLINE=true
export npm_config_offline=true

run_step "install managed bundle" "${CLI_BINARY}" toolchain install

toolchain_root="$("${CLI_BINARY}" toolchain path)"
test -d "${toolchain_root}"
unset LD_LIBRARY_PATH
export LD_LIBRARY_PATH="${toolchain_root}/lib"

doctor_json="${RUN_ROOT}/doctor.json"
run_step "doctor" bash -c '"$1" doctor --json >"$2"' _ "${CLI_BINARY}" "${doctor_json}"
python3 - "${doctor_json}" "${INSTALL_HOME}" <<'PY'
import json
import pathlib
import sys

doctor = json.loads(pathlib.Path(sys.argv[1]).read_text())
home = pathlib.Path(sys.argv[2]).resolve()
assert doctor["canonical"] is True
assert doctor["offline"] is True
for key in ("node", "clang", "llvm_ar", "ar", "lld", "host_linker", "rustc", "cargo", "jam_sdk"):
    path = pathlib.Path(doctor[key]).resolve()
    assert path == home or home in path.parents, (key, path, home)
print("MANAGED_PATHS_ONLY=PASS")
PY

tar --extract --file "${FIXTURE_ARCHIVE}" --directory "${PROJECT_ROOT}"
test -f "${PROJECT_ROOT}/jamscript.toml"
run_step "offline consumer build" "${CLI_BINARY}" build "${PROJECT_ROOT}" --offline --output "${OUTPUT}"

test -s "${OUTPUT}/build.json"
grep -q '"canonical_toolchain": true' "${OUTPUT}/build.json"
grep -q "\"jamscript_toolchain_sha256\": \"${BUNDLE_SHA256}\"" "${OUTPUT}/build.json"
for artifact in service.blob service.polkavm service.pvm service.abi.json; do
  test -s "${OUTPUT}/${artifact}"
done
if [[ -n "$(find "${CARGO_HOME_ROOT}" -mindepth 1 -print -quit 2>/dev/null)" ]]; then
  echo "consumer Cargo home was unexpectedly populated" >&2
  exit 1
fi
if grep -R -E -n '/home/runner/work/|/home/libingjiang/|JamScript/targets/|minijam-client|Jambda|JAMSCRIPT_MINIJAM_SDK' "${OUTPUT}"; then
  echo "SOURCE_CHECKOUT_LEAK=FAIL" >&2
  exit 1
fi

echo "CLEAN_CONSUMER_HOME=PASS"
echo "EMPTY_CARGO_HOME=PASS"
echo "NO_SOURCE_CHECKOUT_DEPENDENCY=PASS"
echo "NO_MINIJAM_CHECKOUT_DEPENDENCY=PASS"
echo "NO_JAMBDA_DEPENDENCY=PASS"
echo "MANAGED_NODE_ONLY=PASS"
echo "MANAGED_LLVM_ONLY=PASS"
echo "MANAGED_RUST_ONLY=PASS"
echo "MANAGED_JAM_SDK_ONLY=PASS"
echo "OFFLINE_TOOLCHAIN_INSTALL_VERIFY=PASS"
echo "TOOLCHAIN_LOCAL_INSTALL=PASS"
echo "OFFLINE_CONSUMER_BUILD=PASS"
