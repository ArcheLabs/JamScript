#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../../.." && pwd -P)"
OUT="${1:-${ROOT}/dist/toolchain}"
SOURCE_DATE_EPOCH="${SOURCE_DATE_EPOCH:-$(git -C "${ROOT}" log -1 --format=%ct)}"
NODE_BIN="${SCRIPTC_NODE:?set SCRIPTC_NODE to the exact Node binary}"
LLVM_LOCK="${ROOT}/toolchains/llvm/linux-x86_64.lock"
LLVM_LOCK_PARSER="${ROOT}/tools/release/toolchain/llvm-lock.py"
SCRIPTC_LINUX_RANDOM="${ROOT}/toolchains/scriptc/compat/linux_random.c"
SCRIPTC_LINUX_PATCH="${ROOT}/toolchains/scriptc/patches/0001-linux-old-glibc-secure-random.patch"
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
copy_file "${LLD_BIN}" bin/guest-linker
copy_tree "${ROOT}/toolchains/scriptc" scriptc
copy_file "${SCRIPTC_LINUX_RANDOM}" scriptc/node_modules/@scriptc/runtime/src/scr_linux.c
test -f "${SCRIPTC_LINUX_PATCH}"
command -v patch >/dev/null 2>&1 || {
  echo "missing bundle input: patch" >&2
  exit 1
}
RUNTIME_SRC="${STAGE}/scriptc/node_modules/@scriptc/runtime/src"
COMPILER_CC="${STAGE}/scriptc/node_modules/@scriptc/compiler/dist/backend/cc.js"
grep -Fq 'if (b->len > 0) arc4random_buf(b->data, b->len);' "${RUNTIME_SRC}/scr_bytes_io.c" || {
  echo "ScriptC Linux compatibility patch precondition failed: scr_bytes_io.c" >&2
  exit 1
}
grep -Fq 'arc4random_buf(r, sizeof r);' "${RUNTIME_SRC}/scr_lib.c" || {
  echo "ScriptC Linux compatibility patch precondition failed: scr_lib.c" >&2
  exit 1
}
grep -Fq 'targetPlatform(driver) === "win32" ? ["scr_win.c"]' "${COMPILER_CC}" || {
  echo "ScriptC Linux compatibility patch precondition failed: cc.js" >&2
  exit 1
}
patch --batch --forward --fuzz=0 --strip=0 --directory="${STAGE}" < "${SCRIPTC_LINUX_PATCH}"
grep -Fq 'void jamscript_secure_random(void *buf, size_t n);' "${RUNTIME_SRC}/scr_runtime.h"
grep -Fq '"scr_linux.c"' "${COMPILER_CC}"
echo "SCRIPTC_LINUX_COMPAT_PATCH=PASS"
copy_tree "${ROOT}/crates/jamscript-runtime-scriptc" runtime-scriptc
copy_tree "${ROOT}/crates/jamscript-target-jam/sdk" targets/jam/sdk

TARGET_JSON="$(RUSTC="${RUSTC_BIN}" JAMSCRIPT_CARGO="${CARGO_BIN}" \
  "${CARGO_BIN}" run --quiet --locked \
    --manifest-path "${ROOT}/tools/release/toolchain/Cargo.toml" \
    --bin polkavm-target-json)"
JAMSCRIPT_CARGO="${CARGO_BIN}" \
JAMSCRIPT_RUSTC="${RUSTC_BIN}" \
JAMSCRIPT_CLANG="${CLANG_BIN}" \
JAMSCRIPT_LLVM_AR="${LLVM_AR_BIN}" \
JAMSCRIPT_POLKAVM_TARGET_JSON="${TARGET_JSON}" \
  "${ROOT}/tools/release/toolchain/build-runtime-archives.sh" "${STAGE}"
copy_file "${TARGET_JSON}" targets/polkavm/riscv64emac-unknown-none-polkavm.json

LLVM_RESOURCE_DIR="$("${CLANG_BIN}" -print-resource-dir)"
case "${LLVM_RESOURCE_DIR}" in
  "${LLVM_ROOT}"/*) copy_tree "${LLVM_RESOURCE_DIR}" "${LLVM_RESOURCE_DIR#"${LLVM_ROOT}"/}" ;;
  *) echo "clang resource directory is outside the locked LLVM root: ${LLVM_RESOURCE_DIR}" >&2; exit 1 ;;
esac

declare -a dependency_queue=("${NODE_BIN}" "${CLANG_BIN}" "${LLVM_AR_BIN}" "${LLD_BIN}")
declare -A seen_dependencies=()
RUNTIME_LIBRARY_PATH="${LLVM_ROOT}/lib${LD_LIBRARY_PATH:+:${LD_LIBRARY_PATH}}"
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
for binary in bin/node; do
  if command -v patchelf >/dev/null 2>&1; then
    patchelf --set-rpath '$ORIGIN/../lib' "${STAGE}/${binary}"
  fi
done
if [[ "${JAMSCRIPT_TOOLCHAIN_RELEASE_ENGINEERING:-0}" != "1" && "$(command -v patchelf || true)" ]]; then
  for binary in bin/clang bin/llvm-ar bin/guest-linker; do
    patchelf --set-rpath '$ORIGIN/../lib' "${STAGE}/${binary}"
  done
fi

test "$("${NODE_BIN}" --version | tr -d '\r\n' | sed 's/^v//')" = "$(tr -d '\r\n' < "${ROOT}/toolchains/scriptc/NODE_VERSION")"
test "$("${CLANG_BIN}" --version | sed -n '1s/.*clang version \([0-9.]*\).*/\1/p')" = "${LLVM_VERSION}"
test -d "${STAGE}/targets/jam/sdk"

# The Rust/Cargo tree above exists only inside this release-engineering build.
# Consumer builds link the immutable archives and never receive the producer
# compiler, sysroot, vendor tree, host linker, or generated Rust guest.
rm -rf -- \
  "${STAGE}/cargo" \
  "${STAGE}/lib/rustlib" \
  "${STAGE}/runtime/crates" \
  "${STAGE}/runtime/Cargo.toml" \
  "${STAGE}/runtime/Cargo.lock" \
  "${STAGE}/toolchains/polkavm-guest" \
  "${STAGE}/bin/rustc" \
  "${STAGE}/bin/cargo" \
  "${STAGE}/bin/jamscript-host-linker" \
  "${STAGE}/bin/ar" \
  "${STAGE}/bin/llvm-readelf" \
  "${STAGE}/Cargo.lock" \
  "${STAGE}/toolchains/polkavm.lock"
rm -rf -- \
  "${STAGE}/runtime-scriptc/src" \
  "${STAGE}/runtime-scriptc/Cargo.toml" \
  "${STAGE}/targets/jam/sdk/src"
test ! -e "${STAGE}/bin/rustc"
test ! -e "${STAGE}/bin/cargo"
test ! -e "${STAGE}/cargo"
test ! -e "${STAGE}/lib/rustlib"
test ! -e "${STAGE}/toolchains/polkavm-guest"
echo "CONSUMER_RUST_REMOVED=PASS"

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

ARCHIVE="${OUT}/jamscript-toolchain-scriptc-m2-v1-linux-x86_64.tar.zst"
if command -v zstd >/dev/null 2>&1; then
  python3 "${ROOT}/tools/release/toolchain/create-deterministic-archive.py" \
    --root "${STAGE}" --source-date-epoch "${SOURCE_DATE_EPOCH}" | \
    zstd -q -T1 -19 -o "${ARCHIVE}"
else
  # The producer may run on a minimal development host without the zstd CLI.
  # The archive remains the same tar.zst format and the managed CLI decodes it
  # through its Rust implementation.
  python3 "${ROOT}/tools/release/toolchain/create-deterministic-archive.py" \
    --root "${STAGE}" --source-date-epoch "${SOURCE_DATE_EPOCH}" | \
    CARGO_TARGET_DIR="${OUT}/.cargo-target" RUSTC="${RUSTC_BIN}" \
      JAMSCRIPT_ZSTD_LEVEL="${JAMSCRIPT_ZSTD_LEVEL:-19}" "${CARGO_BIN}" run --quiet --locked \
      --manifest-path "${ROOT}/tools/release/toolchain/Cargo.toml" \
      --bin compress-zstd -- "${ARCHIVE}"
fi
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
