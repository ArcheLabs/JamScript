#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)"
JAMS="${JAMSCRIPT_TEST_CLI:-${ROOT}/target/release/jams}"
RUN_ROOT="$(mktemp -d "${TMPDIR:-/tmp}/jamscript-guest-memory.XXXXXX")"
trap 'rm -rf -- "${RUN_ROOT}"' EXIT

[[ -x "${JAMS}" ]] || {
  echo "build the JamScript CLI first or set JAMSCRIPT_TEST_CLI" >&2
  exit 2
}
[[ "${JAMSCRIPT_DEV_TOOLCHAIN:-}" == "1" ]] || {
  echo "this gate must use the contributor ScriptC toolchain" >&2
  exit 2
}

default_output="${RUN_ROOT}/default"
limited_project="${RUN_ROOT}/dynamic-state-scriptc-512k"
limited_output="${RUN_ROOT}/limited"

"${JAMS}" build "${ROOT}/examples/dynamic-state-scriptc" --offline --output "${default_output}"
cp -R "${ROOT}/examples/dynamic-state-scriptc" "${limited_project}"
cat >> "${limited_project}/jamscript.toml" <<'EOF'

[guest.memory]
heap_initial_bytes = 524288
heap_max_bytes = 524288
EOF
"${JAMS}" build "${limited_project}" --offline --output "${limited_output}"

test -s "${default_output}/service.polkavm"
test -s "${limited_output}/service.polkavm"
grep -q '"heapMaxBytes": 16777216' "${default_output}/build.json"
grep -q '"heapMaxBytes": 524288' "${limited_output}/build.json"

run_with_libclang_path() {
  if [[ -n "${LIBCLANG_PATH:-}" ]]; then
    LD_LIBRARY_PATH="${LIBCLANG_PATH}${LD_LIBRARY_PATH:+:${LD_LIBRARY_PATH}}" "$@"
  else
    "$@"
  fi
}

JAMSCRIPT_GUEST_MEMORY_TEST_ARTIFACT="${default_output}/service.polkavm" \
JAMSCRIPT_GUEST_MEMORY_LIMIT_TEST_ARTIFACT="${limited_output}/service.polkavm" \
run_with_libclang_path cargo test --locked --release -p jamscript-service-backend --features pvm-memory-tests actual_pvm_guest
run_with_libclang_path cargo clippy --locked -p jamscript-service-backend --release --features pvm-memory-tests --tests -- -D warnings

echo "GUEST_MEMORY_ACTUAL_PVM_REGRESSION=PASS"
