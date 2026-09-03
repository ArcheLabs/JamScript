#!/usr/bin/env python3
import hashlib
import os
import sys
from pathlib import Path


def files(root):
    return sorted(
        path.relative_to(root).as_posix()
        for path in root.rglob("*")
        if path.is_file() or path.is_symlink()
    )


def digest(path):
    if path.is_symlink():
        return f"symlink:{os.readlink(path)}"
    h = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            h.update(chunk)
    return h.hexdigest()


left, right = map(Path, sys.argv[1:3])
left_files, right_files = files(left), files(right)
if left_files != right_files:
    print("BUNDLE_FIRST_DIFFERING_FILE=list of files differs")
    print("A-only:", sorted(set(left_files) - set(right_files))[:5])
    print("B-only:", sorted(set(right_files) - set(left_files))[:5])
    raise SystemExit(1)
for relative in left_files:
    a, b = left / relative, right / relative
    if digest(a) != digest(b) or a.stat().st_mode != b.stat().st_mode:
        print(f"BUNDLE_FIRST_DIFFERING_FILE={relative}")
        print(f"A_SHA256={digest(a)}")
        print(f"B_SHA256={digest(b)}")
        print(f"A_MODE={oct(a.stat().st_mode)}")
        print(f"B_MODE={oct(b.stat().st_mode)}")
        raise SystemExit(1)
print("BUNDLE_EXTRACTED_CONTENT_IDENTICAL=PASS")
