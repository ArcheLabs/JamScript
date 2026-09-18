#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd -P)"
OUT="${1:?usage: build-backend-artifact.sh OUTPUT VERSION}"
VERSION="${2:?usage: build-backend-artifact.sh OUTPUT VERSION}"
TARGET="${JAMSCRIPT_BACKEND_TARGET:-$(uname -s | tr '[:upper:]' '[:lower:]')-$(uname -m)}"
VERSION_LABEL="${VERSION#backend-}"
ARCHIVE="jamscript-backend-${VERSION_LABEL}-${TARGET}.tar.gz"
STAGE="${OUT}/stage"

rm -rf -- "${STAGE}"
mkdir -p "${STAGE}/bin" "${OUT}"
(cd "${ROOT}" && cargo build --locked --release --bin jamscript-service-backend)
cp -L "${ROOT}/target/release/jamscript-service-backend" "${STAGE}/bin/"
cp -L "${ROOT}/Dockerfile.backend" "${ROOT}/docker-compose.backend.yml" "${STAGE}/"
cp -L "${ROOT}/docs/service-backend.md" "${ROOT}/docs/service-backend-v1.md" "${STAGE}/"

SOURCE_DATE_EPOCH="${SOURCE_DATE_EPOCH:-$(cd "${ROOT}" && git show -s --format=%ct HEAD)}"
python3 "${ROOT}/tools/release/toolchain/create-deterministic-archive.py" \
  --root "${STAGE}" --source-date-epoch "${SOURCE_DATE_EPOCH}" | \
  gzip -n -c > "${OUT}/${ARCHIVE}"
if command -v sha256sum >/dev/null 2>&1; then
  (cd "${OUT}" && sha256sum "${ARCHIVE}" > "${ARCHIVE}.sha256")
else
  (cd "${OUT}" && shasum -a 256 "${ARCHIVE}" > "${ARCHIVE}.sha256")
fi
printf '%s\n' "${OUT}/${ARCHIVE}"
