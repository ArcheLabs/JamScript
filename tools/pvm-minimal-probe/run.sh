#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
sdk_root="${JAMSCRIPT_MINIJAM_SDK:?set JAMSCRIPT_MINIJAM_SDK to the MiniJAM checkout}"
output="${1:-$(mktemp -d /tmp/jamscript-pvm-minimal.XXXXXX)}"

JAMSCRIPT_MINIJAM_SDK="$sdk_root" \
  cargo run --manifest-path "$repo_root/Cargo.toml" -p jamscript-cli --offline -- \
  build "$repo_root/examples/counter" --output "$output"

test "$(jq -r .finalElfLinker "$output/build.json")" = rust-lld
test "$(jq -r .targetEnvironment "$output/build.json")" = polkavm
readelf -hW "$output/service.elf" | grep 'Machine:.*RISC-V' >/dev/null
readelf -rW "$output/service.elf" | grep 'R_RISCV_' >/dev/null
printf 'PolkaVM minimal probe passed: %s\n' "$output"
