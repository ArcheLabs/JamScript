#!/usr/bin/env bash
set -euo pipefail

BUNDLE_ROOT="${1:?usage: verify-prebuilt-closure.sh bundle-root}"

for binary in node clang llvm-ar guest-linker; do
  path="${BUNDLE_ROOT}/bin/${binary}"
  test -x "${path}" || { echo "missing consumer executable: ${path}" >&2; exit 1; }
  if [[ "${binary}" == "guest-linker" ]]; then
    "${path}" -flavor gnu --version >/dev/null 2>&1
  else
    "${path}" --version >/dev/null 2>&1
  fi || {
    echo "consumer executable did not run: ${path}" >&2
    exit 1
  }
done

for forbidden in \
  bin/rustc \
  bin/cargo \
  bin/jamscript-host-linker \
  cargo \
  lib/rustlib \
  toolchains/polkavm-guest \
  toolchains/polkavm.lock; do
  test ! -e "${BUNDLE_ROOT}/${forbidden}" || {
    echo "forbidden consumer Rust input is present: ${forbidden}" >&2
    exit 1
  }
done
echo "CONSUMER_RUST_ABSENT=PASS"

run_root="$(mktemp -d "${TMPDIR:-/tmp}/jamscript-prebuilt-closure.XXXXXX")"
trap 'rm -rf -- "${run_root}"' EXIT
printf '%s\n' 'int jamscript_consumer_probe(void) { return 0; }' >"${run_root}/probe.c"
"${BUNDLE_ROOT}/bin/clang" \
  --target=riscv64-unknown-elf -march=rv64emac -mabi=lp64e \
  -ffreestanding -fno-builtin -fPIC -ffunction-sections -Os \
  -c "${run_root}/probe.c" -o "${run_root}/probe.o"
"${BUNDLE_ROOT}/bin/llvm-ar" rcsD "${run_root}/probe.a" "${run_root}/probe.o"
test -s "${run_root}/probe.a"
echo "CONSUMER_CLANG_ARCHIVE=PASS"
echo "PREBUILT_CONSUMER_CLOSURE=PASS"
