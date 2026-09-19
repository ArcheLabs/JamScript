#!/usr/bin/env python3
"""Static invariants for the minimal four-job JamScript release producer."""

import re
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
WORKFLOWS = ROOT / ".github/workflows"
release_path = WORKFLOWS / "release.yml"
maintenance_path = WORKFLOWS / "toolchain-maintenance.yml"
release = release_path.read_text(encoding="utf-8")
maintenance = maintenance_path.read_text(encoding="utf-8")


if not release_path.is_file() or not maintenance_path.is_file():
    raise SystemExit("release and maintenance workflows are required")
if re.search(r"(?m)^  (?:push|pull_request|schedule):", release):
    raise SystemExit("release workflow must be manual-only")
if "workflow_dispatch:" not in release or "inputs:\n      version:" not in release:
    raise SystemExit("release workflow must expose a version-only workflow_dispatch input")
if "test \"${GITHUB_REF_NAME}\" = main" not in release:
    raise SystemExit("release must be started from main")
if 'ref: ${{ github.sha }}' not in release or 'test "${source_sha}" = "${GITHUB_SHA}"' not in release:
    raise SystemExit("release validate job must bind SOURCE_SHA to github.sha")
if "validate-release-version.sh" not in release:
    raise SystemExit("release must use the generic workspace-version validator")
if "git ls-remote origin \"refs/tags/${VERSION}\"" not in release:
    raise SystemExit("release must verify the tag identity before publication")
if "gh release view \"${VERSION}\"" not in release:
    raise SystemExit("release must check GitHub Release identity")
if "actions: read" not in release or "gh run list" not in release:
    raise SystemExit("release validate job must require a successful source CI run")
if "SOURCE_CI_STATUS=PASS" not in release:
    raise SystemExit("release must report the source CI status")
if "minijam-network-e2e.yml" not in release or "JAMSCRIPT_CANONICAL_MINIJAM_E2E=PASS" not in release:
    raise SystemExit("release must require a successful canonical MiniJAM consumer E2E")

jobs = set(re.findall(r"(?m)^  ([a-z][a-z0-9-]+):\n", release.split("jobs:\n", 1)[1]))
if jobs != {"validate", "build-toolchain", "build-cli", "publish"}:
    raise SystemExit(f"release job graph is not minimal: {sorted(jobs)}")

for platform in ("linux-x86_64", "macos-arm64"):
    if f"platform: {platform}" not in release:
        raise SystemExit(f"release is missing platform {platform}")
for required in (
    "setup-host-build-env-linux.sh",
    "setup-host-build-env-macos.sh",
    "build-linux.sh",
    "build-macos.sh",
    "build-cli-archive.sh",
    "make-candidate-manifest.py",
    "npm ci --ignore-scripts",
    "zstd -q -t",
    "tar -tf -",
    "tar -tzf",
    "--version",
    "--help",
    "LICENSE",
    "README.md",
    "jamscript-service-*",
    "for platform in linux-x86_64 macos-arm64",
    "SHA256SUMS",
    "JAMSCRIPT_RELEASE_PUBLISHED=PASS",
):
    if required not in release:
        raise SystemExit(f"release is missing required producer behavior: {required}")

for forbidden in (
    "prepublish-consumer",
    "published-consumer",
    "cross-host-compare",
    "release-ready",
    "release-kill-test",
    "compiler-builtins-regression",
    "verify-execution-closure",
    "write-release-manifest.py",
    "release-manifest.json",
    "toolchain-manifest",
    "bundle-metadata",
    "TOOLCHAIN_AB_REPRODUCIBILITY",
    "CROSS_HOST_CANONICAL_ARTIFACTS",
    "JAMSCRIPT_RELEASE_READY",
    "build-backend-artifact.sh",
    "jamscript-service-backend",
    "Dockerfile.backend",
    "docker",
    "backend",
    "ghcr.io",
    "packages: write",
):
    if forbidden.lower() in release.lower():
        raise SystemExit(
            "minimal JamScript release contains forbidden late gate or backend ownership: "
            + forbidden
        )

if "for suffix in a b" in release or "cmp -s" in release:
    raise SystemExit("release must build each toolchain only once")
if "actual_count" not in release or "-eq 4" not in release or "-eq 5" not in release:
    raise SystemExit("publish must enforce the four-input/five-public-asset contract")
if 'git tag -a "${VERSION}" "${SOURCE_SHA}"' not in release:
    raise SystemExit("release must create the exact source tag")
if "environment: release" not in release:
    raise SystemExit("publish must use the release environment")
if "contents: write" not in release or "packages: write" in release:
    raise SystemExit("publish must own contents write permission only")
if "git tag" in maintenance or "git push" in maintenance or "contents: write" in maintenance:
    raise SystemExit("toolchain maintenance must not mutate repository refs")
if "MACOS_LLVM_NATIVE_MEASUREMENT=PASS" not in maintenance:
    raise SystemExit("toolchain maintenance is missing the measurement marker")

print("RELEASE_JOB_GRAPH=validate,build-toolchain,build-cli,publish")
print("RELEASE_PIPELINE_POLICY=PASS")
