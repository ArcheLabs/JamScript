#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
output="$(mktemp -d "${TMPDIR:-/tmp}/jamscript-smoke.XXXXXX")"
trap 'rm -rf "$output"' EXIT

cd "$root"
cargo run --locked --offline -p jamscript-cli -- build examples/counter --output "$output"

for artifact in service.blob service.polkavm service.pvm service.abi.json build.json; do
  test -s "$output/$artifact"
done

rg -q '"rust_toolchain": "nightly-2026-05-02"' "$output/build.json"
rg -q '"jam_target_version": "jam-v1"' "$output/build.json"
rg -q 'minijam_storage_write' "$output/generated_service.rs"
rg -q 'verify_signed_action' "$output/generated_service.rs"

echo "MiniJAM artifact smoke passed."
