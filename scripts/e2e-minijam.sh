#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
output="$(mktemp -d "${TMPDIR:-/tmp}/jamscript-e2e.XXXXXX")"
trap 'rm -rf "$output"' EXIT

if [[ -s "${SCRIPTC_NVM_SH:-/home/libingjiang/.nvm/nvm.sh}" ]]; then
  # The release gate is pinned to the toolchain's Node version.
  source "${SCRIPTC_NVM_SH:-/home/libingjiang/.nvm/nvm.sh}"
  nvm use 24.15.0 >/dev/null
fi
test "$(node --version)" = "v24.15.0"

cd "$root"
cargo run --locked --offline -p jamscript-cli -- build examples/counter --output "$output"

dynamic_output="$output/dynamic"
cargo run --locked --offline -p jamscript-cli -- build examples/dynamic-state-scriptc --output "$dynamic_output"

for artifact in service.blob service.polkavm service.pvm service.abi.json build.json; do
  test -s "$output/$artifact"
done

rg -q '"rustToolchain": "nightly-2026-05-02"' "$output/build.json"
rg -q '"jam_target_version": "jam-v1"' "$output/build.json"
rg -q 'minijam_storage_write' "$output/generated_service.rs"
rg -q 'verify_signed_action' "$output/generated_service.rs"

for artifact in service.blob service.polkavm service.pvm service.abi.json build.json generated_service.rs generated_builder_application.rs builder.json; do
  test -s "$dynamic_output/$artifact"
done
rg -q '"language_version": "0.2"' "$dynamic_output/build.json"
rg -q '"backend": "scriptc-m2"' "$dynamic_output/build.json"
rg -q '"runtime_profile_version": "scriptc-deterministic-v1"' "$dynamic_output/build.json"
rg -q '"runtimeRefineInputVersion": 1' "$dynamic_output/build.json"
rg -q '"typedRuntimeVersion": 1' "$dynamic_output/build.json"
rg -q '"stateViewVersion": 1' "$dynamic_output/build.json"
rg -q 'JAMSCRIPT_RUNTIME_REFINE_INPUT_VERSION: u8 = 1' "$dynamic_output/generated_builder_application.rs"

JAMSCRIPT_E2E_BUILDER_APPLICATION_RS="$dynamic_output/generated_builder_application.rs" \
JAMSCRIPT_E2E_SCRIPTC_ARCHIVE="$dynamic_output/scriptc/scriptc_service.lib.a" \
cargo run --locked --offline \
  --manifest-path tools/minijam-e2e/Cargo.toml -- --dynamic-only "$dynamic_output/service.blob"
