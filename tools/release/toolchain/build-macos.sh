#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../../.." && pwd -P)"
OUT="${1:-${ROOT}/dist/toolchain}"
SOURCE_DATE_EPOCH="${SOURCE_DATE_EPOCH:-$(git -C "${ROOT}" log -1 --format=%ct)}"
NODE_BIN="${SCRIPTC_NODE:?set SCRIPTC_NODE to the exact Node binary}"
LLVM_LOCK="${ROOT}/toolchains/llvm/macos-arm64.lock"
LLVM_LOCK_PARSER="${ROOT}/tools/release/toolchain/llvm-lock.py"
LLVM_VERSION="$(python3 "${LLVM_LOCK_PARSER}" "${LLVM_LOCK}" --get llvm_version)"
LLVM_DISTRIBUTION="$(python3 "${LLVM_LOCK_PARSER}" "${LLVM_LOCK}" --get distribution)"
LLVM_ARCHIVE_SHA256="$(python3 "${LLVM_LOCK_PARSER}" "${LLVM_LOCK}" --get archive_sha256)"

test "$(uname -s)" = "Darwin" || { echo "build-macos.sh requires Darwin" >&2; exit 1; }
test "$(uname -m)" = "arm64" || { echo "build-macos.sh requires native arm64" >&2; exit 1; }
test "${JAMSCRIPT_TOOLCHAIN_RELEASE_ENGINEERING:-0}" = "1" || {
  echo "build-macos.sh is release-engineering-only" >&2
  exit 1
}
CLANG_BIN="${JAMSCRIPT_CLANG:?bootstrap LLVM before building the macOS bundle}"
LLVM_AR_BIN="${JAMSCRIPT_LLVM_AR:?bootstrap LLVM before building the macOS bundle}"
LLD_BIN="${JAMSCRIPT_LLVM_LD:?bootstrap LLVM before building the macOS bundle}"
LLVM_ROOT="${JAMSCRIPT_LLVM_ROOT:?bootstrap LLVM before building the macOS bundle}"
LLVM_READELF_BIN="${LLVM_ROOT}/bin/llvm-readelf"
RUSTC_BIN="${JAMSCRIPT_RUSTC:-$(rustup which rustc --toolchain nightly-2026-05-02)}"
CARGO_BIN="${JAMSCRIPT_CARGO:-$(rustup which cargo --toolchain nightly-2026-05-02)}"
command -v zstd >/dev/null 2>&1 || { echo "zstd is required only by release engineering to package the toolchain" >&2; exit 1; }
command -v otool >/dev/null 2>&1 || { echo "otool is required to verify macOS runtime dependencies" >&2; exit 1; }
command -v install_name_tool >/dev/null 2>&1 || { echo "install_name_tool is required to repair macOS runtime paths" >&2; exit 1; }

sha256_file() {
  if command -v shasum >/dev/null 2>&1; then
    shasum -a 256 "$1" | awk '{print $1}'
  else
    sha256sum "$1" | awk '{print $1}'
  fi
}

mkdir -p "${OUT}"
STAGE="$(mktemp -d "${OUT}/.stage.XXXXXX")"
trap 'rm -rf -- "${STAGE}"' EXIT

copy_file() {
  local source="$1" destination="$2"
  test -f "${source}" || { echo "missing bundle input: ${source}" >&2; exit 1; }
  mkdir -p "$(dirname -- "${STAGE}/${destination}")"
  cp -L "${source}" "${STAGE}/${destination}"
}

copy_tree() {
  local source="$1" destination="$2"
  test -d "${source}" || { echo "missing bundle input: ${source}" >&2; exit 1; }
  mkdir -p "${STAGE}/${destination}"
  cp -R -L "${source}/." "${STAGE}/${destination}/"
}

copy_file "${NODE_BIN}" bin/node
copy_file "${CLANG_BIN}" bin/clang
copy_file "${LLVM_AR_BIN}" bin/llvm-ar
copy_file "${LLVM_AR_BIN}" bin/ar
copy_file "${LLD_BIN}" bin/ld.lld
copy_file "${LLVM_READELF_BIN}" bin/llvm-readelf
copy_file "${ROOT}/tools/release/toolchain/jamscript-host-linker" bin/jamscript-host-linker
copy_file "${RUSTC_BIN}" bin/rustc
copy_file "${CARGO_BIN}" bin/cargo
copy_file "${ROOT}/Cargo.lock" Cargo.lock
copy_file "${ROOT}/toolchains/polkavm.lock" toolchains/polkavm.lock
copy_tree "${ROOT}/toolchains/scriptc" scriptc
copy_tree "${ROOT}/crates/jamscript-runtime-scriptc" runtime-scriptc
copy_tree "${ROOT}/crates/jamscript-target-jam/sdk" targets/jam/sdk

LLVM_RESOURCE_DIR="$(${CLANG_BIN} -print-resource-dir)"
case "${LLVM_RESOURCE_DIR}" in
  "${LLVM_ROOT}"/*) copy_tree "${LLVM_RESOURCE_DIR}" "${LLVM_RESOURCE_DIR#"${LLVM_ROOT}"/}" ;;
  *) echo "clang resource directory is outside the locked LLVM root: ${LLVM_RESOURCE_DIR}" >&2; exit 1 ;;
esac

mkdir -p "${STAGE}/runtime/crates"
for crate in jamscript-crypto jamscript-runtime-core service-runtime-core service-runtime-state service-runtime-guest; do
  copy_tree "${ROOT}/crates/${crate}" "runtime/crates/${crate}"
done
cp -L "${ROOT}/Cargo.lock" "${STAGE}/runtime/Cargo.lock"
cat > "${STAGE}/runtime/Cargo.toml" <<'EOF'
[workspace]
resolver = "2"
members = ["crates/jamscript-crypto", "crates/jamscript-runtime-core", "crates/service-runtime-core", "crates/service-runtime-state", "crates/service-runtime-guest"]

[workspace.package]
version = "0.1.0"
edition = "2021"
license = "Apache-2.0"

[workspace.dependencies]
blake2b_simd = { version = "1.0.4", default-features = false }
polkavm-derive = "=0.30.0"
schnorrkel = { version = "0.11.5", default-features = false }
thiserror = { version = "2.0.17", default-features = false }
EOF

mkdir -p "${STAGE}/cargo/vendor"
(cd "${ROOT}" && "${CARGO_BIN}" vendor --locked --versioned-dirs "${STAGE}/cargo/vendor" >/dev/null)
(cd "${ROOT}" && "${CARGO_BIN}" vendor --locked --versioned-dirs --no-delete --sync crates/jamscript-runtime-core/Cargo.toml "${STAGE}/cargo/vendor" >/dev/null)
RUST_SYSROOT="$(${RUSTC_BIN} --print sysroot)"
(cd "${RUST_SYSROOT}/lib/rustlib/src/rust" && "${CARGO_BIN}" vendor --locked --versioned-dirs --no-delete --manifest-path library/Cargo.toml "${STAGE}/cargo/vendor" >/dev/null)
cat > "${STAGE}/cargo/config.toml" <<'EOF'
[source.crates-io]
replace-with = "vendored-sources"

[source.vendored-sources]
directory = "cargo/vendor"
EOF

copy_tree "${RUST_SYSROOT}/lib/rustlib" lib/rustlib
while IFS= read -r runtime_library; do
  copy_file "${runtime_library}" "lib/$(basename -- "${runtime_library}")"
done < <(
  find "${RUST_SYSROOT}/lib" \
    -type d ! -path "${RUST_SYSROOT}/lib" -prune -o \
    \( -type f -o -type l \) \
    -name '*.dylib*' \
    -print |
  sort
)
test -d "${RUST_SYSROOT}/share" && copy_tree "${RUST_SYSROOT}/share" share || true

for binary in "${NODE_BIN}" "${CLANG_BIN}" "${LLVM_AR_BIN}" "${LLD_BIN}" "${LLVM_READELF_BIN}" "${RUSTC_BIN}" "${CARGO_BIN}"; do
  description="$(file -b "${binary}")"
  grep -Eiq 'Mach-O.*(arm64|arm64e)' <<<"${description}" || {
    echo "non-arm64 executable entered the macOS bundle: ${binary} (${description})" >&2
    exit 1
  }
done

ensure_bundle_rpath() {
  local binary="$1"
  if ! otool -l "${binary}" | grep -F 'path @executable_path/../lib' >/dev/null; then
    install_name_tool -add_rpath '@executable_path/../lib' "${binary}"
  fi
}

# Rust's macOS binaries use @rpath for the compiler runtime. Keep the
# relocation self-contained even if a future Rust release omits the standard
# rustup rpath from one of its binaries or dylibs.
ensure_bundle_rpath "${STAGE}/bin/rustc"
ensure_bundle_rpath "${STAGE}/bin/cargo"
while IFS= read -r runtime_library; do
  ensure_bundle_rpath "${runtime_library}"
done < <(
  find "${STAGE}/lib" \
    -type d ! -path "${STAGE}/lib" -prune -o \
    -type f -name '*.dylib*' -print |
  sort
)

verify_macos_dependency_closure() {
  local consumer="$1"
  local dependency relative resolved
  while IFS= read -r dependency; do
    [[ -n "${dependency}" ]] || continue
    case "${dependency}" in
      /usr/lib/*|/System/Library/*)
        ;;
      @rpath/*)
        relative="${dependency#@rpath/}"
        test -f "${STAGE}/lib/${relative}" || {
          echo "missing bundle @rpath dependency: ${dependency} (from ${consumer})" >&2
          exit 1
        }
        ;;
      @loader_path/*)
        relative="${dependency#@loader_path/}"
        resolved="$(dirname -- "${consumer}")/${relative}"
        test -f "${resolved}" || {
          echo "missing bundle @loader_path dependency: ${dependency} (from ${consumer})" >&2
          exit 1
        }
        ;;
      @executable_path/*)
        relative="${dependency#@executable_path/}"
        resolved="${STAGE}/bin/${relative}"
        test -f "${resolved}" || {
          echo "missing bundle @executable_path dependency: ${dependency} (from ${consumer})" >&2
          exit 1
        }
        ;;
      *)
        echo "non-relocatable macOS dependency: ${dependency} (from ${consumer})" >&2
        exit 1
        ;;
    esac
  done < <(otool -L "${consumer}" | awk 'NR > 1 {print $1}')
}

for binary in "${STAGE}/bin/node" "${STAGE}/bin/clang" "${STAGE}/bin/llvm-ar" \
  "${STAGE}/bin/ld.lld" "${STAGE}/bin/llvm-readelf" "${STAGE}/bin/rustc" "${STAGE}/bin/cargo"; do
  verify_macos_dependency_closure "${binary}"
done
while IFS= read -r runtime_library; do
  verify_macos_dependency_closure "${runtime_library}"
done < <(
  find "${STAGE}/lib" \
    -type d ! -path "${STAGE}/lib" -prune -o \
    -type f -name '*.dylib*' -print |
  sort
)
echo "STAGED_DEPENDENCY_CLOSURE=PASS"

staged_version_gate() {
  local name="$1" binary="$2"
  env -i HOME="${HOME}" PATH="/usr/bin:/bin" "${binary}" --version >/dev/null
  echo "${name}=PASS"
}

staged_version_gate STAGED_NODE "${STAGE}/bin/node"
staged_version_gate STAGED_CLANG "${STAGE}/bin/clang"
staged_version_gate STAGED_LLVM_AR "${STAGE}/bin/llvm-ar"
staged_version_gate STAGED_LLD "${STAGE}/bin/ld.lld"
staged_version_gate STAGED_LLVM_READELF "${STAGE}/bin/llvm-readelf"
staged_version_gate STAGED_RUSTC "${STAGE}/bin/rustc"
staged_version_gate STAGED_CARGO "${STAGE}/bin/cargo"

clang_sha256="$(sha256_file "${CLANG_BIN}")"
llvm_ar_sha256="$(sha256_file "${LLVM_AR_BIN}")"
ld_lld_sha256="$(sha256_file "${LLD_BIN}")"
python3 "${ROOT}/tools/release/toolchain/write-manifest.py" \
  --root "${STAGE}" --output "${STAGE}/manifest.json" \
  --platform macos-arm64 --toolchain-id scriptc-m2-v1 \
  --node-version "$(tr -d '\r\n' < "${ROOT}/toolchains/scriptc/NODE_VERSION")" \
  --clang-version "${LLVM_VERSION}" \
  --llvm-distribution "${LLVM_DISTRIBUTION}" --llvm-archive-sha256 "${LLVM_ARCHIVE_SHA256}" \
  --llvm-clang-sha256 "${clang_sha256}" --llvm-ar-sha256 "${llvm_ar_sha256}" --llvm-lld-sha256 "${ld_lld_sha256}" \
  --rust-toolchain nightly-2026-05-02 \
  --jam-target-version "$(sed -n 's/^jam_target_version = \"\(.*\)\"/\1/p' "${ROOT}/toolchains/distribution-v1.toml")" \
  --jam-blob-encoder-version "$(sed -n 's/^jam_blob_encoder_version = \"\(.*\)\"/\1/p' "${ROOT}/toolchains/distribution-v1.toml")" \
  --scriptc-revision "$(sed -n 's/^commit=//p' "${ROOT}/toolchains/scriptc/REVISION")"

# BSD tar is used on the native runner. zstd remains an internal bundle
# format; it is never required by the end-user CLI bootstrap archive.
find "${STAGE}" -type f -exec touch -t 197001010000 {} +
find "${STAGE}" -type d -exec touch -t 197001010000 {} +
ARCHIVE="${OUT}/jamscript-toolchain-scriptc-m2-v1-macos-arm64.tar.zst"
(cd "${STAGE}" && tar --format pax --uid 0 --gid 0 --uname root --gname root -cf - . | zstd -q -T0 -19 -o "${ARCHIVE}")
if command -v shasum >/dev/null 2>&1; then shasum -a 256 "${ARCHIVE}"; else sha256sum "${ARCHIVE}"; fi
wc -c < "${ARCHIVE}"
cp -L "${STAGE}/manifest.json" "${OUT}/toolchain-manifest-macos-arm64.json"
python3 "${ROOT}/tools/release/toolchain/write-bundle-metadata.py" \
  --output "${OUT}/bundle-metadata-macos-arm64.json" \
  --toolchain-id scriptc-m2-v1 --platform macos-arm64 \
  --archive "$(basename -- "${ARCHIVE}")" \
  --source-revision "$(git -C "${ROOT}" rev-parse HEAD)"
echo "BUNDLE_PATH=${ARCHIVE}"
