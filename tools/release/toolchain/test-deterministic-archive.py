#!/usr/bin/env python3
import io
import os
import subprocess
import tarfile
import tempfile
from pathlib import Path


ROOT = Path(__file__).resolve().parents[3]
PACKER = ROOT / "tools/release/toolchain/create-deterministic-archive.py"


def write_tree(root, reverse):
    entries = [
        ("bin/clang", b"clang\n", 0o755),
        ("bin/ld64.lld", b"lld\n", 0o755),
        ("lib/rustlib/target/libstd.dylib", b"rust\n", 0o644),
        ("manifest.json", b'{"files": {}}\n', 0o644),
    ]
    if reverse:
        entries.reverse()
    for relative, content, mode in entries:
        path = root / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(content)
        path.chmod(mode)
        os.utime(path, (17 if reverse else 31, 17 if reverse else 31))


def archive_bytes(root):
    result = subprocess.run(
        [
            "python3",
            str(PACKER),
            "--root",
            str(root),
            "--source-date-epoch",
            "1234567890",
        ],
        check=True,
        stdout=subprocess.PIPE,
    )
    return result.stdout


with tempfile.TemporaryDirectory(prefix="jamscript-deterministic-archive-") as directory:
    root = Path(directory)
    left = root / "left"
    right = root / "right"
    write_tree(left, reverse=False)
    write_tree(right, reverse=True)
    left_archive = archive_bytes(left)
    right_archive = archive_bytes(right)
    assert left_archive == right_archive

    with tarfile.open(fileobj=io.BytesIO(left_archive), mode="r:") as archive:
        members = archive.getmembers()
        assert [member.name for member in members] == [
            ".",
            "bin",
            "bin/clang",
            "bin/ld64.lld",
            "lib",
            "lib/rustlib",
            "lib/rustlib/target",
            "lib/rustlib/target/libstd.dylib",
            "manifest.json",
        ]
        assert all(member.uid == 0 and member.gid == 0 for member in members)
        assert all(member.uname == "root" and member.gname == "root" for member in members)
        assert all(member.mtime == 1234567890 for member in members)
        assert all(member.isreg() or member.isdir() for member in members)

print("DETERMINISTIC_ARCHIVE_TESTS=PASS")
