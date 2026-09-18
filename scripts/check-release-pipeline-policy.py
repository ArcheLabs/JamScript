#!/usr/bin/env python3
"""Static invariants for the JamScript-only manual release producer."""

import re
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
WORKFLOWS = ROOT / ".github/workflows"
release_path = WORKFLOWS / "release.yml"
maintenance_path = WORKFLOWS / "toolchain-maintenance.yml"
release = release_path.read_text(encoding="utf-8")
maintenance = maintenance_path.read_text(encoding="utf-8")


def require(needle: str, description: str) -> None:
    if needle not in release:
        raise SystemExit(f"release policy missing {description}: {needle}")


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

for platform in ("linux-x86_64", "macos-arm64"):
    if f"platform: {platform}" not in release:
        raise SystemExit(f"release is missing platform {platform}")
for script in (
    "setup-host-build-env-linux.sh",
    "setup-host-build-env-macos.sh",
    "build-cli-archive.sh",
    "verify-bundle.sh",
    "make-candidate-manifest.py",
    "write-release-manifest.py",
):
    require(script, f"producer script {script}")
if release.count("for suffix in a b") < 1 or "cmp -s" not in release:
    raise SystemExit("release must build and compare toolchain A/B")
if "release-input-toolchain-" not in release:
    raise SystemExit("release must upload only the validated toolchain producer bytes")
for required_gate in (
    "prepublish-consumer:",
    "published-consumer:",
    "cross-host-compare:",
    "release-kill-test-001.sh",
    "--asset-dir",
    "--release-url",
    "CROSS_HOST_CANONICAL_ARTIFACTS=PASS",
    "JAMSCRIPT_RELEASE_READY=PASS",
):
    if required_gate not in release:
        raise SystemExit(f"release is missing JamScript consumer gate: {required_gate}")
for forbidden in (
    "build-backend-artifact.sh",
    "jamscript-service-backend",
    "Dockerfile.backend",
    "docker-prepublish",
    "backend-image-test",
    "ghcr.io/archelabs/jamscript-backend",
    "packages: write",
    "--backend",
):
    if forbidden in release:
        raise SystemExit(f"JamScript release must not own backend lifecycle: {forbidden}")
if "write-release-manifest.py" not in release:
    raise SystemExit("JamScript release must write its CLI/toolchain manifest")
if "git tag -a \"${VERSION}\" \"${SOURCE_SHA}\"" not in release or "git push origin \"refs/tags/${VERSION}\"" not in release:
    raise SystemExit("release must create and push the exact source tag after prepublish gates")
if "environment: release" not in release:
    raise SystemExit("publish must use the release environment")
if "contents: write" not in release or "packages: write" in release:
    raise SystemExit("publish must own the write permissions")
if "git tag" in maintenance or "git push" in maintenance or "contents: write" in maintenance:
    raise SystemExit("toolchain maintenance must not mutate repository refs")
if "MACOS_LLVM_NATIVE_MEASUREMENT=PASS" not in maintenance:
    raise SystemExit("toolchain maintenance is missing the measurement marker")

print("RELEASE_PIPELINE_POLICY=PASS")
