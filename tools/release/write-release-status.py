#!/usr/bin/env python3
"""Write a machine-readable release preflight or publication status record."""

import argparse
import json
import re
from pathlib import Path


SHA = re.compile(r"^[0-9a-f]{40}$")
PLATFORMS = ("linux-x86_64", "macos-arm64")


parser = argparse.ArgumentParser()
parser.add_argument("--output", required=True, type=Path)
parser.add_argument("--version", required=True)
parser.add_argument("--source-revision", required=True)
parser.add_argument("--stage", required=True, choices=("preflight", "published"))
parser.add_argument("--platform-result", action="append", default=[])
parser.add_argument("--reproducibility", required=True, choices=("PASS", "FAIL"))
parser.add_argument("--consumer-validation", required=True, choices=("PASS", "FAIL"))
parser.add_argument("--result", required=True, choices=("PASS", "FAIL"))
parser.add_argument("--workflow-run-id", default="")
parser.add_argument("--workflow-run-attempt", default="")
args = parser.parse_args()

if not re.fullmatch(r"^v0\.1\.0(-rc\.[0-9]+)?$", args.version):
    raise SystemExit("invalid release version")
if not SHA.fullmatch(args.source_revision):
    raise SystemExit("source revision must be a full lowercase git SHA")

platform_results = {}
for item in args.platform_result:
    try:
        platform, result = item.split("=", 1)
    except ValueError:
        raise SystemExit(f"invalid platform result: {item}")
    if platform not in PLATFORMS or result not in ("PASS", "FAIL"):
        raise SystemExit(f"invalid platform result: {item}")
    if platform in platform_results:
        raise SystemExit(f"duplicate platform result: {platform}")
    platform_results[platform] = result

status = {
    "schemaVersion": 1,
    "version": args.version,
    "sourceRevision": args.source_revision,
    "stage": args.stage,
    "platformResults": platform_results,
    "reproducibility": args.reproducibility,
    "consumerValidation": args.consumer_validation,
    "result": args.result,
    "workflowRunId": args.workflow_run_id,
    "workflowRunAttempt": args.workflow_run_attempt,
}
args.output.write_text(json.dumps(status, indent=2, sort_keys=True) + "\n", encoding="utf-8")
