#!/usr/bin/env bash
set -euo pipefail

JAMSCRIPT_ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)"
RUN_ROOT="$(mktemp -d "${TMPDIR:-/tmp}/jamscript-contributor-cold.XXXXXX")"
trap 'rm -rf -- "${RUN_ROOT}"' EXIT

nvm_script="${JAMSCRIPT_NVM_SH:-}"
if [[ -z "${nvm_script}" ]]; then
  task_home="$(cd ~ && pwd -P)"
  nvm_script="${task_home}/.nvm/nvm.sh"
fi
if [[ "$(command -v node || true)" == "" || "$(node --version 2>/dev/null || true)" != "v24.15.0" ]] \
  && [[ -s "${nvm_script}" ]]; then
  # shellcheck disable=SC1090
  source "${nvm_script}"
  nvm use 24.15.0 >/dev/null
fi
[[ "$(node --version)" == "v24.15.0" ]] || {
  echo "ScriptC M2 requires Node v24.15.0" >&2
  exit 1
}

command -v cargo >/dev/null 2>&1 || { echo "cargo is required" >&2; exit 127; }
command -v node >/dev/null 2>&1 || { echo "node is required" >&2; exit 127; }

# Build the CLI with the normal workspace cache first. The guest gate below
# uses a separate empty Cargo home and never prefetches into it.
(cd "${JAMSCRIPT_ROOT}" && cargo build --locked --bin jams)

guest_cargo_home="${RUN_ROOT}/cargo-home"
mkdir -p "${guest_cargo_home}"
export CARGO_HOME="${guest_cargo_home}"
export JAMSCRIPT_DEV_TOOLCHAIN=1
export SCRIPTC_NODE="$(command -v node)"
unset CARGO_NET_OFFLINE

"${JAMSCRIPT_ROOT}/target/debug/jams" build \
  "${JAMSCRIPT_ROOT}/examples/dynamic-state-scriptc" \
  --output "${RUN_ROOT}/dist-cold"
test -s "${RUN_ROOT}/dist-cold/service.blob"
echo "CONTRIBUTOR_GUEST_COLD_BUILD=PASS"

"${JAMSCRIPT_ROOT}/target/debug/jams" build \
  "${JAMSCRIPT_ROOT}/examples/dynamic-state-scriptc" \
  --output "${RUN_ROOT}/dist-warm"
test -s "${RUN_ROOT}/dist-warm/service.blob"
echo "CONTRIBUTOR_GUEST_WARM_BUILD=PASS"
