#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
minijam_root="${JAMSCRIPT_MINIJAM_SDK:-$root/../minijam-client}"
patch_file="$root/patches/minijam-client/refine-current-item.patch"
jambda_root="$minijam_root/external/jambda"

if git -C "$jambda_root" apply --check "$patch_file" >/dev/null 2>&1; then
  git -C "$jambda_root" apply "$patch_file"
fi
