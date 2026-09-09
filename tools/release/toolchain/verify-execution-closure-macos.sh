#!/usr/bin/env bash
set -euo pipefail

BUNDLE_ROOT="${1:?usage: verify-execution-closure-macos.sh installed-bundle-root}"
test "$(uname -s)" = "Darwin" || { echo "macOS execution closure requires Darwin" >&2; exit 1; }
test "$(uname -m)" = "arm64" || { echo "macOS execution closure requires native arm64" >&2; exit 1; }
command -v file >/dev/null 2>&1 || { echo "file is required" >&2; exit 1; }
command -v otool >/dev/null 2>&1 || { echo "otool is required" >&2; exit 1; }

RUN_ROOT="$(mktemp -d "${TMPDIR:-/tmp}/jamscript-macos-closure.XXXXXX")"
trap 'rm -rf -- "${RUN_ROOT}"' EXIT
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
run_gate MANAGED_CLANG_HOST_LINK \
  "${BUNDLE_ROOT}/bin/clang" -fuse-ld=lld \
  "--ld-path=${BUNDLE_ROOT}/bin/ld64.lld" "${RUN_ROOT}/hello.c" -o "${RUN_ROOT}/hello-c"

printf '%s\n' 'fn main() {}' > "${RUN_ROOT}/hello.rs"
run_gate MANAGED_RUST_HOST_LINK \
  "${BUNDLE_ROOT}/bin/rustc" --edition=2021 \
  -C "linker=${BUNDLE_ROOT}/bin/jamscript-host-linker" \
  "${RUN_ROOT}/hello.rs" -o "${RUN_ROOT}/hello-rust"

test -d "${BUNDLE_ROOT}/lib/rustlib/src/rust/library/compiler-builtins"
test -d "${BUNDLE_ROOT}/scriptc"
test -d "${BUNDLE_ROOT}/runtime"
test -d "${BUNDLE_ROOT}/runtime-scriptc"
test -d "${BUNDLE_ROOT}/targets/jam/sdk"
echo "COMPILER_BUILTINS_SOURCE=PASS"
echo "MACOS_MANAGED_EXECUTABLES=PASS"
echo "MACOS_MACHO_DEPENDENCIES=PASS"
echo "MACOS_MANAGED_EXECUTION_CLOSURE=PASS"
