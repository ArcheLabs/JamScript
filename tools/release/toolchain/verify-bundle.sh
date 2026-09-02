#!/usr/bin/env bash
set -euo pipefail
archive="${1:?usage: verify-bundle.sh bundle.tar.zst}"
tar --zstd -tf "${archive}" | while IFS= read -r entry; do
  case "${entry}" in
    /*|../*|*/../*|*\\*) echo "unsafe archive path: ${entry}" >&2; exit 1 ;;
  esac
done
if tar --zstd -tvf "${archive}" | grep -Eq '^[lh]'; then
  echo "symlinks and hard links are not allowed in toolchain bundles" >&2
  exit 1
fi
echo "TOOLCHAIN_BUNDLE_INTEGRITY=PASS"
