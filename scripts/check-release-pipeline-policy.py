#!/usr/bin/env python3
"""Static invariants for the release source gate and manual preflight."""

import re
from pathlib import Path
from typing import Set


ROOT = Path(__file__).resolve().parents[1]
WORKFLOWS = ROOT / ".github/workflows"
SOURCE_GATE = re.compile(
    r"\.\/scripts\/release\/check-release-source\.sh\s+\\?\s*\n?\s*--platform\s+([a-z0-9_-]+)"
)


def text(name: str) -> str:
    path = WORKFLOWS / name
    if not path.is_file():
        raise SystemExit(f"missing release workflow: {path}")
    return path.read_text(encoding="utf-8")


def require_source_gates(name: str, expected: Set[str]) -> str:
    workflow = text(name)
    platforms = set(SOURCE_GATE.findall(workflow))
    if platforms != expected:
        raise SystemExit(
            f"{name} must use the shared release source gate for {sorted(expected)}; "
            f"found {sorted(platforms)}"
        )
    return workflow


def workflow_trigger(workflow: str) -> str:
    match = re.search(r"(?ms)^on:\n(.*?)(?=^\S|\Z)", workflow)
    if not match:
        raise SystemExit("workflow has no top-level on block")
    return match.group(1)


build = require_source_gates("build-toolchain-bundle.yml", {"linux-x86_64", "macos-arm64"})
release = require_source_gates("release-candidate.yml", {"linux-x86_64", "macos-arm64"})
preflight = require_source_gates("release-preflight.yml", {"linux-x86_64", "macos-arm64"})
promotion = text("promote-macos-llvm-lock.yml")

for name, workflow in (
    ("build-toolchain-bundle.yml", build),
    ("release-candidate.yml", release),
    ("release-preflight.yml", preflight),
):
    if "cargo test --locked -p jamscript-toolchain" in workflow:
        raise SystemExit(f"{name} duplicates the shared toolchain source gate")
    if "cargo clippy --locked -p jamscript-toolchain" in workflow:
        raise SystemExit(f"{name} duplicates the shared clippy source gate")


def assert_linux_env_scope(name: str, workflow: str) -> None:
    source = re.search(r"source \"\$\{[^}]+\}/llvm\.env\"", workflow)
    verify = re.search(r"verify-llvm-linux\.sh \"\$\{LLVM_ROOT\}\"", workflow)
    publish = re.search(r"cat \"\$\{[^}]+\}/llvm\.env\" >> \"\$\{GITHUB_ENV\}\"", workflow)
    if not source or not verify or not publish or not (source.start() < verify.start() < publish.start()):
        raise SystemExit(f"{name} must source llvm.env, verify it, then publish GITHUB_ENV")


assert_linux_env_scope("build-toolchain-bundle.yml", build)
assert_linux_env_scope("release-candidate.yml", release)
assert_linux_env_scope("release-preflight.yml", preflight)

trigger = workflow_trigger(preflight)
if re.findall(r"(?m)^  ([A-Za-z_][A-Za-z0-9_-]*):", trigger) != ["workflow_dispatch"]:
    raise SystemExit("release-preflight.yml must be manual-only")
for field in ("ref:", "version:"):
    if not re.search(rf"(?m)^      {re.escape(field)}$", trigger):
        raise SystemExit(f"release-preflight.yml is missing workflow input {field[:-1]}")
if "ref: ${{ inputs.ref }}" not in preflight:
    raise SystemExit("release preflight must checkout the exact input ref")
if "contents: write" in preflight:
    raise SystemExit("release preflight must not have contents: write")
if re.search(r"(?m)\bgit\s+(?:tag|push)\b", preflight):
    raise SystemExit("release preflight must not mutate git refs")
if re.search(r"(?m)\bgh\s+release\s+(?:create|upload)\b", preflight):
    raise SystemExit("release preflight must not publish a GitHub Release")
if "RELEASE_PREFLIGHT_READY=PASS" not in preflight:
    raise SystemExit("release preflight is missing its final readiness marker")

promotion_trigger = workflow_trigger(promotion)
if re.findall(r"(?m)^  ([A-Za-z_][A-Za-z0-9_-]*):", promotion_trigger) != ["workflow_dispatch"]:
    raise SystemExit("macOS LLVM promotion workflow must be manual-only")
if "contents: write" in promotion or re.search(r"(?m)\bgit\s+(?:tag|push)\b", promotion):
    raise SystemExit("macOS LLVM promotion workflow must not mutate repository refs")
if "MACOS_LLVM_NATIVE_MEASUREMENT=PASS" not in promotion:
    raise SystemExit("macOS LLVM promotion workflow is missing its measurement marker")

if "concurrency:" not in release or "cancel-in-progress: false" not in release:
    raise SystemExit("release-candidate must serialize a tag's publication attempt")
if "concurrency:" not in preflight or "cancel-in-progress: false" not in preflight:
    raise SystemExit("release-preflight must serialize a candidate preflight")

print("RELEASE_PIPELINE_POLICY=PASS")
