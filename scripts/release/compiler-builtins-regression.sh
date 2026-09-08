#!/usr/bin/env bash
set -euo pipefail

# Permanent regression for the historical clean consumer failure. The
# execution-closure probe builds a PolkaVM cdylib with build-std while PATH,
# rustup, and host compilers are unavailable; it therefore exercises the
# bundled compiler-builtins source rather than matching an error string.
ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd -P)"
BUNDLE_ROOT="${1:?usage: compiler-builtins-regression.sh installed-bundle-root}"
test -d "${BUNDLE_ROOT}/lib/rustlib/src/rust/library/compiler-builtins"
case "$(uname -s):$(uname -m)" in
  Darwin:arm64)
    "${ROOT}/tools/release/toolchain/verify-execution-closure-macos.sh" "${BUNDLE_ROOT}"
    ;;
  Linux:x86_64)
    "${ROOT}/tools/release/toolchain/verify-execution-closure.sh" "${BUNDLE_ROOT}"
    ;;
  *)
    echo "unsupported release regression host: $(uname -s) $(uname -m)" >&2
    exit 1
    ;;
esac
echo "COMPILER_BUILTINS_POLKAVM_REGRESSION=PASS"
