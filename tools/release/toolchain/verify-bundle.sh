#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../../.." && pwd -P)"
ARCHIVE="${1:?usage: verify-bundle.sh bundle.tar.zst [fresh-directory] [root-output-file]}"
EXTRACT_DIR="${2:-$(mktemp -d "${RUNNER_TEMP:-${TMPDIR:-/tmp}}/toolchain-verify.XXXXXX")}"
ROOT_OUTPUT="${3:-}"
mkdir -p "${EXTRACT_DIR}"

while IFS= read -r entry; do
  case "${entry}" in
    /*|../*|*/../*|*\\*)
      echo "unsafe archive path: ${entry}" >&2
      exit 1
      ;;
  esac
done < <(tar --zstd -tf "${ARCHIVE}")
if tar --zstd -tvf "${ARCHIVE}" | grep -Eq '^[lh]'; then
  echo "symlinks and hard links are not allowed in toolchain bundles" >&2
  exit 1
fi

# This helper calls ToolchainManager::install with a temporary candidate
# manifest. The manager checks the archive SHA/size and performs the safe
# tar.zst extraction itself; this is deliberately not a second shell unpacker.
CACHE_DIR="${EXTRACT_DIR}/manager-cache"
mkdir -p "${CACHE_DIR}"
INSTALLED_ROOT="$(cargo run --quiet --locked \
  --manifest-path "${ROOT}/tools/release/toolchain/Cargo.toml" -- \
  "${ARCHIVE}" "${CACHE_DIR}" "${ROOT}/toolchains/distribution-v1.toml")"
test -d "${INSTALLED_ROOT}"
python3 "${ROOT}/tools/release/toolchain/verify-bundle.py" \
  --root "${INSTALLED_ROOT}" --source-root "${ROOT}"
if [[ -n "${ROOT_OUTPUT}" ]]; then
  printf '%s\n' "${INSTALLED_ROOT}" > "${ROOT_OUTPUT}"
fi
echo "TOOLCHAIN_BUNDLE_INTEGRITY=PASS"
