#!/usr/bin/env python3
"""Write a deterministic tar stream for a staged toolchain directory."""

import argparse
import os
import stat
import sys
import tarfile
from pathlib import Path


def parse_args():
    parser = argparse.ArgumentParser()
    parser.add_argument("--root", required=True, type=Path)
    parser.add_argument("--source-date-epoch", required=True, type=int)
    return parser.parse_args()


def relative_key(path, root):
    return path.relative_to(root).as_posix().encode("utf-8")


def make_info(name, mode, epoch, member_type, size=0):
    info = tarfile.TarInfo(name)
    info.mode = stat.S_IMODE(mode)
    info.uid = 0
    info.gid = 0
    info.uname = "root"
    info.gname = "root"
    info.mtime = epoch
    info.type = member_type
    info.size = size
    info.pax_headers = {}
    return info


def write_archive(root, epoch):
    if not root.is_dir():
        raise SystemExit(f"archive root is not a directory: {root}")

    paths = sorted(root.rglob("*"), key=lambda path: relative_key(path, root))
    with tarfile.open(fileobj=sys.stdout.buffer, mode="w|", format=tarfile.PAX_FORMAT) as archive:
        archive.addfile(make_info(".", root.stat().st_mode, epoch, tarfile.DIRTYPE))
        for path in paths:
            relative = path.relative_to(root).as_posix()
            metadata = path.lstat()
            if stat.S_ISLNK(metadata.st_mode):
                raise SystemExit(f"symbolic links are not allowed in deterministic archives: {relative}")
            if stat.S_ISDIR(metadata.st_mode):
                archive.addfile(make_info(relative, metadata.st_mode, epoch, tarfile.DIRTYPE))
                continue
            if not stat.S_ISREG(metadata.st_mode):
                raise SystemExit(f"unsupported file type in deterministic archive: {relative}")
            info = make_info(relative, metadata.st_mode, epoch, tarfile.REGTYPE, metadata.st_size)
            with path.open("rb") as stream:
                archive.addfile(info, stream)


def main():
    args = parse_args()
    if args.source_date_epoch < 0:
        raise SystemExit("SOURCE_DATE_EPOCH must be non-negative")
    try:
        write_archive(args.root, args.source_date_epoch)
    except BrokenPipeError:
        try:
            sys.stdout.close()
        except BrokenPipeError:
            pass
        os._exit(1)


if __name__ == "__main__":
    main()
