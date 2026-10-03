#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
if [[ -n "${JAMSCRIPT_E2E_OUTPUT_DIR:-}" ]]; then
  output="${JAMSCRIPT_E2E_OUTPUT_DIR}"
  mkdir -p "$output"
  keep_output=1
else
  output="$(mktemp -d "${TMPDIR:-/tmp}/jamscript-e2e.XXXXXX")"
  keep_output=0
fi
cleanup() {
  if [[ "$keep_output" == 0 ]]; then
    rm -rf "$output"
  fi
}
trap cleanup EXIT

if [[ -s "${SCRIPTC_NVM_SH:-/home/libingjiang/.nvm/nvm.sh}" ]]; then
  # The release gate is pinned to the toolchain's Node version.
  source "${SCRIPTC_NVM_SH:-/home/libingjiang/.nvm/nvm.sh}"
  nvm use 24.15.0 >/dev/null
fi
test "$(node --version)" = "v24.15.0"

cd "$root"
cargo run --locked -p jamscript-cli --bin jams -- build examples/counter --output "$output"

dynamic_output="$output/dynamic"
cargo run --locked -p jamscript-cli --bin jams -- build examples/dynamic-state-scriptc --output "$dynamic_output"

limited_project="$output/dynamic-state-scriptc-512k"
limited_output="$output/dynamic-512k"
cp -R examples/dynamic-state-scriptc "$limited_project"
cat >> "$limited_project/jamscript.toml" <<'EOF'

[guest.memory]
heap_initial_bytes = 524288
heap_max_bytes = 524288
EOF
cargo run --locked -p jamscript-cli --bin jams -- build "$limited_project" --output "$limited_output"

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
for artifact in service.blob service.polkavm build.json; do
  test -s "$limited_output/$artifact"
done
rg -q '"language_version": "0.2"' "$dynamic_output/build.json"
rg -q '"backend": "scriptc-m2"' "$dynamic_output/build.json"
rg -q '"runtime_profile_version": "scriptc-deterministic-v1"' "$dynamic_output/build.json"
rg -q '"runtimeRefineInputVersion": 1' "$dynamic_output/build.json"
rg -q '"serviceDescriptorVersion": 2' "$dynamic_output/build.json"
rg -q '"heapInitialBytes": 1048576' "$dynamic_output/build.json"
rg -q '"heapMaxBytes": 16777216' "$dynamic_output/build.json"
rg -q '"effectiveHeapMaxBytes": 16777216' "$dynamic_output/build.json"
rg -q '"typedRuntimeVersion": 1' "$dynamic_output/build.json"
rg -q '"stateViewVersion": 1' "$dynamic_output/build.json"
rg -q 'JAMSCRIPT_RUNTIME_REFINE_INPUT_VERSION: u8 = 1' "$dynamic_output/generated_builder_application.rs"

JAMSCRIPT_GUEST_MEMORY_TEST_ARTIFACT="$dynamic_output/service.polkavm" \
JAMSCRIPT_GUEST_MEMORY_LIMIT_TEST_ARTIFACT="$limited_output/service.polkavm" \
cargo test --locked -p jamscript-service-backend --features pvm-memory-tests actual_pvm_guest

JAMSCRIPT_E2E_BUILDER_APPLICATION_RS="$dynamic_output/generated_builder_application.rs" \
JAMSCRIPT_E2E_SCRIPTC_ARCHIVE="$dynamic_output/scriptc/scriptc_service.lib.a" \
cargo run --locked \
  --manifest-path tools/minijam-e2e/Cargo.toml -- --dynamic-only "$dynamic_output/service.blob"

JAMSCRIPT_E2E_BUILDER_APPLICATION_RS="$dynamic_output/generated_builder_application.rs" \
JAMSCRIPT_E2E_SCRIPTC_ARCHIVE="$dynamic_output/scriptc/scriptc_service.lib.a" \
cargo run --locked --offline \
  --manifest-path tools/minijam-e2e/Cargo.toml -- --dynamic-only --memory-probe \
  --diagnostic-item-gas 100000000 "$dynamic_output/service.blob"

formal_limit_log="$output/formal-memory-limit.log"
JAMSCRIPT_E2E_BUILDER_APPLICATION_RS="$dynamic_output/generated_builder_application.rs" \
JAMSCRIPT_E2E_SCRIPTC_ARCHIVE="$dynamic_output/scriptc/scriptc_service.lib.a" \
RUST_LOG=warn cargo run --locked \
  --manifest-path tools/minijam-e2e/Cargo.toml -- --dynamic-only --expect-heap-limit \
  --diagnostic-item-gas 100000000 "$limited_output/service.blob" 2>&1 | tee "$formal_limit_log"

python3 - "$formal_limit_log" <<'PY'
import re
import sys
from pathlib import Path

log = Path(sys.argv[1]).read_text()
match = re.search(
    r"JSGF;v=1;code=(\d+);stage=(\d+);req=(\d+);align=(\d+);"
    r"committed=(\d+);max=(\d+);allocator_live_requested=(\d+);"
    r"allocator_high_water_requested=(\d+);allocator_cumulative_requested=(\d+)",
    log,
)
assert match, "Formal PVM log omitted a complete, parseable JSGF v1 record"
code, stage, requested, alignment, committed, maximum, live, high_water, cumulative = map(
    int, match.groups()
)
assert code == 1, f"expected GUEST_HEAP_LIMIT_EXCEEDED, got fault code {code}"
assert stage == 2, f"expected refine-stage fault, got stage {stage}"
assert requested >= 530000, f"fault request is smaller than the action: {requested}"
assert alignment == 0 or alignment & (alignment - 1) == 0, (
    f"guest request alignment is not a power of two: {alignment}"
)
assert committed == maximum == 524288, (
    f"expected the 512 KiB heap budget, got committed={committed}, max={maximum}"
)
assert live <= high_water <= cumulative, "allocator counters are inconsistent"
print(
    "GUEST_MEMORY_FORMAL_CLASSIFICATION=PASS "
    f"code={code} stage={stage} requested={requested} heap_max={maximum}"
)
PY
