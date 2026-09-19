#!/usr/bin/env bash
set -euo pipefail

# Keep the public release gate in the existing consumer harness so the release
# graph stays minimal while Ownership gets the same managed-only validation.
script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
exec "${script_dir}/release-kill-test-001.sh" "$@"
