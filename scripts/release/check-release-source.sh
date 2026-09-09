#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd -P)"
PLATFORM=""

usage() {
  echo "usage: check-release-source.sh --platform linux-x86_64|macos-arm64" >&2
  exit 2
}

while (($#)); do
  case "$1" in
    --platform)
      PLATFORM="${2:?missing platform}"
      shift 2
      ;;
    -h|--help)
      usage
      ;;
    *)
      usage
      ;;
  esac
done

case "${PLATFORM}" in
  linux-x86_64)
    [[ "$(uname -s):$(uname -m)" == "Linux:x86_64" ]] || {
      echo "release source check requires a native Linux x86_64 runner" >&2
      exit 1
    }
    ;;
  macos-arm64)
    [[ "$(uname -s):$(uname -m)" == "Darwin:arm64" ]] || {
      echo "release source check requires a native macOS arm64 runner" >&2
      exit 1
    }
    ;;
  *)
    usage
    ;;
esac

cd "${ROOT}"
if ! python3 tools/release/toolchain/check-llvm-lock-promoted.py \
  "toolchains/llvm/${PLATFORM}.lock" --platform "${PLATFORM}"; then
  exit 1
fi

cargo test --locked -p jamscript-toolchain
cargo test --locked -p jamscript-cli
cargo clippy --locked -p jamscript-toolchain -- -D warnings
./scripts/check-toolchain-distribution.sh
python3 ./tools/release/toolchain/test-llvm-lock.py
echo "RELEASE_SOURCE_CHECK=PASS"
