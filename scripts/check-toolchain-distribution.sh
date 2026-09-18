#!/usr/bin/env bash
set -euo pipefail

python3 tools/release/toolchain/test-llvm-lock.py
python3 tools/release/toolchain/test-deterministic-archive.py
./scripts/check-host-build-env.sh
python3 scripts/check-ci-trigger-policy.py
python3 scripts/check-release-pipeline-policy.py
python3 tools/release/test-write-release-manifest.py
python3 tools/release/toolchain/test-release-source-check.py

ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)"
cd "${ROOT}"
command -v rg >/dev/null 2>&1 || {
  echo "ripgrep is required for distribution checks" >&2
  exit 1
}

if rg -n --glob '*.sh' -- '--sort=name|--numeric-owner|--owner=|--group=|--mtime=@' tools/release; then
  echo "PORTABLE_RELEASE_ARCHIVES=FAIL GNU-only tar options remain in release scripts" >&2
  exit 1
fi
echo "PORTABLE_RELEASE_ARCHIVES=PASS"

if rg -n -- 'stat -c|stat -f' .github/workflows/release.yml; then
  echo "PORTABLE_RELEASE_METADATA=FAIL platform-specific stat remains in release workflow" >&2
  exit 1
fi
echo "PORTABLE_RELEASE_METADATA=PASS"

# The compiler and release gates must remain self-contained. The manually
# triggered MiniJAM compatibility workflow is intentionally outside this set.
for workflow in ci.yml release.yml; do
  if rg -n -i 'minijam-client|jambda|JAMSCRIPT_MINIJAM_SDK' ".github/workflows/${workflow}"; then
    echo "CORE_WORKFLOW_DEPENDENCY=FAIL (${workflow})" >&2
    exit 1
  fi
done

test -f toolchains/release-targets.toml
test -f toolchains/polkavm-guest/Cargo.toml
test -f toolchains/polkavm-guest/Cargo.lock
grep -q 'triple = "linux-x86_64"' toolchains/release-targets.toml
grep -q 'triple = "macos-arm64"' toolchains/release-targets.toml
grep -q 'triple = "windows-x86_64"' toolchains/release-targets.toml

case "$(uname -s):$(uname -m)" in
  Linux:x86_64) expected_platform='linux-x86_64' ;;
  Darwin:arm64) expected_platform='macos-arm64' ;;
  *) echo "unsupported test runner platform" >&2; exit 1 ;;
esac

cargo test --locked -p jamscript-toolchain
cargo run --quiet --locked --bin jams -- toolchain status --json > "${TMPDIR:-/tmp}/jamscript-toolchain-status.json"
grep -q '"toolchainId": "scriptc-m2-v1"' "${TMPDIR:-/tmp}/jamscript-toolchain-status.json"
grep -q "\"platform\": \"${expected_platform}\"" "${TMPDIR:-/tmp}/jamscript-toolchain-status.json"
grep -q 'canonical_toolchain' crates/jamscript-target-jam/src/lib.rs
grep -q 'JAMSCRIPT_OFFLINE' crates/jamscript-toolchain/src/lib.rs
grep -q 'JAMSCRIPT_RELEASE_KILL_001' scripts/release/release-kill-test-001.sh
grep -q 'compiler-builtins' scripts/release/compiler-builtins-regression.sh
grep -q 'Docker' docs/toolchain-distribution.md
echo "JAMSCRIPT_TOOLCHAIN_DISTRIBUTION=PASS"
echo "TOOLCHAIN_MANIFEST=PASS"
echo "SYSTEM_LLVM_REQUIRED=NO"
echo "SYSTEM_NODE_REQUIRED=NO"
echo "SYSTEM_RUST_REQUIRED=NO"
echo "DOCKER_REQUIRED=NO"
