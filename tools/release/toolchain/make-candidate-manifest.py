#!/usr/bin/env python3
import argparse
import re
from pathlib import Path

parser = argparse.ArgumentParser()
parser.add_argument("--input", required=True, type=Path)
parser.add_argument("--output", required=True, type=Path)
parser.add_argument("--platform", required=True)
parser.add_argument("--url", required=True)
parser.add_argument("--sha256", required=True)
parser.add_argument("--size", required=True)
args = parser.parse_args()

text = args.input.read_text(encoding="utf-8")
section_pattern = re.compile(
    rf'(^\[platforms\.{re.escape(args.platform)}\]\n)(.*?)(?=^\[|\Z)',
    re.MULTILINE | re.DOTALL,
)
section = section_pattern.search(text)
if not section:
    raise SystemExit(f"distribution manifest has no platform section for {args.platform}")
body = section.group(2)
body, url_count = re.subn(r'^url = ".*"$', f'url = "{args.url}"', body, count=1, flags=re.MULTILINE)
body, sha_count = re.subn(r'^sha256 = ".*"$', f'sha256 = "{args.sha256}"', body, count=1, flags=re.MULTILINE)
body, size_count = re.subn(r'^size = [0-9]+$', f'size = {args.size}', body, count=1, flags=re.MULTILINE)
body, published_count = re.subn(r'^published = false$', 'published = true', body, count=1, flags=re.MULTILINE)
if (url_count, sha_count, size_count, published_count) != (1, 1, 1, 1):
    raise SystemExit(f"platform section {args.platform} did not have expected candidate fields")
text = text[:section.start()] + section.group(1) + body + text[section.end():]
args.output.write_text(text, encoding="utf-8")
