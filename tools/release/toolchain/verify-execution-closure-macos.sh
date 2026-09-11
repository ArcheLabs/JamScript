#!/usr/bin/env bash
set -euo pipefail

BUNDLE_ROOT="${1:?usage: verify-execution-closure-macos.sh installed-bundle-root}"
test "$(uname -s)" = "Darwin" || { echo "macOS execution closure requires Darwin" >&2; exit 1; }
test "$(uname -m)" = "arm64" || { echo "macOS execution closure requires native arm64" >&2; exit 1; }
command -v file >/dev/null 2>&1 || { echo "file is required" >&2; exit 1; }
command -v otool >/dev/null 2>&1 || { echo "otool is required" >&2; exit 1; }
xcrun_path=""
if command -v xcrun >/dev/null 2>&1; then
  xcrun_path="$(command -v xcrun)"
fi
sdkroot="${SDKROOT:-}"
if [[ -z "${sdkroot}" ]]; then
  test -n "${xcrun_path}" || {
    echo "Apple SDK discovery requires xcrun or SDKROOT" >&2
    exit 1
  }
  sdkroot="$("${xcrun_path}" --sdk macosx --show-sdk-path)"
fi
test -d "${sdkroot}" || {
  echo "invalid Apple SDK root: ${sdkroot}" >&2
  exit 1
}
echo "MACOS_APPLE_SDK=PASS"

RUN_ROOT="$(mktemp -d "${TMPDIR:-/tmp}/jamscript-macos-closure.XXXXXX")"
trap 'rm -rf -- "${RUN_ROOT}"' EXIT
mkdir -p "${RUN_ROOT}/home" "${RUN_ROOT}/cache"
export HOME="${RUN_ROOT}/home"
export XDG_CACHE_HOME="${RUN_ROOT}/cache"
export CARGO_HOME="${BUNDLE_ROOT}/cargo"
export CARGO_NET_OFFLINE=true
export RUSTC="${BUNDLE_ROOT}/bin/rustc"
export CC="${BUNDLE_ROOT}/bin/clang"
export CXX="${BUNDLE_ROOT}/bin/clang"
export AR="${BUNDLE_ROOT}/bin/ar"
export CARGO_TARGET_AARCH64_APPLE_DARWIN_LINKER="${BUNDLE_ROOT}/bin/jamscript-host-linker"
run_gate() {
  local name="$1"
  shift
  local log="${RUN_ROOT}/${name}.log"
  if "$@" >"${log}" 2>&1; then
    echo "${name}=PASS"
  else
    cat "${log}"
    echo "${name}=FAIL"
    exit 1
  fi
}

for binary in node clang llvm-ar ld64.lld llvm-readelf rustc cargo; do
  path="${BUNDLE_ROOT}/bin/${binary}"
  test -x "${path}" || { echo "managed executable is missing: ${path}" >&2; exit 1; }
  description="$(file -b "${path}")"
  grep -Eiq 'Mach-O.*(arm64|arm64e)' <<<"${description}" || {
    echo "managed executable is not native arm64: ${path} (${description})" >&2
    exit 1
  }
  "${path}" --version >/dev/null 2>&1 || {
    echo "managed executable did not run: ${path}" >&2
    exit 1
  }
  dependencies="$(otool -L "${path}")"
  grep -Eq '(/usr/lib/|/System/Library/|@loader_path|@rpath)' <<<"${dependencies}" || {
    echo "unexpected Mach-O dependency layout: ${path}" >&2
    exit 1
  }
done

printf '%s\n' 'int main(void) { return 0; }' > "${RUN_ROOT}/hello.c"
run_gate MANAGED_CLANG_COMPILE \
  "${BUNDLE_ROOT}/bin/clang" -c "${RUN_ROOT}/hello.c" -o "${RUN_ROOT}/hello.o"

run_gate MANAGED_CLANG_HOST_LINK \
  env SDKROOT="${sdkroot}" \
  "${BUNDLE_ROOT}/bin/jamscript-host-linker" \
  "${RUN_ROOT}/hello.c" -o "${RUN_ROOT}/hello-c"

printf '%s\n' 'fn main() {}' > "${RUN_ROOT}/hello.rs"
run_gate MANAGED_RUST_HOST_LINK \
  env SDKROOT="${sdkroot}" \
  "${BUNDLE_ROOT}/bin/rustc" --edition=2021 \
  -C "linker=${BUNDLE_ROOT}/bin/jamscript-host-linker" \
  "${RUN_ROOT}/hello.rs" -o "${RUN_ROOT}/hello-rust"

test -d "${BUNDLE_ROOT}/lib/rustlib/src/rust/library/compiler-builtins"
test -d "${BUNDLE_ROOT}/scriptc"
test -d "${BUNDLE_ROOT}/runtime"
test -d "${BUNDLE_ROOT}/runtime-scriptc"
test -d "${BUNDLE_ROOT}/targets/jam/sdk"
test -f "${BUNDLE_ROOT}/toolchains/polkavm-guest/Cargo.toml"
test -f "${BUNDLE_ROOT}/toolchains/polkavm-guest/Cargo.lock"

TARGET_JSON="${RUN_ROOT}/riscv64emac-unknown-none-polkavm.json"
cp "${BUNDLE_ROOT}/cargo/vendor/polkavm-linker-0.30.0/targets/1_91/riscv64emac-unknown-none-polkavm.json" "${TARGET_JSON}"
mkdir -p "${RUN_ROOT}/managed-guest/src"
sed \
  -e "s|path = \"../../crates/jamscript-runtime-core\"|path = \"${BUNDLE_ROOT}/runtime/crates/jamscript-runtime-core\"|" \
  -e "s|path = \"../../crates/service-runtime-core\"|path = \"${BUNDLE_ROOT}/runtime/crates/service-runtime-core\"|" \
  -e "s|path = \"../../crates/service-runtime-guest\"|path = \"${BUNDLE_ROOT}/runtime/crates/service-runtime-guest\"|" \
  "${BUNDLE_ROOT}/toolchains/polkavm-guest/Cargo.toml" \
  >"${RUN_ROOT}/managed-guest/Cargo.toml"
cp "${BUNDLE_ROOT}/toolchains/polkavm-guest/Cargo.lock" "${RUN_ROOT}/managed-guest/Cargo.lock"
printf '%s\n' \
  '#![no_std]' \
  '#[panic_handler]' \
  'fn panic(_: &core::panic::PanicInfo) -> ! { loop {} }' \
  '#[no_mangle]' \
  'pub extern "C" fn managed_guest_probe() {}' \
  >"${RUN_ROOT}/managed-guest/src/lib.rs"
run_gate MANAGED_GUEST_OFFLINE_BUILD \
  env SDKROOT="${sdkroot}" \
  "${BUNDLE_ROOT}/bin/cargo" -Z build-std=core,alloc -Z json-target-spec build --release --locked \
  --target "${TARGET_JSON}" --target-dir "${RUN_ROOT}/managed-guest/target" \
  --manifest-path "${RUN_ROOT}/managed-guest/Cargo.toml" --offline

echo "COMPILER_BUILTINS_SOURCE=PASS"
echo "MACOS_MANAGED_EXECUTABLES=PASS"
echo "MACOS_MACHO_DEPENDENCIES=PASS"
echo "MACOS_MANAGED_EXECUTION_CLOSURE=PASS"
