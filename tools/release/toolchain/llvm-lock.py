#!/usr/bin/env python3
"""Small, dependency-free parser and validator for the LLVM distribution lock."""

import argparse
import re
from pathlib import Path


HEX64 = re.compile(r"^[0-9a-f]{64}$")
KEY = re.compile(r"^([A-Za-z_][A-Za-z0-9_]*)\s*=\s*(?:\"([^\"]*)\"|([0-9]+))\s*$")


class LockError(ValueError):
    pass


def parse_lock(path):
    values = {}
    for number, raw_line in enumerate(Path(path).read_text(encoding="utf-8").splitlines(), 1):
        line = raw_line.split("#", 1)[0].strip()
        if not line or line.startswith("["):
            continue
        match = KEY.match(line)
        if not match:
            if "=" in line:
                raise LockError(f"invalid lock syntax on line {number}")
            continue
        key, string_value, integer_value = match.groups()
        values[key] = string_value if string_value is not None else int(integer_value)

    required = (
        "format", "version", "platform", "llvm_version", "clang_version", "distribution", "archive_url",
        "archive_filename", "archive_sha256", "clang_relpath", "llvm_ar_relpath",
        "lld_relpath", "clang_sha256", "llvm_ar_sha256", "ld_lld_sha256",
    )
    missing = [key for key in required if key not in values]
    if missing:
        raise LockError("missing lock fields: " + ", ".join(missing))
    if values["format"] != 1:
        raise LockError("unsupported LLVM lock format")
    if values["platform"] != "linux-x86_64":
        raise LockError("LLVM lock platform is not linux-x86_64")
    if values["llvm_version"] != "20.1.8":
        raise LockError("LLVM lock must pin version 20.1.8")
    if values["clang_version"] != values["llvm_version"]:
        raise LockError("LLVM clang version disagrees with LLVM version")
    if values["distribution"] != "llvm-official-linux-x64":
        raise LockError("unexpected LLVM distribution")
    url = values["archive_url"]
    if not url.startswith("https://github.com/llvm/llvm-project/releases/download/"):
        raise LockError("LLVM archive URL is not the official release endpoint")
    if any(word in url.lower() for word in ("latest", "main", "nightly", "rolling")):
        raise LockError("LLVM archive URL is floating")
    if "/llvmorg-20.1.8/" not in url:
        raise LockError("LLVM archive URL is not pinned to llvmorg-20.1.8")
    if values["archive_filename"] != "LLVM-20.1.8-Linux-X64.tar.xz":
        raise LockError("unexpected LLVM archive filename")
    if not url.endswith("/" + values["archive_filename"]):
        raise LockError("archive filename does not match URL")
    for key in ("archive_sha256", "clang_sha256", "llvm_ar_sha256", "ld_lld_sha256"):
        value = values[key]
        if not isinstance(value, str) or not HEX64.fullmatch(value):
            raise LockError(f"{key} must be a lowercase SHA-256")
    for key in ("clang_relpath", "llvm_ar_relpath", "lld_relpath"):
        value = values[key]
        path = Path(value)
        if path.is_absolute() or ".." in path.parts or "\\" in value or not value.startswith("bin/"):
            raise LockError(f"unsafe LLVM tool path: {key}")
    return values


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("lock", type=Path)
    parser.add_argument("--get", choices=("archive_filename", "archive_url", "llvm_version", "distribution", "archive_sha256", "clang_sha256", "llvm_ar_sha256", "ld_lld_sha256"))
    args = parser.parse_args()
    values = parse_lock(args.lock)
    if args.get:
        print(values[args.get])
    else:
        for key in sorted(values):
            print(f"{key}={values[key]}")


if __name__ == "__main__":
    main()
