#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../../.." && pwd -P)"
OUT="${1:-${ROOT}/dist/toolchain}"
SOURCE_DATE_EPOCH="${SOURCE_DATE_EPOCH:-$(git -C "${ROOT}" log -1 --format=%ct)}"
NODE_BIN="${SCRIPTC_NODE:?set SCRIPTC_NODE to the exact Node binary}"
LLVM_LOCK="${ROOT}/toolchains/llvm/linux-x86_64.lock"
LLVM_LOCK_PARSER="${ROOT}/tools/release/toolchain/llvm-lock.py"
LLVM_VERSION="$(python3 "${LLVM_LOCK_PARSER}" "${LLVM_LOCK}" --get llvm_version)"
LLVM_DISTRIBUTION="$(python3 "${LLVM_LOCK_PARSER}" "${LLVM_LOCK}" --get distribution)"
LLVM_ARCHIVE_SHA256="$(python3 "${LLVM_LOCK_PARSER}" "${LLVM_LOCK}" --get archive_sha256)"
LLVM_CLANG_SHA256="$(python3 "${LLVM_LOCK_PARSER}" "${LLVM_LOCK}" --get clang_sha256)"
LLVM_AR_SHA256="$(python3 "${LLVM_LOCK_PARSER}" "${LLVM_LOCK}" --get llvm_ar_sha256)"
LLVM_LLD_SHA256="$(python3 "${LLVM_LOCK_PARSER}" "${LLVM_LOCK}" --get ld_lld_sha256)"
if [[ "${JAMSCRIPT_TOOLCHAIN_RELEASE_ENGINEERING:-0}" == "1" ]]; then
  CLANG_BIN="${JAMSCRIPT_CLANG:?set JAMSCRIPT_CLANG to the bootstrapped LLVM clang}"
  LLVM_AR_BIN="${JAMSCRIPT_LLVM_AR:?set JAMSCRIPT_LLVM_AR to the bootstrapped LLVM llvm-ar}"
  LLD_BIN="${JAMSCRIPT_LLVM_LD:?set JAMSCRIPT_LLVM_LD to the bootstrapped LLVM ld.lld}"
  LLVM_ROOT="${JAMSCRIPT_LLVM_ROOT:?set JAMSCRIPT_LLVM_ROOT to the bootstrapped LLVM root}"
else
  # Development compatibility is explicit; release engineering cannot use it.
  test "${JAMSCRIPT_DEV_TOOLCHAIN:-0}" = "1" || {
    echo "build-linux.sh requires JAMSCRIPT_TOOLCHAIN_RELEASE_ENGINEERING=1 or explicit JAMSCRIPT_DEV_TOOLCHAIN=1" >&2
    exit 1
  }
  CLANG_BIN="${JAMSCRIPT_CLANG:-/usr/lib/llvm-20/bin/clang}"
  LLVM_AR_BIN="${JAMSCRIPT_LLVM_AR:-/usr/lib/llvm-20/bin/llvm-ar}"
  LLD_BIN="${JAMSCRIPT_LLVM_LD:-/usr/lib/llvm-20/bin/ld.lld}"
  LLVM_ROOT="${JAMSCRIPT_LLVM_ROOT:-$(cd -- "$(dirname -- "${CLANG_BIN}")/.." && pwd -P)}"
fi
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
  cp -aL --no-preserve=links --no-target-directory "${source}" "${STAGE}/${destination}"
}

copy_file "${NODE_BIN}" bin/node
copy_file "${CLANG_BIN}" bin/clang
copy_file "${LLVM_AR_BIN}" bin/llvm-ar
copy_file "${LLVM_AR_BIN}" bin/ar
copy_file "${LLD_BIN}" bin/ld.lld
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
while read -r runtime_library; do
  copy_file "${runtime_library}" "lib/$(basename -- "${runtime_library}")"
done < <(find "${RUST_SYSROOT}/lib" -maxdepth 1 -type f -name '*.so*' -print | sort)
test -d "${RUST_SYSROOT}/share" && copy_tree "${RUST_SYSROOT}/share" share || true

declare -a dependency_queue=("${NODE_BIN}" "${CLANG_BIN}" "${LLVM_AR_BIN}" "${LLD_BIN}" "${RUSTC_BIN}" "${CARGO_BIN}")
declare -A seen_dependencies=()
RUNTIME_LIBRARY_PATH="${RUST_SYSROOT}/lib:${LLVM_ROOT}/lib${LD_LIBRARY_PATH:+:${LD_LIBRARY_PATH}}"
while ((${#dependency_queue[@]})); do
  binary="${dependency_queue[0]}"
  dependency_queue=("${dependency_queue[@]:1}")
  [[ -n "${seen_dependencies[${binary}]:-}" ]] && continue
  seen_dependencies["${binary}"]=1
  ldd_output="$(LD_LIBRARY_PATH="${RUNTIME_LIBRARY_PATH}" ldd "${binary}" 2>&1)" || {
    echo "unable to inspect runtime dependencies: ${binary}" >&2
    echo "${ldd_output}" >&2
    exit 1
  }
  grep -q 'not found' <<<"${ldd_output}" && { echo "unresolved runtime dependency: ${binary}" >&2; echo "${ldd_output}" >&2; exit 1; }
  while read -r dependency; do
    [[ -n "${dependency}" ]] || continue
    [[ -n "${seen_dependencies[${dependency}]:-}" ]] && continue
    base="$(basename -- "${dependency}")"
    case "${base}" in
      libc.so*|libm.so*|libdl.so*|libpthread.so*|librt.so*|libresolv.so*|libnss_*.so*|ld-linux*.so*)
        seen_dependencies["${dependency}"]=1
        ;;
      *)
        copy_file "${dependency}" "lib/${base}"
        dependency_queue+=("${dependency}")
        ;;
    esac
  done < <(awk '$3 ~ /^\// {print $3}' <<<"${ldd_output}" | sort -u)
done
for binary in bin/node bin/rustc bin/cargo; do
  if command -v patchelf >/dev/null 2>&1; then
    patchelf --set-rpath '$ORIGIN/../lib' "${STAGE}/${binary}"
  fi
done
if [[ "${JAMSCRIPT_TOOLCHAIN_RELEASE_ENGINEERING:-0}" != "1" && "$(command -v patchelf || true)" ]]; then
  for binary in bin/clang bin/llvm-ar bin/ld.lld; do
    patchelf --set-rpath '$ORIGIN/../lib' "${STAGE}/${binary}"
  done
fi

test "$("${NODE_BIN}" --version | tr -d '\r\n' | sed 's/^v//')" = "$(tr -d '\r\n' < "${ROOT}/toolchains/scriptc/NODE_VERSION")"
test "$("${CLANG_BIN}" --version | sed -n '1s/.*clang version \([0-9.]*\).*/\1/p')" = "${LLVM_VERSION}"
test -d "${STAGE}/targets/jam/sdk"

rm -f -- "${STAGE}/cargo/.global-cache" "${STAGE}/cargo/.package-cache" \
  "${STAGE}/cargo/.package-cache-mutate"

python3 "${ROOT}/tools/release/toolchain/write-manifest.py" \
  --root "${STAGE}" --output "${STAGE}/manifest.json" \
  --platform linux-x86_64 --toolchain-id scriptc-m2-v1 \
  --node-version "$(tr -d '\r\n' < "${ROOT}/toolchains/scriptc/NODE_VERSION")" \
  --clang-version "${LLVM_VERSION}" \
  --llvm-distribution "${LLVM_DISTRIBUTION}" --llvm-archive-sha256 "${LLVM_ARCHIVE_SHA256}" \
  --llvm-clang-sha256 "${LLVM_CLANG_SHA256}" --llvm-ar-sha256 "${LLVM_AR_SHA256}" --llvm-lld-sha256 "${LLVM_LLD_SHA256}" \
  --rust-toolchain nightly-2026-05-02 \
  --jam-target-version "$(sed -n 's/^jam_target_version = "\(.*\)"/\1/p' "${ROOT}/toolchains/distribution-v1.toml")" \
  --jam-blob-encoder-version "$(sed -n 's/^jam_blob_encoder_version = "\(.*\)"/\1/p' "${ROOT}/toolchains/distribution-v1.toml")" \
  --scriptc-revision "$(sed -n 's/^commit=//p' "${ROOT}/toolchains/scriptc/REVISION")"

find "${STAGE}" -type f -exec touch -d "@${SOURCE_DATE_EPOCH}" {} +
find "${STAGE}" -type d -exec touch -d "@${SOURCE_DATE_EPOCH}" {} +
ARCHIVE="${OUT}/jamscript-toolchain-scriptc-m2-v1-linux-x86_64.tar.zst"
(cd "${STAGE}" && tar --sort=name --numeric-owner --owner=0 --group=0 --mtime="@${SOURCE_DATE_EPOCH}" --zstd -cf "${ARCHIVE}" .)
sha256sum "${ARCHIVE}"
stat -c '%s' "${ARCHIVE}"
cp -L "${STAGE}/manifest.json" "${OUT}/toolchain-manifest.json"
python3 "${ROOT}/tools/release/toolchain/write-bundle-metadata.py" \
  --output "${OUT}/bundle-metadata.json" \
  --toolchain-id scriptc-m2-v1 --platform linux-x86_64 \
  --archive "$(basename -- "${ARCHIVE}")" \
  --source-revision "$(git -C "${ROOT}" rev-parse HEAD)"
echo "BUNDLE_PATH=${ARCHIVE}"
echo "BUNDLE_METADATA=${OUT}/bundle-metadata.json"
