#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../../.." && pwd -P)"
ENV_FILE="${1:?usage: promote-llvm-macos.sh llvm.env [lock-file]}"
LOCK="${2:-${ROOT}/toolchains/llvm/macos-arm64.lock}"

test "$(uname -s)" = Darwin || { echo "LLVM lock promotion requires macOS" >&2; exit 1; }
test "$(uname -m)" = arm64 || { echo "LLVM lock promotion requires native arm64" >&2; exit 1; }
test -f "${ENV_FILE}" || { echo "LLVM measurement file is missing: ${ENV_FILE}" >&2; exit 1; }
set -a
source "${ENV_FILE}"
set +a
for value in "${LLVM_CLANG_SHA256_MEASURED:-}" "${LLVM_AR_SHA256_MEASURED:-}" "${LLVM_LD_LLD_SHA256_MEASURED:-}"; do
  [[ "${value}" =~ ^[0-9a-f]{64}$ ]] || { echo "measurement file does not contain native LLVM hashes" >&2; exit 1; }
  [[ "${value}" != "$(printf '0%.0s' {1..64})" ]] || { echo "native LLVM hash is still the measurement sentinel" >&2; exit 1; }
done

python3 - "${LOCK}" "${LLVM_CLANG_SHA256_MEASURED}" "${LLVM_AR_SHA256_MEASURED}" "${LLVM_LD_LLD_SHA256_MEASURED}" <<'PY'
import pathlib
import re
import sys

path = pathlib.Path(sys.argv[1])
values = dict(zip(("clang_sha256", "llvm_ar_sha256", "ld_lld_sha256"), sys.argv[2:]))
text = path.read_text(encoding="utf-8")
for key, value in values.items():
    text, count = re.subn(rf'^{key} = ".*"$', f'{key} = "{value}"', text, count=1, flags=re.MULTILINE)
    if count != 1:
        raise SystemExit(f"missing LLVM lock field: {key}")
path.write_text(text, encoding="utf-8")
PY
echo "MACOS_LLVM_LOCK_PROMOTED=PASS"
echo "lock=${LOCK}"
