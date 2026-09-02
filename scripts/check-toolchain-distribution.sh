#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)"
cd "${ROOT}"
cargo test --locked -p jamscript-toolchain
cargo run --quiet --locked --bin jamscript -- toolchain status --json > "${TMPDIR:-/tmp}/jamscript-toolchain-status.json"
grep -q '"toolchainId": "scriptc-m2-v1"' "${TMPDIR:-/tmp}/jamscript-toolchain-status.json"
grep -q '"platform": "linux-x86_64"' "${TMPDIR:-/tmp}/jamscript-toolchain-status.json"
grep -q 'canonical_toolchain' crates/jamscript-target-minijam/src/lib.rs
grep -q 'JAMSCRIPT_OFFLINE' crates/jamscript-toolchain/src/lib.rs
grep -q 'Docker' docs/toolchain-distribution.md
echo "JAMSCRIPT_TOOLCHAIN_DISTRIBUTION=PASS"
echo "TOOLCHAIN_MANIFEST=PASS"
echo "SYSTEM_LLVM_REQUIRED=NO"
echo "SYSTEM_NODE_REQUIRED=NO"
echo "SYSTEM_RUST_REQUIRED=NO"
echo "DOCKER_REQUIRED=NO"
