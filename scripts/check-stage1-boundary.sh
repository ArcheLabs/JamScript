#!/usr/bin/env bash
set -euo pipefail
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
if rg -n 'playground\.minijam\.xyz|VITE_PLAYGROUND|PLAYGROUND_API_URL|/api/v1/build' \
  "$root/crates" "$root/packages" "$root/tools" "$root/.github"; then
  echo "JamScript production path contains a Playground dependency" >&2
  exit 1
fi

if rg -n '/api/v1/services|/api/v1/actions/prepare|/api/v1/operations|provision-service' \
  "$root/scripts/minijam-network-e2e.sh"; then
  echo "Canonical MiniJAM E2E still uses the legacy Playground lifecycle" >&2
  exit 1
fi
