#!/usr/bin/env bash
set -euo pipefail

BUNDLE_ROOT="$1"
RUN_ROOT="$(mktemp -d /tmp/jamscript-execution-closure.XXXXXX)"
trap 'rm -rf -- "$RUN_ROOT"' EXIT

test -x "$BUNDLE_ROOT/bin/clang"
test -x "$BUNDLE_ROOT/bin/ld.lld"
test -x "$BUNDLE_ROOT/bin/rustc"
test -x "$BUNDLE_ROOT/bin/cargo"
test -x "$BUNDLE_ROOT/bin/jamscript-host-linker"

mkdir -p "$RUN_ROOT/host-tools" "$RUN_ROOT/home" "$RUN_ROOT/cache"
for host_tool in bash cat cp dirname find grep mkdir mktemp rm sed sha256sum stat tar tee tr; do
  host_tool_path="$(type -P "$host_tool" || true)"
  test -x "$host_tool_path"
  ln -s -- "$host_tool_path" "$RUN_ROOT/host-tools/$host_tool"
done
export PATH="$RUN_ROOT/host-tools"
export HOME="$RUN_ROOT/home"
export XDG_CACHE_HOME="$RUN_ROOT/cache"
export CARGO_HOME="$BUNDLE_ROOT/cargo"
export CARGO_NET_OFFLINE=true
export RUSTC="$BUNDLE_ROOT/bin/rustc"
export CC="$BUNDLE_ROOT/bin/clang"
export CXX="$BUNDLE_ROOT/bin/clang"
export AR="$BUNDLE_ROOT/bin/ar"
export CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER="$BUNDLE_ROOT/bin/jamscript-host-linker"
unset RUSTUP_HOME RUSTFLAGS LD_LIBRARY_PATH
export LD_LIBRARY_PATH="$BUNDLE_ROOT/lib"

run_gate() {
  local name="$1"
  shift
  local log="$RUN_ROOT/$name.log"
  if "$@" >"$log" 2>&1; then
    echo "$name=PASS"
  else
    cat "$log"
    echo "$name=FAIL"
    exit 1
  fi
}

printf '%s\n' 'int main(void) { return 0; }' >"$RUN_ROOT/hello.c"
run_gate MANAGED_CLANG_HOST_LINK \
  "$BUNDLE_ROOT/bin/clang" -fuse-ld=lld "--ld-path=$BUNDLE_ROOT/bin/ld.lld" \
  "$RUN_ROOT/hello.c" -o "$RUN_ROOT/hello-c"

printf '%s\n' 'fn main() {}' >"$RUN_ROOT/hello.rs"
run_gate MANAGED_RUST_HOST_LINK \
  "$BUNDLE_ROOT/bin/rustc" --edition=2021 \
  -C "linker=$BUNDLE_ROOT/bin/jamscript-host-linker" \
  "$RUN_ROOT/hello.rs" -o "$RUN_ROOT/hello-rust"

mkdir -p "$RUN_ROOT/cargo-probe/app/src" "$RUN_ROOT/cargo-probe/macro/src"
printf '%s\n' \
  '[workspace]' \
  'members = ["macro", "app"]' \
  'resolver = "2"' \
  >"$RUN_ROOT/cargo-probe/Cargo.toml"
printf '%s\n' \
  '[package]' \
  'name = "probe-macro"' \
  'version = "0.1.0"' \
  'edition = "2021"' \
  '[lib]' \
  'proc-macro = true' \
  >"$RUN_ROOT/cargo-probe/macro/Cargo.toml"
printf '%s\n' \
  'use proc_macro::TokenStream;' \
  '#[proc_macro_attribute]' \
  'pub fn probe_attr(_: TokenStream, item: TokenStream) -> TokenStream { item }' \
  >"$RUN_ROOT/cargo-probe/macro/src/lib.rs"
printf '%s\n' \
  '[package]' \
  'name = "probe-app"' \
  'version = "0.1.0"' \
  'edition = "2021"' \
  '[dependencies]' \
  'probe-macro = { path = "../macro" }' \
  >"$RUN_ROOT/cargo-probe/app/Cargo.toml"
printf '%s\n' \
  'fn main() {' \
  '    println!("cargo:rerun-if-changed=build.rs");' \
  '}' \
  >"$RUN_ROOT/cargo-probe/app/build.rs"
printf '%s\n' \
  'use probe_macro::probe_attr;' \
  '#[probe_attr]' \
  'fn main() { println!("managed proc macro"); }' \
  >"$RUN_ROOT/cargo-probe/app/src/main.rs"

run_gate MANAGED_CARGO_BUILD_SCRIPT \
  "$BUNDLE_ROOT/bin/cargo" build --offline \
  --manifest-path "$RUN_ROOT/cargo-probe/app/Cargo.toml"
test -n "$(find "$RUN_ROOT/cargo-probe/target/debug/build" -type f -name '*-build-script-build' -print -quit 2>/dev/null)"
echo "MANAGED_CARGO_PROC_MACRO=PASS"

TARGET_JSON="$RUN_ROOT/riscv64emac-unknown-none-polkavm.json"
cp "$BUNDLE_ROOT/cargo/vendor/polkavm-linker-0.30.0/targets/1_91/riscv64emac-unknown-none-polkavm.json" "$TARGET_JSON"
mkdir -p "$RUN_ROOT/cross-probe/guest/src" "$RUN_ROOT/cross-probe/macro/src"
printf '%s\n' \
  '[workspace]' \
  'members = ["macro", "guest"]' \
  'resolver = "2"' \
  >"$RUN_ROOT/cross-probe/Cargo.toml"
printf '%s\n' \
  '[package]' \
  'name = "cross-probe-macro"' \
  'version = "0.1.0"' \
  'edition = "2021"' \
  '[lib]' \
  'proc-macro = true' \
  >"$RUN_ROOT/cross-probe/macro/Cargo.toml"
cp "$RUN_ROOT/cargo-probe/macro/src/lib.rs" "$RUN_ROOT/cross-probe/macro/src/lib.rs"
printf '%s\n' \
  '[package]' \
  'name = "cross-probe-guest"' \
  'version = "0.1.0"' \
  'edition = "2021"' \
  '[lib]' \
  'crate-type = ["cdylib"]' \
  '[dependencies]' \
  'cross-probe-macro = { path = "../macro" }' \
  >"$RUN_ROOT/cross-probe/guest/Cargo.toml"
printf '%s\n' \
  '#![no_std]' \
  'use cross_probe_macro::probe_attr;' \
  '#[panic_handler]' \
  'fn panic(_: &core::panic::PanicInfo) -> ! { loop {} }' \
  '#[probe_attr]' \
  '#[no_mangle]' \
  'pub extern "C" fn probe_entry() {}' \
  >"$RUN_ROOT/cross-probe/guest/src/lib.rs"
printf '%s\n' \
  'fn main() { println!("cross probe build script"); }' \
  >"$RUN_ROOT/cross-probe/guest/build.rs"
run_gate MANAGED_CROSS_CARGO_HOST_GUEST_SPLIT \
  "$BUNDLE_ROOT/bin/cargo" -Z build-std=core,alloc -Z json-target-spec build --release \
  --target "$TARGET_JSON" --target-dir "$RUN_ROOT/cross-probe/target" \
  --manifest-path "$RUN_ROOT/cross-probe/guest/Cargo.toml" --offline

mkdir -p "$RUN_ROOT/no-lld"
cp "$BUNDLE_ROOT/bin/jamscript-host-linker" "$RUN_ROOT/no-lld/jamscript-host-linker"
cp "$BUNDLE_ROOT/bin/clang" "$RUN_ROOT/no-lld/clang"
if "$RUN_ROOT/no-lld/jamscript-host-linker" "$RUN_ROOT/hello.c" -o "$RUN_ROOT/no-lld/hello" >"$RUN_ROOT/no-lld.log" 2>&1; then
  cat "$RUN_ROOT/no-lld.log"
  echo "MANAGED_HOST_LINKER_DEPENDENCY_TEST=FAIL"
  exit 1
fi
echo "MANAGED_HOST_LINKER_DEPENDENCY_TEST=PASS"
