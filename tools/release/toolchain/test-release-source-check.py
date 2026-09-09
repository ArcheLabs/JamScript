#!/usr/bin/env python3
"""Regression test for the publication-grade LLVM sentinel gate."""

import re
import subprocess
import sys
import tempfile
from pathlib import Path
from typing import Any


ROOT = Path(__file__).resolve().parents[3]
LOCK = ROOT / "toolchains/llvm/macos-arm64.lock"
CHECKER = ROOT / "tools/release/toolchain/check-llvm-lock-promoted.py"
HASH_KEYS = ("clang_sha256", "llvm_ar_sha256", "ld_lld_sha256")
VALID_HASHES = {
    "clang_sha256": "1" * 64,
    "llvm_ar_sha256": "2" * 64,
    "ld_lld_sha256": "3" * 64,
}


def run(lock: Path) -> Any:
    return subprocess.run(
        [sys.executable, str(CHECKER), str(lock), "--platform", "macos-arm64"],
        cwd=ROOT,
        text=True,
        capture_output=True,
        check=False,
    )


with tempfile.TemporaryDirectory(prefix="jamscript-release-source-test-") as directory:
    fixture = Path(directory) / "macos-arm64.lock"
    original = LOCK.read_text(encoding="utf-8")
    valid = original
    for key, value in VALID_HASHES.items():
        valid, count = re.subn(
            rf'^{key} = "[0-9a-f]+"$',
            f'{key} = "{value}"',
            valid,
            count=1,
            flags=re.MULTILINE,
        )
        assert count == 1, key
    fixture.write_text(valid, encoding="utf-8")
    result = run(fixture)
    assert result.returncode == 0, result.stderr + result.stdout
    assert "MACOS_LLVM_LOCK_PROMOTED=PASS" in result.stdout

    for key in HASH_KEYS:
        pending = valid.replace(f'{key} = "{VALID_HASHES[key]}"', f'{key} = "{"0" * 64}"')
        fixture.write_text(pending, encoding="utf-8")
        result = run(fixture)
        assert result.returncode != 0, key
        assert "MACOS_LLVM_LOCK_PROMOTED=FAIL" in result.stdout, result.stdout
        assert key in result.stdout, result.stdout

print("RELEASE_SOURCE_SENTINEL_TESTS=PASS")
