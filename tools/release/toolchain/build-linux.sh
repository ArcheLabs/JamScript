#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../../.." && pwd -P)"
SDK_ROOT="${JAMSCRIPT_MINIJAM_SDK:?set JAMSCRIPT_MINIJAM_SDK to the pinned MiniJAM checkout}"
OUT="${1:-${ROOT}/dist/toolchain}"
SOURCE_DATE_EPOCH="${SOURCE_DATE_EPOCH:-$(git -C "${ROOT}" log -1 --format=%ct)}"
NODE_BIN="${SCRIPTC_NODE:?set SCRIPTC_NODE to the exact Node binary}"
CLANG_BIN="${JAMSCRIPT_CLANG:-/usr/lib/llvm-20/bin/clang}"
LLVM_AR_BIN="${JAMSCRIPT_LLVM_AR:-/usr/lib/llvm-20/bin/llvm-ar}"
LLD_BIN="${JAMSCRIPT_LLVM_LD:-/usr/lib/llvm-20/bin/ld.lld}"
RUSTC_BIN="${JAMSCRIPT_RUSTC:-$(rustup which rustc --toolchain nightly-2026-05-02)}"
CARGO_BIN="${JAMSCRIPT_CARGO:-$(rustup which cargo --toolchain nightly-2026-05-02)}"
mkdir -p "${OUT}"
STAGE="$(mktemp -d "${OUT}/.stage.XXXXXX")"
trap 'rm -rf -- "${STAGE}"' EXIT

copy_file() {
  local source="$1" destination="$2"
  test -f "${source}" || { echo "missing bundle input: ${source}" >&2; exit 1; }
  mkdir -p "$(dirname -- "${STAGE}/${destination}")"
  cp -L -- "${source}" "${STAGE}/${destination}"
}

copy_tree() {
  local source="$1" destination="$2"
  test -d "${source}" || { echo "missing bundle input: ${source}" >&2; exit 1; }
  mkdir -p "$(dirname -- "${STAGE}/${destination}")"
  cp -aL --no-target-directory "${source}" "${STAGE}/${destination}"
}

copy_file "${NODE_BIN}" bin/node
copy_file "${CLANG_BIN}" bin/clang
copy_file "${LLVM_AR_BIN}" bin/llvm-ar
copy_file "${LLD_BIN}" bin/ld.lld
copy_file "${RUSTC_BIN}" bin/rustc
copy_file "${CARGO_BIN}" bin/cargo
copy_file "${ROOT}/Cargo.lock" Cargo.lock
copy_file "${ROOT}/toolchains/polkavm.lock" toolchains/polkavm.lock
copy_tree "${ROOT}/toolchains/scriptc" scriptc
copy_tree "${ROOT}/crates/jamscript-runtime-scriptc" runtime-scriptc

mkdir -p "${STAGE}/runtime/crates"
for crate in jamscript-crypto jamscript-runtime-core service-runtime-core service-runtime-state service-runtime-guest; do
  copy_tree "${ROOT}/crates/${crate}" "runtime/crates/${crate}"
done
cp -L "${ROOT}/Cargo.lock" "${STAGE}/runtime/Cargo.lock"
cat > "${STAGE}/runtime/Cargo.toml" <<'EOF'
[workspace]
members = ["crates/jamscript-crypto", "crates/jamscript-runtime-core", "crates/service-runtime-core", "crates/service-runtime-state", "crates/service-runtime-guest"]

[workspace.package]
version = "0.1.0"
edition = "2021"
license = "Apache-2.0"

[workspace.dependencies]
blake2b_simd = "1.0.4"
polkavm-derive = "=0.30.0"
schnorrkel = "0.11.5"
thiserror = "2.0.17"
EOF

mkdir -p "${STAGE}/cargo/vendor"
(cd "${ROOT}" && "${CARGO_BIN}" vendor --locked "${STAGE}/cargo/vendor" >/dev/null)
cat > "${STAGE}/cargo/config.toml" <<'EOF'
[source.crates-io]
replace-with = "vendored-sources"

[source.vendored-sources]
directory = "vendor"
EOF

RUST_SYSROOT="$("${RUSTC_BIN}" --print sysroot)"
copy_tree "${RUST_SYSROOT}/lib/rustlib" lib/rustlib
test -d "${RUST_SYSROOT}/share" && copy_tree "${RUST_SYSROOT}/share" share || true

while read -r dependency; do
  case "${dependency}" in
    /usr/lib/llvm-*/*|/lib/*/libffi.so*|/usr/lib/*/libffi.so*)
      copy_file "${dependency}" "lib/$(basename -- "${dependency}")"
      ;;
  esac
done < <(ldd "${CLANG_BIN}" | awk '$3 ~ /^\// {print $3}' | sort -u)
for binary in bin/node bin/clang bin/llvm-ar bin/ld.lld bin/rustc bin/cargo; do
  if command -v patchelf >/dev/null 2>&1; then
    patchelf --set-rpath '\$ORIGIN/../lib' "${STAGE}/${binary}" || true
  fi
done

test "$("${NODE_BIN}" --version | tr -d '\r\n' | sed 's/^v//')" = "$(tr -d '\r\n' < "${ROOT}/toolchains/scriptc/NODE_VERSION")"
test "$("${CLANG_BIN}" --version | sed -n '1s/.*clang version \([0-9.]*\).*/\1/p')" = "20.1.8"
test "$(git -C "${SDK_ROOT}" rev-parse HEAD)" = "$(sed -n 's/^revision = "\(.*\)"/\1/p' "${ROOT}/toolchains/minijam.lock")"

mkdir -p "${STAGE}/targets/minijam/sdk"
SDK_REVISION="$(sed -n 's/^revision = "\(.*\)"/\1/p' "${ROOT}/toolchains/minijam.lock")"
git -C "${SDK_ROOT}" archive --format=tar "${SDK_REVISION}" | tar -xf - -C "${STAGE}/targets/minijam/sdk"
CONVERTER_MANIFEST="${SDK_ROOT}/service-toolchain/compiler/polkavm-to-jam/Cargo.toml"
CARGO_HOME="${STAGE}/cargo" CARGO_TARGET_DIR="${OUT}/converter-target" RUSTC="${RUSTC_BIN}" \
  "${CARGO_BIN}" build --offline --locked --release --manifest-path "${CONVERTER_MANIFEST}"
copy_file "${OUT}/converter-target/release/minijam-polkavm-to-jam" \
  targets/minijam/sdk/service-toolchain/compiler/polkavm-to-jam/target/release/minijam-polkavm-to-jam

python3 "${ROOT}/tools/release/toolchain/write-manifest.py" \
  --root "${STAGE}" --output "${STAGE}/manifest.json" \
  --platform linux-x86_64 --toolchain-id scriptc-m2-v1 \
  --node-version "$(tr -d '\r\n' < "${ROOT}/toolchains/scriptc/NODE_VERSION")" \
  --clang-version 20.1.8 --rust-toolchain nightly-2026-05-02 \
  --minijam-revision "$(sed -n 's/^revision = "\(.*\)"/\1/p' "${ROOT}/toolchains/minijam.lock")" \
  --scriptc-revision "$(sed -n 's/^commit=//p' "${ROOT}/toolchains/scriptc/REVISION")"

find "${STAGE}" -type f -exec touch -d "@${SOURCE_DATE_EPOCH}" {} +
find "${STAGE}" -type d -exec touch -d "@${SOURCE_DATE_EPOCH}" {} +
ARCHIVE="${OUT}/jamscript-toolchain-scriptc-m2-v1-linux-x86_64.tar.zst"
(cd "${STAGE}" && tar --sort=name --numeric-owner --owner=0 --group=0 --mtime="@${SOURCE_DATE_EPOCH}" --zstd -cf "${ARCHIVE}" .)
sha256sum "${ARCHIVE}"
stat -c '%s' "${ARCHIVE}"
cp -L "${STAGE}/manifest.json" "${OUT}/toolchain-manifest.json"
