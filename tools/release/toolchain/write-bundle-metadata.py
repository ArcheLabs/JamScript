#!/usr/bin/env python3
import argparse
import json

parser = argparse.ArgumentParser()
parser.add_argument("--output", required=True)
parser.add_argument("--toolchain-id", required=True)
parser.add_argument("--platform", required=True)
parser.add_argument("--archive", required=True)
parser.add_argument("--source-revision", required=True)
parser.add_argument("--published", action="store_true")
args = parser.parse_args()

metadata = {
    "format": 1,
    "toolchainId": args.toolchain_id,
    "platform": args.platform,
    "archive": args.archive,
    "sourceRevision": args.source_revision,
    "published": args.published,
}
with open(args.output, "w", encoding="utf-8", newline="\n") as stream:
    json.dump(metadata, stream, indent=2, sort_keys=True)
    stream.write("\n")
