#!/usr/bin/env python3
"""Write one immutable manifest for every platform in a JamScript release."""

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
parser.add_argument("--target", required=True, action="append")
parser.add_argument("--cli", required=True, type=Path, action="append")
parser.add_argument("--toolchain", required=True, type=Path, action="append")
parser.add_argument("--toolchain-manifest", required=True, type=Path, action="append")
parser.add_argument("--bundle-metadata", required=True, type=Path, action="append")
parser.add_argument("--unsupported-target", action="append", default=[])
args = parser.parse_args()

for value in (args.release_version, args.source_commit, args.repository):
    if not value or any(character in value for character in "\r\n"):
        raise SystemExit("manifest identity contains an invalid value")
if not args.release_version.startswith("v"):
    raise SystemExit("release version must be an immutable semver tag beginning with v")
if len(args.source_commit) != 40 or any(character not in "0123456789abcdefABCDEF" for character in args.source_commit):
    raise SystemExit("source commit must be a full hexadecimal git SHA")

fields = (args.target, args.cli, args.toolchain, args.toolchain_manifest, args.bundle_metadata)
if len({len(field) for field in fields}) != 1:
    raise SystemExit("each supported target needs a CLI, toolchain, manifest, and metadata")
if len(set(args.target)) != len(args.target):
    raise SystemExit("release targets must be unique")

release_url = f"https://github.com/{args.repository}/releases/download/{args.release_version}"
targets = []
common = None
for target, cli_path, bundle_path, manifest_path, metadata_path in zip(*fields):
    toolchain = json.loads(manifest_path.read_text(encoding="utf-8"))
    metadata = json.loads(metadata_path.read_text(encoding="utf-8"))
    if toolchain.get("toolchainId") != "scriptc-m2-v1":
        raise SystemExit(f"unexpected toolchain identity for {target}")
    if toolchain.get("platform") != target or metadata.get("platform") != target:
        raise SystemExit(f"toolchain metadata platform mismatch for {target}")
    if metadata.get("toolchainId") != toolchain["toolchainId"]:
        raise SystemExit(f"toolchain metadata identity mismatch for {target}")
    if metadata.get("archive") != bundle_path.name:
        raise SystemExit(f"toolchain metadata archive mismatch for {target}")
    source_revision = metadata.get("sourceRevision")
    if not isinstance(source_revision, str) or source_revision.lower() != args.source_commit.lower():
        raise SystemExit(f"toolchain metadata source revision mismatch for {target}")
    if common is None:
        common = {
            "id": toolchain["toolchainId"],
            "nodeVersion": toolchain.get("nodeVersion"),
            "clangVersion": toolchain.get("clangVersion"),
            "rustToolchain": toolchain.get("rustToolchain"),
            "scriptcRevision": toolchain.get("scriptcRevision"),
            "polkavmLinker": "0.30.0",
            "jamTargetVersion": toolchain.get("jamTargetVersion"),
            "jamBlobEncoderVersion": toolchain.get("jamBlobEncoderVersion"),
        }
    else:
        for key in ("nodeVersion", "clangVersion", "rustToolchain", "scriptcRevision", "jamTargetVersion", "jamBlobEncoderVersion"):
            if toolchain.get(key) != common.get(key):
                raise SystemExit(f"common toolchain identity differs for {target}: {key}")
    targets.append(
        {
            "triple": target,
            "supported": True,
            "cli": asset(cli_path, "jamscript-cli", release_url),
            "toolchain": asset(bundle_path, "managed-toolchain", release_url),
            "toolchainManifest": asset(manifest_path, "toolchain-manifest", release_url),
            "toolchainMetadata": asset(metadata_path, "toolchain-metadata", release_url),
        }
    )

for unsupported in args.unsupported_target:
    if unsupported in args.target:
        raise SystemExit(f"target cannot be both supported and unsupported: {unsupported}")
    targets.append(
        {
            "triple": unsupported,
            "supported": False,
            "reason": "Native Windows support is outside the v0.1 release scope." if unsupported == "windows-x86_64" else "Target is outside this release scope.",
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
    "toolchain": common,
    "checksums": {"algorithm": "sha256", "asset": "SHA256SUMS"},
    "provenance": {
        "builder": "GitHub Actions",
        "workflow": "release-candidate.yml",
        "sourceCommit": args.source_commit.lower(),
    },
}
args.output.write_text(json.dumps(manifest, indent=2, sort_keys=True) + "\n", encoding="utf-8")
