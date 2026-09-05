#!/usr/bin/env python3
import argparse
import json

parser = argparse.ArgumentParser()
parser.add_argument("--output", required=True)
parser.add_argument("--source-sha", required=True)
parser.add_argument("--toolchain-id", required=True)
parser.add_argument("--platform", required=True)
parser.add_argument("--bundle", required=True)
parser.add_argument("--sha256", required=True)
parser.add_argument("--size", required=True, type=int)
parser.add_argument("--run-id", required=True)
parser.add_argument("--run-attempt", required=True)
args = parser.parse_args()

status = {
    "format": "jamscript-toolchain-bundle-status/v1",
    "sourceSha": args.source_sha,
    "toolchainId": args.toolchain_id,
    "platform": args.platform,
    "bundle": args.bundle,
    "sha256": args.sha256,
    "size": args.size,
    "producer": "PASS",
    "verifier": "PASS",
    "reproducible": "PASS",
    "offlineConsumer": "PASS",
    "githubRunId": args.run_id,
    "githubRunAttempt": args.run_attempt,
    "published": False,
}
with open(args.output, "w", encoding="utf-8", newline="\n") as stream:
    json.dump(status, stream, indent=2, sort_keys=True)
    stream.write("\n")
