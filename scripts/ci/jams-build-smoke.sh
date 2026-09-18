#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd -P)"
JAMS="${JAMSCRIPT_CLI:-${ROOT}/target/release/jams}"
WORK="${RUNNER_TEMP:-${TMPDIR:-/tmp}}/jams-build-smoke"
FIXTURE="${WORK}/fixture"
OUTPUT="${WORK}/out"

test -x "${JAMS}"
rm -rf -- "${WORK}"
mkdir -p "${FIXTURE}/.jamscript"

cat > "${FIXTURE}/hello.ts" <<'EOF'
import { action, publicAction, u64 } from "jam";

export const hello = action({
  auth: publicAction(),
  input: { value: u64 },
  execute(_ctx, _input) {},
});
EOF

cat > "${FIXTURE}/jamscript.toml" <<'EOF'
[package]
name = "ci-jams-build-smoke"
version = "0.1.0"
entry = "hello.ts"
language = "0.3"

[compiler]
backend = "scriptc"

[management]
mode = "immutable"
EOF

cat > "${FIXTURE}/.jamscript/service.json" <<'EOF'
{
  "version": 2,
  "serviceKey": "0x4444444444444444444444444444444444444444444444444444444444444444",
  "instanceId": "0x5555555555555555555555555555555555555555555555555555555555555555",
  "name": "ci-jams-build-smoke"
}
EOF

if [[ "$(uname -s)" == Darwin ]]; then
  unset SDKROOT
fi

export JAMSCRIPT_DEV_TOOLCHAIN=1
export SCRIPTC_CC=clang
test -x "${JAMSCRIPT_CLANG:?JAMSCRIPT_CLANG must be set by the canonical host setup}"
export PATH="$(dirname -- "${JAMSCRIPT_CLANG}"):${PATH}"
"${JAMS}" build "${FIXTURE}" --offline --output "${OUTPUT}"

test -s "${OUTPUT}/service.pvm"
test -s "${OUTPUT}/service.polkavm"
test -s "${OUTPUT}/service.blob"

if [[ "$(uname -s)" == Darwin ]]; then
  echo "MACOS_APPLE_SDK_DISCOVERY=PASS"
  echo "MACOS_SCRIPTC_RUNTIME_HEADERS=PASS"
  echo "MACOS_JAMS_BUILD_SMOKE=PASS"
else
  echo "LINUX_JAMS_BUILD_SMOKE=PASS"
fi
