#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)"
mapfile -t SETUP_FILES < <(find "${ROOT}/tools/release" -maxdepth 1 -type f -name 'setup-host-build-env-*.sh' -print | sort)
test "${#SETUP_FILES[@]}" -gt 0
WORKFLOW_FILES=(
  "${ROOT}/.github/workflows/ci.yml"
  "${ROOT}/.github/workflows/release.yml"
)

for file in "${SETUP_FILES[@]}"; do
  test -f "${file}"
  if rg -n '(^|[[:space:]])(LD_LIBRARY_PATH|DYLD_LIBRARY_PATH|DYLD_FALLBACK_LIBRARY_PATH)=' "${file}"; then
    echo "GLOBAL_DYNAMIC_LOADER_OVERRIDE=FAIL (${file})" >&2
    exit 1
  fi
done

for file in "${WORKFLOW_FILES[@]}"; do
  test -f "${file}"
  if rg -n 'cat .*host\.env.*GITHUB_ENV|GITHUB_ENV.*host\.env' "${file}"; then
    echo "HOST_ENVIRONMENT_PERSISTENCE=FAIL (${file})" >&2
    exit 1
  fi
  if ! rg -q 'source "\$\{RUNNER_TEMP\}/host\.env"' "${file}"; then
    echo "HOST_ENVIRONMENT_SOURCE=FAIL (${file})" >&2
    exit 1
  fi
  if ! rg -q 'JAMSCRIPT_HOST_RPATH_FLAG' "${file}"; then
    echo "HOST_BUILD_RPATH=FAIL (${file})" >&2
    exit 1
  fi
done

echo "GLOBAL_DYNAMIC_LOADER_OVERRIDE=NO"
echo "GLOBAL_LD_LIBRARY_PATH=ABSENT"
echo "GLOBAL_DYLD_LIBRARY_PATH=ABSENT"
echo "GLOBAL_DYLD_FALLBACK_LIBRARY_PATH=ABSENT"
echo "HOST_ENVIRONMENT_ISOLATED=YES"
echo "HOST_BUILD_RPATH=SCOPED"
