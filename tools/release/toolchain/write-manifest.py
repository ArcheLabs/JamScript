#!/usr/bin/env python3
import argparse
import hashlib
import json
import os

parser = argparse.ArgumentParser()
parser.add_argument('--root', required=True)
parser.add_argument('--output', required=True)
parser.add_argument('--platform', required=True)
parser.add_argument('--toolchain-id', required=True)
parser.add_argument('--node-version', required=True)
parser.add_argument('--clang-version', required=True)
parser.add_argument('--rust-toolchain', required=True)
parser.add_argument('--minijam-revision', required=True)
parser.add_argument('--scriptc-revision', required=True)
args = parser.parse_args()

files = {}
for directory, _, names in os.walk(args.root):
    for name in sorted(names):
        path = os.path.join(directory, name)
        relative = os.path.relpath(path, args.root).replace(os.sep, '/')
        if relative == 'manifest.json':
            continue
        if os.path.islink(path):
            raise SystemExit(f'symlink is not allowed in a toolchain bundle: {relative}')
        digest = hashlib.sha256()
        with open(path, 'rb') as stream:
            for chunk in iter(lambda: stream.read(1024 * 1024), b''):
                digest.update(chunk)
        files[relative] = digest.hexdigest()

manifest = {
    'format': 1,
    'toolchainId': args.toolchain_id,
    'platform': args.platform,
    'nodeVersion': args.node_version,
    'clangVersion': args.clang_version,
    'rustToolchain': args.rust_toolchain,
    'minijamRevision': args.minijam_revision,
    'scriptcRevision': args.scriptc_revision,
    'files': files,
}
with open(args.output, 'w', encoding='utf-8', newline='\n') as stream:
    json.dump(manifest, stream, indent=2, sort_keys=True)
    stream.write('\n')
