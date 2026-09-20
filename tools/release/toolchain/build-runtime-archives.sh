#!/usr/bin/env bash
set -euo pipefail

# Build fixed guest-side inputs during release engineering. The resulting
# archives are the only runtime inputs consumed by the application linker.
ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../../.." && pwd -P)"
STAGE="${1:?usage: build-runtime-archives.sh <bundle-stage>}"
CARGO_BIN="${JAMSCRIPT_CARGO:?set JAMSCRIPT_CARGO to the release Cargo binary}"
RUSTC_BIN="${JAMSCRIPT_RUSTC:?set JAMSCRIPT_RUSTC to the release rustc}"
CLANG_BIN="${JAMSCRIPT_CLANG:?set JAMSCRIPT_CLANG to the release clang}"
LLVM_AR_BIN="${JAMSCRIPT_LLVM_AR:?set JAMSCRIPT_LLVM_AR to the release llvm-ar}"
TARGET_JSON="${JAMSCRIPT_POLKAVM_TARGET_JSON:?set JAMSCRIPT_POLKAVM_TARGET_JSON to the locked target JSON}"

test -d "${STAGE}/scriptc/node_modules/@scriptc/runtime/src"
test -d "${STAGE}/runtime-scriptc"
test -d "${STAGE}/targets/jam/sdk"
mkdir -p "${STAGE}/runtime" "${STAGE}/.runtime-build"

guest_target="${STAGE}/.runtime-build/guest-target"
RUSTC="${RUSTC_BIN}" "${CARGO_BIN}" build \
  -Z build-std=core,alloc \
  -Z json-target-spec \
  --target "${TARGET_JSON}" \
  --release \
  --locked \
  --manifest-path "${ROOT}/toolchains/polkavm-guest-runtime/Cargo.toml" \
  --target-dir "${guest_target}"
cp -L \
  "${guest_target}/riscv64emac-unknown-none-polkavm/release/libjamscript_guest_runtime.a" \
  "${STAGE}/runtime/libjamscript_guest_runtime.a"

common_flags=(
  "--target=riscv64-unknown-elf"
  "-march=rv64emac"
  "-mabi=lp64e"
  "-ffreestanding"
  "-fno-builtin"
  "-fPIC"
  "-fdata-sections"
  "-ffunction-sections"
  "-Os"
  "-DSCR_LIB"
)
runtime_src="${STAGE}/scriptc/node_modules/@scriptc/runtime/src"
runtime_include="${STAGE}/runtime-scriptc/include"

build_archive() {
  local name="${1}"
  shift
  local archive="${STAGE}/runtime/libjamscript_${name}_runtime.a"
  local object_dir="${STAGE}/.runtime-build/${name}"
  mkdir -p "${object_dir}"
  local objects=()
  local index=0
  local source object
  for source in "$@"; do
    object="${object_dir}/${index}.o"
    "${CLANG_BIN}" "${common_flags[@]}" -std=c11 \
      -I "${runtime_include}" -I "${runtime_src}" \
      -I "${STAGE}/targets/jam/sdk/include" \
      -c "${source}" -o "${object}"
    objects+=("${object}")
    index=$((index + 1))
  done
  ZERO_AR_DATE=1 "${LLVM_AR_BIN}" rcsD "${archive}" "${objects[@]}"
}

scriptc_sources=()
for unit in \
  scr_library.c scr_number.c scr_string.c scr_array.c scr_bytes.c \
  scr_closure.c scr_cycle.c scr_error.c scr_exception.c scr_json.c \
  scr_object.c scr_union.c; do
  scriptc_sources+=("${runtime_src}/${unit}")
done
scriptc_sources+=(
  "${STAGE}/runtime-scriptc/src/scr_lib_cleanup.c"
  "${STAGE}/runtime-scriptc/src/freestanding.c"
)
build_archive scriptc "${scriptc_sources[@]}"

build_archive jam \
  "${STAGE}/targets/jam/sdk/src/host.c" \
  "${STAGE}/targets/jam/sdk/src/minijam.c" \
  "${STAGE}/targets/jam/sdk/src/crypto.c"

rm -rf -- "${STAGE}/.runtime-build"
echo "PREBUILT_GUEST_RUNTIME=PASS"
echo "PREBUILT_SCRIPTC_RUNTIME=PASS"
echo "PREBUILT_JAM_RUNTIME=PASS"
