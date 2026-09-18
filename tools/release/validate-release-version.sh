#!/usr/bin/env bash
set -euo pipefail

VERSION="${1:?usage: validate-release-version.sh VERSION}"
[[ "${VERSION}" =~ ^v[0-9]+\.[0-9]+\.[0-9]+(-rc\.[0-9]+)?$ ]] || {
  echo "invalid release version: ${VERSION}" >&2
  exit 1
}
BASE_VERSION="${VERSION%%-rc.*}"
WORKSPACE_VERSION="$(sed -n '/^\[workspace\.package\]/,/^\[/ { s/^version = "\([^"]*\)"/\1/p; }' Cargo.toml | head -n 1)"
test -n "${WORKSPACE_VERSION}"
test "${BASE_VERSION}" = "v${WORKSPACE_VERSION}" || {
  echo "release version ${VERSION} does not match workspace package v${WORKSPACE_VERSION}" >&2
  exit 1
}
echo "RELEASE_VERSION=PASS ${VERSION}"
