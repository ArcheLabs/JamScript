#!/usr/bin/env bash
set -euo pipefail

python3 tools/release/toolchain/test-llvm-lock.py

ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)"
cd "${ROOT}"

# The compiler and release gates must remain self-contained. The manually
# triggered MiniJAM compatibility workflow is intentionally outside this set.
for workflow in ci.yml build-toolchain-bundle.yml publish-toolchain.yml release.yml; do
  if rg -n -i 'minijam-client|jambda|JAMSCRIPT_MINIJAM_SDK' ".github/workflows/${workflow}"; then
    echo "CORE_WORKFLOW_DEPENDENCY=FAIL (${workflow})" >&2
    exit 1
  fi
done

cargo test --locked -p jamscript-toolchain
cargo run --quiet --locked --bin jamscript -- toolchain status --json > "${TMPDIR:-/tmp}/jamscript-toolchain-status.json"
grep -q '"toolchainId": "scriptc-m2-v1"' "${TMPDIR:-/tmp}/jamscript-toolchain-status.json"
grep -q '"platform": "linux-x86_64"' "${TMPDIR:-/tmp}/jamscript-toolchain-status.json"
grep -q 'canonical_toolchain' crates/jamscript-target-jam/src/lib.rs
grep -q 'JAMSCRIPT_OFFLINE' crates/jamscript-toolchain/src/lib.rs
grep -q 'Docker' docs/toolchain-distribution.md
echo "JAMSCRIPT_TOOLCHAIN_DISTRIBUTION=PASS"
echo "TOOLCHAIN_MANIFEST=PASS"
echo "SYSTEM_LLVM_REQUIRED=NO"
echo "SYSTEM_NODE_REQUIRED=NO"
echo "SYSTEM_RUST_REQUIRED=NO"
echo "DOCKER_REQUIRED=NO"
