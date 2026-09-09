#!/usr/bin/env python3
import re
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
BUILD_WORKFLOW = ROOT / ".github/workflows/build-toolchain-bundle.yml"
RELEASE_WORKFLOW = ROOT / ".github/workflows/release-candidate.yml"

EXPECTED_PATHS = {
    ".github/workflows/build-toolchain-bundle.yml",
    "Cargo.lock",
    "Cargo.toml",
    "rust-toolchain.toml",
    "crates/**",
    "scripts/release/**",
    "scripts/check-toolchain-distribution.sh",
    "toolchains/**",
    "tools/release/**",
}


def trigger_block(path):
    text = path.read_text(encoding="utf-8")
    match = re.search(r"(?ms)^on:\n(.*?)(?=^\S|\Z)", text)
    if not match:
        raise SystemExit(f"workflow has no top-level on block: {path}")
    return match.group(1)


def event_paths(trigger, event):
    match = re.search(rf"(?ms)^  {re.escape(event)}:\n(.*?)(?=^  \S|\Z)", trigger)
    if not match:
        raise SystemExit(f"workflow trigger is missing {event}")
    section = match.group(1)
    paths = re.search(r"(?ms)^    paths:\n(.*?)(?=^    \S|\Z)", section)
    if not paths:
        raise SystemExit(f"{event} trigger has no paths filter")
    return {
        value
        for value in re.findall(r"^      - '([^']+)'$", paths.group(1), re.MULTILINE)
    }


build_trigger = trigger_block(BUILD_WORKFLOW)
if "  workflow_dispatch:\n" not in build_trigger:
    raise SystemExit("toolchain bundle workflow must keep workflow_dispatch")
push_paths = event_paths(build_trigger, "push")
pull_request_paths = event_paths(build_trigger, "pull_request")
if push_paths != EXPECTED_PATHS or pull_request_paths != EXPECTED_PATHS:
    raise SystemExit(
        "toolchain bundle push/pull_request paths do not match the policy: "
        f"push={sorted(push_paths)} pull_request={sorted(pull_request_paths)}"
    )

release_trigger = trigger_block(RELEASE_WORKFLOW)
if re.search(r"(?m)^    paths:", release_trigger):
    raise SystemExit("release-candidate workflow must not use a path filter")
if '      - "v0.1.0-rc.*"' not in release_trigger:
    raise SystemExit("release-candidate workflow lost the RC tag trigger")
if '      - "v0.1.0"' not in release_trigger:
    raise SystemExit("release-candidate workflow lost the stable tag trigger")

print("CI_TRIGGER_POLICY=PASS")
