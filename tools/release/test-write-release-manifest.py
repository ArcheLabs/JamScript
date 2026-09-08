#!/usr/bin/env python3
import hashlib
import json
import subprocess
import sys
import tempfile
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
WRITER = ROOT / "tools/release/write-release-manifest.py"
SOURCE_SHA = "a" * 40


def write_json(path: Path, value: dict) -> None:
    path.write_text(json.dumps(value) + "\n", encoding="utf-8")


with tempfile.TemporaryDirectory(prefix="jamscript-release-manifest-test-") as directory:
    root = Path(directory)
    args = [
        sys.executable,
        str(WRITER),
        "--output",
        str(root / "release-manifest.json"),
        "--release-version",
        "v0.1.0-rc.2",
        "--source-commit",
        SOURCE_SHA,
        "--repository",
        "ArcheLabs/JamScript",
    ]
    for platform in ("linux-x86_64", "macos-arm64"):
        cli = root / f"jamscript-v0.1.0-rc.2-{platform}.tar.gz"
        bundle = root / f"jamscript-toolchain-scriptc-m2-v1-{platform}.tar.zst"
        manifest = root / f"toolchain-manifest-{platform}.json"
        metadata = root / f"bundle-metadata-{platform}.json"
        cli.write_bytes(platform.encode())
        bundle.write_bytes(platform.encode())
        write_json(
            manifest,
            {
                "toolchainId": "scriptc-m2-v1",
                "platform": platform,
                "nodeVersion": "24.15.0",
                "clangVersion": "20.1.8",
                "rustToolchain": "nightly-2026-05-02",
                "scriptcRevision": SOURCE_SHA,
                "jamTargetVersion": "jam-v1",
                "jamBlobEncoderVersion": "0.1.28",
            },
        )
        write_json(
            metadata,
            {
                "toolchainId": "scriptc-m2-v1",
                "platform": platform,
                "archive": bundle.name,
                "sourceRevision": SOURCE_SHA,
            },
        )
        args.extend(
            [
                "--target",
                platform,
                "--cli",
                str(cli),
                "--toolchain",
                str(bundle),
                "--toolchain-manifest",
                str(manifest),
                "--bundle-metadata",
                str(metadata),
            ]
        )
    args.extend(["--unsupported-target", "windows-x86_64"])
    subprocess.run(args, cwd=ROOT, check=True)
    release = json.loads((root / "release-manifest.json").read_text(encoding="utf-8"))
    assert release["releaseVersion"] == "v0.1.0-rc.2"
    assert release["sourceCommit"] == SOURCE_SHA
    assert [target["triple"] for target in release["targets"]] == [
        "linux-x86_64",
        "macos-arm64",
        "windows-x86_64",
    ]
    assert all(target["supported"] for target in release["targets"][:2])
    assert release["targets"][2]["supported"] is False
    for target in release["targets"][:2]:
        asset_path = root / target["cli"]["name"]
        assert target["cli"]["sha256"] == hashlib.sha256(asset_path.read_bytes()).hexdigest()

print("RELEASE_MANIFEST_TESTS=PASS")
