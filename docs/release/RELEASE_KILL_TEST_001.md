# JamScript Release Kill Test 001

This is the canonical external-consumer test for v0.1. It answers one
question: can a developer with no JamScript development environment download
the official release and compile and execute a program?

JamScript is the product name. `jams` is the public executable name; the
release contains no `jamscript` compatibility executable or alias.

The implementation is
[`scripts/release/release-kill-test-001.sh`](../../scripts/release/release-kill-test-001.sh).
It accepts `--target linux-x86_64` or `--target macos-arm64` and must run on the
matching native host. The bootstrap phase reads the target-specific CLI,
managed toolchain, toolchain manifest, metadata, `release-manifest.json`, and
`SHA256SUMS`. It verifies the acquired target bytes and the per-target digests
embedded in the release manifest. After extraction it requires an executable
`jams` and rejects any `jamscript` executable, so the archive structure itself
enforces the public command rename.

The build phase uses isolated `HOME`, Cargo, Rustup, and XDG cache directories,
sets `CARGO_NET_OFFLINE` through the canonical builder, and restricts `PATH` so
host Rust, Cargo, rustup, Node, npm, Clang, LLVM, and Zig cannot be resolved.
The managed bundle is the only compiler input. An external hello fixture is
built twice, the generated `service.pvm` is executed with `jams run`, and
the two artifact hashes must match. On macOS this also proves that the native
Builder can link against the Apple SDK / Command Line Tools ABI boundary.

The script writes a machine-readable result:

```json
{
  "protocol": "JAMSCRIPT_RELEASE_KILL_001",
  "status": "PASS",
  "gates": {
    "R1_clean_consumer_e2e": true,
    "R2_managed_toolchain_published": true,
    "R3_immutable_github_release": true,
    "R4_published_artifact_validation": true
  }
}
```

An `--asset-dir` run is useful for local producer debugging. It leaves the
publication and released-artifact gates false because local files are not proof
of GitHub Release publication. The release workflow runs the same protocol
against the public release URL after upload; the JSON status describes the kill
test itself, while the overall release decision remains conjunctive.
