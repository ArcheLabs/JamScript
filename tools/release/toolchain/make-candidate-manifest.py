#!/usr/bin/env python3
import argparse
import re
from pathlib import Path

parser = argparse.ArgumentParser()
parser.add_argument("--input", required=True, type=Path)
parser.add_argument("--output", required=True, type=Path)
parser.add_argument("--url", required=True)
parser.add_argument("--sha256", required=True)
parser.add_argument("--size", required=True)
args = parser.parse_args()

text = args.input.read_text(encoding="utf-8")
text, url_count = re.subn(r'^url = ".*"$', f'url = "{args.url}"', text, count=1, flags=re.MULTILINE)
text, sha_count = re.subn(r'^sha256 = ".*"$', f'sha256 = "{args.sha256}"', text, count=1, flags=re.MULTILINE)
text, size_count = re.subn(r'^size = [0-9]+$', f'size = {args.size}', text, count=1, flags=re.MULTILINE)
text, published_count = re.subn(r'^published = false$', 'published = true', text, count=1, flags=re.MULTILINE)
if (url_count, sha_count, size_count, published_count) != (1, 1, 1, 1):
    raise SystemExit("distribution manifest did not have the expected candidate fields")
args.output.write_text(text, encoding="utf-8")
