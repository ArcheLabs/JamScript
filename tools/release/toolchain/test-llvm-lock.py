#!/usr/bin/env python3
import importlib.util
import tempfile
from pathlib import Path


MODULE = Path(__file__).with_name("llvm-lock.py")
SPEC = importlib.util.spec_from_file_location("llvm_lock", MODULE)
llvm_lock = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(llvm_lock)


BASE = """\
format = 1
platform = "linux-x86_64"
llvm_version = "20.1.8"
distribution = "llvm-official-linux-x64"
archive_url = "https://github.com/llvm/llvm-project/releases/download/llvmorg-20.1.8/LLVM-20.1.8-Linux-X64.tar.xz"
archive_filename = "LLVM-20.1.8-Linux-X64.tar.xz"
archive_sha256 = "1ead36b3dfcb774b57be530df42bec70ab2d239fbce9889447c7a29a4ddc1ae6"
clang_relpath = "bin/clang"
llvm_ar_relpath = "bin/llvm-ar"
lld_relpath = "bin/ld.lld"
clang_sha256 = "92a0ff2751ad344b94c7d30f53c56a71a5edfb6f867e7269a573e0e3f663786a"
llvm_ar_sha256 = "2eb06f0fa41af4e68aca4a5f09a9cb5375361e9fb4aec542bd5c0744dd6bcd87"
ld_lld_sha256 = "4213c4392590fad303a8c36dc4819e46a3d344ee656ee6431204410f36a79d3d"
"""
EXPECTED_TOOLS = {
    "clang_sha256": "92a0ff2751ad344b94c7d30f53c56a71a5edfb6f867e7269a573e0e3f663786a",
    "llvm_ar_sha256": "2eb06f0fa41af4e68aca4a5f09a9cb5375361e9fb4aec542bd5c0744dd6bcd87",
    "ld_lld_sha256": "4213c4392590fad303a8c36dc4819e46a3d344ee656ee6431204410f36a79d3d",
}


def assert_rejected(text, expected):
    with tempfile.TemporaryDirectory() as directory:
        path = Path(directory) / "lock.toml"
        path.write_text(text, encoding="utf-8")
        try:
            llvm_lock.parse_lock(path)
        except llvm_lock.LockError as error:
            assert expected in str(error), (expected, error)
        else:
            raise AssertionError(f"accepted invalid lock: {expected}")


def assert_expected_tool_hashes(values):
    actual = {key: values[key] for key in EXPECTED_TOOLS}
    if actual != EXPECTED_TOOLS:
        raise AssertionError(f"LLVM tool hash drift: {actual}")


with tempfile.TemporaryDirectory() as directory:
    path = Path(directory) / "lock.toml"
    path.write_text(BASE, encoding="utf-8")
    values = llvm_lock.parse_lock(path)
    assert values["llvm_version"] == "20.1.8"
    assert_expected_tool_hashes(values)

with tempfile.TemporaryDirectory() as directory:
    path = Path(directory) / "lock.toml"
    path.write_text(BASE.replace(EXPECTED_TOOLS["clang_sha256"], "0" * 64), encoding="utf-8")
    try:
        assert_expected_tool_hashes(llvm_lock.parse_lock(path))
    except AssertionError:
        pass
    else:
        raise AssertionError("accepted LLVM binary hash drift")

assert_rejected(BASE.replace('archive_sha256 = "1e', 'archive_sha256 = "zz'), "archive_sha256")
assert_rejected(BASE.replace('clang_sha256 = "92a0', 'clang_sha256 = 123'), "invalid lock syntax")
assert_rejected(BASE.replace('clang_relpath = "bin/clang"', 'clang_relpath = "../clang"'), "unsafe LLVM tool path")
assert_rejected(BASE.replace('llvm_version = "20.1.8"', 'llvm_version = "20.1.7"'), "20.1.8")
assert_rejected(BASE.replace('archive_url = "https://github.com/llvm/llvm-project/releases/download/llvmorg-20.1.8/', 'archive_url = "https://github.com/llvm/llvm-project/releases/download/latest/'), "floating")
print("LLVM_LOCK_TESTS=PASS")
