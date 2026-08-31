#!/usr/bin/env bash
set -euo pipefail
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
if rg -n 'playground\.minijam\.xyz|VITE_PLAYGROUND|PLAYGROUND_API_URL|/api/v1/build' \
  "$root/crates" "$root/packages" "$root/tools" "$root/.github"; then
  echo "JamScript production path contains a Playground dependency" >&2
  exit 1
fi

