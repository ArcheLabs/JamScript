#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../../.." && pwd -P)"
ARCHIVE="${1:?usage: consumer-smoke.sh bundle.tar.zst fresh-install-home output-dir}"
INSTALL_HOME="${2:?usage: consumer-smoke.sh bundle.tar.zst fresh-install-home output-dir}"
OUTPUT="${3:?usage: consumer-smoke.sh bundle.tar.zst fresh-install-home output-dir}"
SOURCE_COPY="${RUNNER_TEMP:-${TMPDIR:-/tmp}}/jamscript-consumer-source"
BUNDLE_SHA256="$(sha256sum "${ARCHIVE}" | awk '{print $1}')"
BUNDLE_SIZE="$(stat -c '%s' "${ARCHIVE}")"

rm -rf -- "${SOURCE_COPY}"
mkdir -p "${SOURCE_COPY}" "${INSTALL_HOME}" "${OUTPUT}"
git -C "${ROOT}" archive --format=tar HEAD | tar -xf - -C "${SOURCE_COPY}"
python3 "${ROOT}/tools/release/toolchain/make-candidate-manifest.py" \
  --input "${SOURCE_COPY}/toolchains/distribution-v1.toml" \
  --output "${SOURCE_COPY}/toolchains/distribution-v1.toml" \
  --url "file://${ARCHIVE}" --sha256 "${BUNDLE_SHA256}" --size "${BUNDLE_SIZE}"

cargo build --locked --manifest-path "${SOURCE_COPY}/Cargo.toml" --bin jamscript
export JAMSCRIPT_TOOLCHAIN_HOME="${INSTALL_HOME}"
unset JAMSCRIPT_OFFLINE JAMSCRIPT_DEV_TOOLCHAIN JAMSCRIPT_MINIJAM_SDK
"${SOURCE_COPY}/target/debug/jamscript" toolchain install
export JAMSCRIPT_OFFLINE=1
"${SOURCE_COPY}/target/debug/jamscript" build "${SOURCE_COPY}/examples/counter" \
  --offline --output "${OUTPUT}"

test -s "${OUTPUT}/build.json"
rg -q '"canonical_toolchain": true' "${OUTPUT}/build.json"
rg -q "\"jamscript_toolchain_sha256\": \"${BUNDLE_SHA256}\"" "${OUTPUT}/build.json"
for artifact in service.blob service.polkavm service.pvm service.abi.json; do
  test -s "${OUTPUT}/${artifact}"
done
echo "TOOLCHAIN_LOCAL_INSTALL=PASS"
echo "OFFLINE_CONSUMER_BUILD=PASS"
