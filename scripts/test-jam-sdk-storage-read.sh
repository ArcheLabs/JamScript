#!/usr/bin/env bash
set -euo pipefail

JAMSCRIPT_ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)"
SDK_ROOT="${JAMSCRIPT_ROOT}/crates/jamscript-target-jam/sdk"
TEST_SOURCE="${SDK_ROOT}/tests/minijam_storage_read_test.c"
CC_BIN="${JAMSCRIPT_CC:-${CC:-clang}}"
TEST_BUILD_DIR="$(mktemp -d)"
trap 'rm -rf -- "${TEST_BUILD_DIR}"' EXIT

"${CC_BIN}" \
  -std=c11 \
  -Wall \
  -Wextra \
  -Werror \
  -DMINIJAM_HOST_TEST \
  -I"${SDK_ROOT}/include" \
  "${TEST_SOURCE}" \
  "${SDK_ROOT}/src/minijam.c" \
  -o "${TEST_BUILD_DIR}/minijam-storage-read-test"

"${TEST_BUILD_DIR}/minijam-storage-read-test"
echo "JAM_SDK_STORAGE_READ_SENTINEL=PASS"
