#!/usr/bin/env python3
"""Enforce that publication uses measured LLVM executable hashes."""

import argparse
import importlib.util
from pathlib import Path
from typing import Dict, Union


ZERO_SHA256 = "0" * 64
REQUIRED_PLATFORM_HASHES = ("clang_sha256", "llvm_ar_sha256", "ld_lld_sha256")


def load_lock_parser():
    module_path = Path(__file__).with_name("llvm-lock.py")
    spec = importlib.util.spec_from_file_location("llvm_lock", module_path)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"unable to load LLVM lock parser: {module_path}")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def check_lock(lock_path: Path, platform: str) -> Dict[str, Union[str, int]]:
    parser = load_lock_parser()
    values = parser.parse_lock(lock_path)
    if values["platform"] != platform:
        raise ValueError(
            f"LLVM lock platform mismatch: expected {platform}, got {values['platform']}"
        )
    if platform == "macos-arm64":
        pending = [
            key for key in REQUIRED_PLATFORM_HASHES if values[key] == ZERO_SHA256
        ]
        if pending:
            raise ValueError(
                "macOS LLVM executable hashes require native promotion: "
                + ", ".join(pending)
            )
    return values


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("lock", type=Path)
    parser.add_argument("--platform", required=True, choices=("linux-x86_64", "macos-arm64"))
    args = parser.parse_args()
    try:
        values = check_lock(args.lock, args.platform)
    except (OSError, ValueError, RuntimeError) as error:
        if args.platform == "macos-arm64":
            print("MACOS_LLVM_LOCK_PROMOTED=FAIL")
        print(f"LLVM_LOCK_PROMOTION=FAIL: {error}")
        return 1

    if args.platform == "macos-arm64":
        print("MACOS_LLVM_LOCK_PROMOTED=PASS")
    else:
        print("LLVM_LOCK_PROMOTED=PASS")
    for key in REQUIRED_PLATFORM_HASHES:
        print(f"{key}={values[key]}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
