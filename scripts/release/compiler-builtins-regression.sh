#!/usr/bin/env bash
set -euo pipefail

# Permanent regression for the historical clean consumer failure. The old
# compiler-builtins probe required shipping Cargo and rust-src to consumers;
# the prebuilt closure probe now proves that the fixed guest runtime artifacts
# are sufficient without exposing that producer-only toolchain.
ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd -P)"
BUNDLE_ROOT="${1:?usage: compiler-builtins-regression.sh installed-bundle-root}"
test -s "${BUNDLE_ROOT}/runtime/libjamscript_guest_runtime.a"
test -s "${BUNDLE_ROOT}/runtime/libjamscript_scriptc_runtime.a"
test -s "${BUNDLE_ROOT}/runtime/libjamscript_jam_runtime.a"
"${ROOT}/tools/release/toolchain/verify-prebuilt-closure.sh" "${BUNDLE_ROOT}"
echo "PREBUILT_RUNTIME_POLKAVM_REGRESSION=PASS"
