#!/usr/bin/env python3
import json
import subprocess
import sys
import tempfile
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
WRITER = ROOT / "tools/release/write-backend-manifest.py"
SOURCE_SHA = "b" * 40


with tempfile.TemporaryDirectory(prefix="jamscript-backend-manifest-test-") as directory:
    root = Path(directory)
    args = [
        sys.executable,
        str(WRITER),
        "--output",
        str(root / "backend-manifest.json"),
        "--backend-version",
        "backend-v0.1.0-rc.1",
        "--source-commit",
        SOURCE_SHA,
        "--repository",
        "ArcheLabs/JamScript",
    ]
    for platform in ("linux-x86_64", "macos-arm64"):
        artifact = root / f"jamscript-backend-v0.1.0-rc.1-{platform}.tar.gz"
        artifact.write_bytes(platform.encode())
        args.extend(["--target", platform, "--artifact", str(artifact)])
    subprocess.run(args, cwd=ROOT, check=True)
    manifest = json.loads((root / "backend-manifest.json").read_text(encoding="utf-8"))
    assert manifest["backendVersion"] == "0.1.0-rc.1"
    assert manifest["releaseTag"] == "backend-v0.1.0-rc.1"
    assert manifest["sourceCommit"] == SOURCE_SHA
    assert manifest["protocol"] == {
        "formalRpc": "v1",
        "signedAction": "v1",
        "runtimeRefineInput": "v1",
        "managedStateWitness": "v1",
    }
    assert [target["target"] for target in manifest["targets"]] == [
        "linux-x86_64",
        "macos-arm64",
    ]
    for target in manifest["targets"]:
        assert target["artifact"].startswith("jamscript-backend-v0.1.0-rc.1-")

print("BACKEND_MANIFEST_TESTS=PASS")
