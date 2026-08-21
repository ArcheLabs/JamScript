#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
output="$(mktemp -d "${TMPDIR:-/tmp}/jamscript-e2e.XXXXXX")"
trap 'rm -rf "$output"' EXIT

cd "$root"
cargo run --locked --offline -p jamscript-cli -- build examples/counter --output "$output"

game_output="$output/game"
cargo run --locked --offline -p jamscript-cli -- build examples/game-replay --output "$game_output"

for artifact in service.blob service.polkavm service.pvm service.abi.json build.json; do
  test -s "$output/$artifact"
done

rg -q '"rust_toolchain": "nightly-2026-05-02"' "$output/build.json"
rg -q '"minijam_sdk_revision"' "$output/build.json"
rg -q 'minijam_storage_write' "$output/generated_service.rs"
rg -q 'verify_signed_action' "$output/generated_service.rs"

for artifact in service.blob service.polkavm service.pvm service.abi.json build.json; do
  test -s "$game_output/$artifact"
done
rg -q '"native_abi_version": 1' "$game_output/build.json"
rg -q 'native/game/replay.c' "$game_output/build.json"
rg -q 'jamscript_native_game_replay_v1' "$game_output/generated_service.rs"
rg -q 'best-score/v1' "$game_output/service.abi.json"
rg -q 'getBestScore' "$game_output/service.abi.json"
rg -q '"Bytes<262144>"' "$game_output/service.abi.json"

cargo run --locked --offline \
  --manifest-path tools/minijam-e2e/Cargo.toml -- "$output/service.blob" "$game_output/service.blob"
