#!/usr/bin/env python3
"""Write the immutable manifest shipped with a JamScript GitHub release."""

import argparse
import hashlib
import json
from pathlib import Path


def digest(path: Path) -> str:
    value = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            value.update(chunk)
    return value.hexdigest()


def asset(path: Path, kind: str, release_url: str) -> dict:
    return {
        "name": path.name,
        "kind": kind,
        "url": f"{release_url}/{path.name}",
        "sha256": digest(path),
        "size": path.stat().st_size,
    }


parser = argparse.ArgumentParser()
parser.add_argument("--output", required=True, type=Path)
parser.add_argument("--release-version", required=True)
parser.add_argument("--source-commit", required=True)
parser.add_argument("--repository", required=True)
parser.add_argument("--target", required=True)
parser.add_argument("--cli", required=True, type=Path)
parser.add_argument("--toolchain", required=True, type=Path)
parser.add_argument("--toolchain-manifest", required=True, type=Path)
parser.add_argument("--bundle-metadata", required=True, type=Path)
parser.add_argument("--unsupported-target", action="append", default=[])
args = parser.parse_args()

for value in (args.release_version, args.source_commit):
    if not value or any(character in value for character in "\r\n"):
        raise SystemExit("manifest identity contains an invalid value")
if not args.release_version.startswith("v"):
    raise SystemExit("release version must be an immutable semver tag beginning with v")
if len(args.source_commit) != 40 or any(character not in "0123456789abcdefABCDEF" for character in args.source_commit):
    raise SystemExit("source commit must be a full hexadecimal git SHA")

toolchain = json.loads(args.toolchain_manifest.read_text(encoding="utf-8"))
bundle_metadata = json.loads(args.bundle_metadata.read_text(encoding="utf-8"))
if toolchain.get("toolchainId") != "scriptc-m2-v1":
    raise SystemExit("unexpected toolchain identity")
target = args.target
release_url = f"https://github.com/{args.repository}/releases/download/{args.release_version}"
cli = asset(args.cli, "jamscript-cli", release_url)
bundle = asset(args.toolchain, "managed-toolchain", release_url)
targets = [
    {
        "triple": target,
        "supported": True,
        "cli": cli,
        "toolchain": bundle,
    }
]
for unsupported in args.unsupported_target:
    targets.append(
        {
            "triple": unsupported,
            "supported": False,
            "reason": "No reproducible v0.1 producer is available on this branch.",
        }
    )

manifest = {
    "schema": 1,
    "format": "jamscript-release-manifest/v1",
    "releaseVersion": args.release_version,
    "sourceCommit": args.source_commit.lower(),
    "repository": args.repository,
    "layoutVersion": "toolchain-distribution-v1",
    "targets": targets,
    "toolchain": {
        "id": toolchain["toolchainId"],
        "platform": toolchain["platform"],
        "sourceRevision": bundle_metadata.get("sourceRevision"),
        "nodeVersion": toolchain.get("nodeVersion"),
        "clangVersion": toolchain.get("clangVersion"),
        "rustToolchain": toolchain.get("rustToolchain"),
        "scriptcRevision": toolchain.get("scriptcRevision"),
        "polkavmLinker": toolchain.get("polkavmLinker", "0.30.0"),
        "manifest": "toolchain-manifest.json",
    },
    "checksums": {
        "algorithm": "sha256",
        "asset": "SHA256SUMS",
    },
    "provenance": {
        "builder": "GitHub Actions",
        "workflow": "release-candidate.yml",
        "sourceCommit": args.source_commit.lower(),
    },
}

args.output.write_text(json.dumps(manifest, indent=2, sort_keys=True) + "\n", encoding="utf-8")
