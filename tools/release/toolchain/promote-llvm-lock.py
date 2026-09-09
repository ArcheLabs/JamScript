#!/usr/bin/env python3
"""Write a measured macOS LLVM lock candidate without modifying the source lock."""

import argparse
import importlib.util
import re
from pathlib import Path


SHA256 = re.compile(r"^[0-9a-f]{64}$")
ZERO_SHA256 = "0" * 64
HASH_KEYS = ("clang_sha256", "llvm_ar_sha256", "ld_lld_sha256")


def load_lock_parser():
    module_path = Path(__file__).with_name("llvm-lock.py")
    spec = importlib.util.spec_from_file_location("llvm_lock", module_path)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"unable to load LLVM lock parser: {module_path}")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


parser = argparse.ArgumentParser()
parser.add_argument("--input", required=True, type=Path)
parser.add_argument("--output", required=True, type=Path)
for key in HASH_KEYS:
    parser.add_argument(f"--{key.replace('_', '-')}", required=True)
args = parser.parse_args()

values = {key: getattr(args, key) for key in HASH_KEYS}
for key, value in values.items():
    if not SHA256.fullmatch(value) or value == ZERO_SHA256:
        raise SystemExit(f"{key} must be a non-zero lowercase SHA-256")

lock_parser = load_lock_parser()
source_values = lock_parser.parse_lock(args.input)
if source_values["platform"] != "macos-arm64":
    raise SystemExit("promotion input must be the macos-arm64 LLVM lock")
pending = [key for key in HASH_KEYS if source_values[key] != ZERO_SHA256]
if pending:
    raise SystemExit("promotion input is already populated: " + ", ".join(pending))

text = args.input.read_text(encoding="utf-8")
for key, value in values.items():
    old = rf'(^\s*{re.escape(key)}\s*=\s*)"{ZERO_SHA256}"(\s*(?:#.*)?$)'
    text, count = re.subn(old, rf'\g<1>"{value}"\g<2>', text, count=1, flags=re.MULTILINE)
    if count != 1:
        raise SystemExit(f"could not promote {key} in {args.input}")

args.output.write_text(text, encoding="utf-8")
promoted = lock_parser.parse_lock(args.output)
for key, value in values.items():
    if promoted[key] != value:
        raise SystemExit(f"promotion output changed unexpectedly: {key}")
print(f"PROMOTED_LOCK={args.output}")
