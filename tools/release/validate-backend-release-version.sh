#!/usr/bin/env bash
set -euo pipefail

VERSION="${1:?usage: validate-backend-release-version.sh BACKEND_VERSION}"
[[ "${VERSION}" =~ ^backend-v[0-9]+\.[0-9]+\.[0-9]+(-rc\.[0-9]+)?$ ]] || {
  echo "invalid backend release version: ${VERSION}" >&2
  exit 1
}
echo "BACKEND_RELEASE_VERSION=PASS ${VERSION}"
