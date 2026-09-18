#!/usr/bin/env python3
"""Check the repository's five-workflow CI/release trigger contract."""

import re
from pathlib import Path
from typing import List


ROOT = Path(__file__).resolve().parents[1]
WORKFLOWS = ROOT / ".github/workflows"
EXPECTED = {
    "ci.yml",
    "release.yml",
    "backend-release.yml",
    "minijam-network-e2e.yml",
    "toolchain-maintenance.yml",
}


actual = {path.name for path in WORKFLOWS.glob("*.yml")}
if actual != EXPECTED:
    raise SystemExit(f"ACTIVE_WORKFLOW_COUNT=FAIL expected={sorted(EXPECTED)} actual={sorted(actual)}")


def trigger_events(path: Path) -> List[str]:
    text = path.read_text(encoding="utf-8")
    match = re.search(r"(?ms)^on:\n(.*?)(?=^\S|\Z)", text)
    if not match:
        raise SystemExit(f"workflow has no top-level on block: {path}")
    return re.findall(r"(?m)^  ([A-Za-z_][A-Za-z0-9_-]*):", match.group(1))


if trigger_events(WORKFLOWS / "ci.yml") != ["push", "pull_request"]:
    raise SystemExit("ci.yml must run on push and pull_request")
if trigger_events(WORKFLOWS / "release.yml") != ["workflow_dispatch"]:
    raise SystemExit("release.yml must be manual-only")
if trigger_events(WORKFLOWS / "backend-release.yml") != ["workflow_dispatch"]:
    raise SystemExit("backend-release.yml must be manual-only")
if trigger_events(WORKFLOWS / "minijam-network-e2e.yml") != ["workflow_dispatch"]:
    raise SystemExit("minijam-network-e2e.yml must remain manual-only")
if trigger_events(WORKFLOWS / "toolchain-maintenance.yml") != ["workflow_dispatch"]:
    raise SystemExit("toolchain-maintenance.yml must be manual-only")

release = (WORKFLOWS / "release.yml").read_text(encoding="utf-8")
backend_release = (WORKFLOWS / "backend-release.yml").read_text(encoding="utf-8")
if "inputs:\n      version:" not in release:
    raise SystemExit("release.yml must accept only a version input")
if re.search(r"(?m)^    (?:push|pull_request|schedule):", release):
    raise SystemExit("release.yml has an unexpected automatic trigger")
for forbidden in ("build-backend-artifact.sh", "Dockerfile.backend", "ghcr.io/archelabs/jamscript-backend", "packages: write"):
    if forbidden in release:
        raise SystemExit(f"release.yml owns backend lifecycle: {forbidden}")
for required in ("build-backend-artifact.sh", "backend-manifest.json", "BACKEND_RELEASE_READY=PASS"):
    if required not in backend_release:
        raise SystemExit(f"backend-release.yml is missing {required}")
for forbidden in ("build-cli-archive.sh", "release-input-toolchain-", "jamscript-toolchain-scriptc"):
    if forbidden in backend_release:
        raise SystemExit(f"backend-release.yml owns JamScript lifecycle: {forbidden}")

print("ACTIVE_WORKFLOW_COUNT=5")
print("CI_TRIGGER_POLICY=PASS")
