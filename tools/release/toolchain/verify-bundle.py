#!/usr/bin/env python3
import argparse
import hashlib
import importlib.util
import json
import os
import re
import subprocess
from pathlib import Path


def read_text(path):
    return path.read_text(encoding="utf-8").strip()


def run_version(path):
    result = subprocess.run(
        [str(path), "--version"],
        check=True,
        capture_output=True,
        text=True,
    )
    return result.stdout.strip() or result.stderr.strip()


def sha256(path):
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


parser = argparse.ArgumentParser()
parser.add_argument("--root", required=True, type=Path)
parser.add_argument("--source-root", required=True, type=Path)
args = parser.parse_args()
root = args.root.resolve()
source_root = args.source_root.resolve()

distribution_text = (source_root / "toolchains/distribution-v1.toml").read_text(encoding="utf-8")
llvm_lock_module_path = source_root / "tools/release/toolchain/llvm-lock.py"
llvm_lock_spec = importlib.util.spec_from_file_location("llvm_lock", llvm_lock_module_path)
llvm_lock = importlib.util.module_from_spec(llvm_lock_spec)
llvm_lock_spec.loader.exec_module(llvm_lock)
llvm_lock_values = llvm_lock.parse_lock(source_root / "toolchains/llvm/linux-x86_64.lock")


def toml_string(name):
    match = re.search(
        r"^" + re.escape(name) + r'\s*=\s*"([^"]*)"$',
        distribution_text,
        re.MULTILINE,
    )
    if not match:
        raise SystemExit(f"missing distribution lock field: {name}")
    return match.group(1)


distribution = {
    "format": 1,
    "toolchain_id": toml_string("toolchain_id"),
    "node_version": toml_string("node_version"),
    "rust_toolchain": toml_string("rust_toolchain"),
    "clang_version": toml_string("clang_version"),
    "minijam_revision": toml_string("minijam_revision"),
    "scriptc_revision": toml_string("scriptc_revision"),
}
manifest = json.loads((root / "manifest.json").read_text(encoding="utf-8"))
if not isinstance(manifest, dict):
    raise SystemExit("internal manifest is not an object")

checks = {
    "format": distribution["format"],
    "toolchainId": distribution["toolchain_id"],
    "platform": "linux-x86_64",
    "nodeVersion": distribution["node_version"],
    "clangVersion": distribution["clang_version"],
    "rustToolchain": distribution["rust_toolchain"],
    "minijamRevision": distribution["minijam_revision"],
    "scriptcRevision": distribution["scriptc_revision"],
}
for key, value in checks.items():
    if manifest.get(key) != value:
        raise SystemExit(f"internal manifest identity mismatch: {key}")

manifest_llvm = manifest.get("llvm")
expected_llvm = {
    "distribution": llvm_lock_values["distribution"],
    "version": llvm_lock_values["llvm_version"],
    "archiveSha256": llvm_lock_values["archive_sha256"],
    "clangSha256": llvm_lock_values["clang_sha256"],
    "llvmArSha256": llvm_lock_values["llvm_ar_sha256"],
    "ldLldSha256": llvm_lock_values["ld_lld_sha256"],
}
if manifest_llvm != expected_llvm:
    raise SystemExit("internal manifest LLVM provenance mismatch")

files = manifest.get("files")
if not isinstance(files, dict) or not files:
    raise SystemExit("internal manifest has no file hashes")

for current, directories, names in os.walk(root, followlinks=False):
    current_path = Path(current)
    for name in directories + names:
        path = current_path / name
        if path.is_symlink():
            raise SystemExit(f"extracted bundle contains a symlink: {path.relative_to(root)}")

for name, expected_hash in files.items():
    relative = Path(name)
    if relative.is_absolute() or ".." in relative.parts or "\\" in name:
        raise SystemExit(f"internal manifest contains an unsafe path: {name}")
    path = root / relative
    if not path.is_file():
        raise SystemExit(f"internal manifest file is missing: {name}")
    normalized_hash = expected_hash.lower()
    if normalized_hash.startswith("0x"):
        normalized_hash = normalized_hash[2:]
    if sha256(path) != normalized_hash:
        raise SystemExit(f"internal manifest hash mismatch: {name}")

required_files = [
    "bin/node",
    "bin/clang",
    "bin/llvm-ar",
    "bin/ld.lld",
    "bin/rustc",
    "bin/cargo",
    "Cargo.lock",
    "toolchains/polkavm.lock",
    "targets/minijam/sdk/service-toolchain/compiler/polkavm-to-jam/target/release/minijam-polkavm-to-jam",
]
required_directories = [
    "scriptc",
    "runtime",
    "runtime-scriptc",
    "targets/minijam/sdk",
    "cargo/vendor",
]
for name in required_files:
    if not (root / name).is_file():
        raise SystemExit(f"required bundle file is missing: {name}")
for name in required_directories:
    if not (root / name).is_dir():
        raise SystemExit(f"required bundle directory is missing: {name}")

for name, key in (("bin/clang", "clang_sha256"), ("bin/llvm-ar", "llvm_ar_sha256"), ("bin/ld.lld", "ld_lld_sha256")):
    if sha256(root / name) != llvm_lock_values[key]:
        raise SystemExit(f"LLVM binary lock hash mismatch: {name}")

node_version = run_version(root / "bin/node")
if node_version.startswith("v"):
    node_version = node_version[1:]
if node_version != distribution["node_version"]:
    raise SystemExit(f"Node identity mismatch: {node_version}")
clang_version = run_version(root / "bin/clang").splitlines()[0]
if distribution["clang_version"] not in clang_version:
    raise SystemExit(f"Clang identity mismatch: {clang_version}")
for name in ["bin/llvm-ar", "bin/ld.lld", "bin/rustc", "bin/cargo"]:
    if not run_version(root / name):
        raise SystemExit(f"tool version query returned no output: {name}")
if "nightly" not in run_version(root / "bin/rustc"):
    raise SystemExit("Rust identity is not a nightly toolchain")

scriptc_revision = read_text(root / "scriptc/REVISION")
if f"commit={distribution['scriptc_revision']}" not in scriptc_revision:
    raise SystemExit("ScriptC revision identity mismatch")
if json.loads((root / "scriptc/node_modules/@scriptc/compiler/package.json").read_text())["version"] != "0.0.34":
    raise SystemExit("ScriptC compiler package identity mismatch")
if json.loads((root / "scriptc/node_modules/typescript/package.json").read_text())["version"] != "7.0.2":
    raise SystemExit("TypeScript package identity mismatch")

minijam_lock = read_text(source_root / "toolchains/minijam.lock")
if f"revision = \"{manifest['minijamRevision']}\"" not in minijam_lock:
    raise SystemExit("MiniJAM lock identity mismatch")

print("TOOLCHAIN_BUNDLE_STRUCTURE=PASS")
print("TOOLCHAIN_INTERNAL_MANIFEST=PASS")
print("TOOLCHAIN_NODE=PASS")
print("TOOLCHAIN_LLVM=PASS")
print("TOOLCHAIN_RUST=PASS")
print("TOOLCHAIN_SCRIPTC=PASS")
print("TOOLCHAIN_MINIJAM=PASS")
print("TOOLCHAIN_FORBIDDEN_PATHS=PASS")
