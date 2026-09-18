#!/usr/bin/env python3
"""Static invariants for the single manual release producer."""

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
    "build-backend-artifact.sh",
    "verify-bundle.sh",
    "make-candidate-manifest.py",
):
    require(script, f"producer script {script}")
if release.count("for suffix in a b") < 1 or "cmp -s" not in release:
    raise SystemExit("release must build and compare toolchain A/B")
if "release-input-toolchain-" not in release:
    raise SystemExit("release must upload only the validated toolchain producer bytes")
if "--asset-dir" not in release or "CROSS_HOST_CANONICAL_ARTIFACTS=PASS" not in release:
    raise SystemExit("release is missing prepublish consumer or cross-host gates")
if "docker-prepublish-smoke:" not in release or "DOCKER_PREPUBLISH_SMOKE=PASS" not in release:
    raise SystemExit("release is missing the prepublish Docker smoke")
if "git tag -a \"${VERSION}\" \"${SOURCE_SHA}\"" not in release or "git push origin \"refs/tags/${VERSION}\"" not in release:
    raise SystemExit("release must create and push the exact source tag after prepublish gates")
if "environment: release" not in release:
    raise SystemExit("publish must use the release environment")
if "contents: write" not in release or "packages: write" not in release:
    raise SystemExit("publish must own the write permissions")
if "RELEASE_READY=PASS" not in release:
    raise SystemExit("release is missing RELEASE_READY=PASS")
if "git tag" in maintenance or "git push" in maintenance or "contents: write" in maintenance:
    raise SystemExit("toolchain maintenance must not mutate repository refs")
if "MACOS_LLVM_NATIVE_MEASUREMENT=PASS" not in maintenance:
    raise SystemExit("toolchain maintenance is missing the measurement marker")

print("RELEASE_PIPELINE_POLICY=PASS")
