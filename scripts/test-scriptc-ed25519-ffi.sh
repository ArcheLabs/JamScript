#!/usr/bin/env bash
set -euo pipefail

JAMSCRIPT_ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)"
RUN_ROOT="$(mktemp -d "${TMPDIR:-/tmp}/jamscript-ed25519-ffi.XXXXXX")"
trap 'rm -rf -- "${RUN_ROOT}"' EXIT

[[ "${JAMSCRIPT_DEV_TOOLCHAIN:-}" == "1" ]] || {
  echo "this gate must use the pinned contributor ScriptC/LLVM toolchain" >&2
  exit 2
}
[[ -x "${JAMSCRIPT_ROOT}/target/debug/jams" ]] || {
  echo "target/debug/jams must be built first" >&2
  exit 2
}

"${JAMSCRIPT_ROOT}/target/debug/jams" build \
  "${JAMSCRIPT_ROOT}/tests/release-consumer-ownership" \
  --output "${RUN_ROOT}/dist"

generated_c="${RUN_ROOT}/dist/scriptc/scriptc_service.lib.c"
adapter_c="${RUN_ROOT}/dist/scriptc/scriptc_service_adapter.c"
service_elf="${RUN_ROOT}/dist/service.elf"
service_pvm="${RUN_ROOT}/dist/service.pvm"

grep -Fq -- 'uint32_t (*)(void *, const uint8_t *, size_t, const uint8_t *, size_t, const uint8_t *, size_t)' "${generated_c}"
grep -Fq -- 'scr_library_cb_require(0, "scriptc: library callback' "${generated_c}"
grep -Fq -- 'extern uint32_t jamscript_verify_ed25519(' "${adapter_c}"
grep -Fq -- 'signature_len' "${adapter_c}"
! grep -Fq -- 'scr_undef_global_read' "${generated_c}"

defined_count="$(nm --defined-only "${service_elf}" | awk '$3 == "jamscript_verify_ed25519" { count++ } END { print count + 0 }')"
[[ "${defined_count}" == "1" ]] || {
  echo "expected one linked generic Ed25519 implementation, found ${defined_count}" >&2
  exit 1
}
test -s "${service_pvm}"
echo "SCRIPT_C_GENERIC_ED25519=PASS"
echo "GENERIC_ED25519_NATIVE_SYMBOL_COUNT=${defined_count}"

cargo run --locked --quiet \
  --manifest-path "${JAMSCRIPT_ROOT}/tools/pvm-scriptc-ed25519-ffi/Cargo.toml" \
  --bin pvm-scriptc-ed25519-ffi \
  -- "${service_pvm}"
