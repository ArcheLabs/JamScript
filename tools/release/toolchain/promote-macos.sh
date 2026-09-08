#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../../.." && pwd -P)"
exec "${ROOT}/tools/release/toolchain/promote-platform.sh" macos-arm64 "$@"
