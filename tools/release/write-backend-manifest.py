#!/usr/bin/env python3
"""Write the immutable manifest for an independently released backend."""

import argparse
import json
from pathlib import Path


PROTOCOL = {
    "formalRpc": "v1",
    "signedAction": "v1",
    "runtimeRefineInput": "v1",
    "managedStateWitness": "v1",
}


parser = argparse.ArgumentParser()
parser.add_argument("--output", required=True, type=Path)
parser.add_argument("--backend-version", required=True)
parser.add_argument("--source-commit", required=True)
parser.add_argument("--repository", required=True)
parser.add_argument("--target", required=True, action="append")
parser.add_argument("--artifact", required=True, type=Path, action="append")
args = parser.parse_args()

for value in (args.backend_version, args.source_commit, args.repository):
    if not value or any(character in value for character in "\r\n"):
        raise SystemExit("backend manifest identity contains an invalid value")
if not args.backend_version.startswith("backend-v"):
    raise SystemExit("backend version must be an immutable backend-v semver tag")
if len(args.source_commit) != 40 or any(
    character not in "0123456789abcdefABCDEF" for character in args.source_commit
):
    raise SystemExit("source commit must be a full hexadecimal git SHA")
if len(args.target) != len(args.artifact):
    raise SystemExit("each backend target needs exactly one artifact")
if len(set(args.target)) != len(args.target):
    raise SystemExit("backend targets must be unique")

targets = []
for target, artifact_path in zip(args.target, args.artifact):
    if not artifact_path.is_file():
        raise SystemExit(f"backend artifact is missing: {artifact_path}")
    targets.append({"target": target, "artifact": artifact_path.name})

manifest = {
    "schema": 1,
    "format": "jamscript-backend-manifest/v1",
    "backendVersion": args.backend_version[len("backend-v") :],
    "releaseTag": args.backend_version,
    "sourceCommit": args.source_commit.lower(),
    "repository": args.repository,
    "protocol": PROTOCOL,
    "targets": targets,
    "provenance": {
        "builder": "GitHub Actions",
        "workflow": "backend-release.yml",
        "sourceCommit": args.source_commit.lower(),
    },
}
args.output.write_text(json.dumps(manifest, indent=2, sort_keys=True) + "\n", encoding="utf-8")
