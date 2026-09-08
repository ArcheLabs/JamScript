# JamScript Toolchain Distribution v1

JamScript canonical builds resolve one immutable compiler distribution per
native target:

```text
JamScript CLI + source + target
  -> exact platform bundle
  -> deterministic service artifact
```

The distribution owns Node, LLVM/Clang, `llvm-ar` (also exposed as the
ScriptC-compatible `ar` command), `ld.lld`, `llvm-readelf`, Rust, rust-src and
compiler-builtins, ScriptC's prepared npm tree, compiler/runtime source crates,
Cargo's vendored dependencies, and the JAM target SDK. It is described by
[`toolchains/distribution-v1.toml`](../toolchains/distribution-v1.toml).

## Platform boundary

The public IDs are deliberately stable:

| Public ID | Native producer | LLVM lock | Status |
| --- | --- | --- | --- |
| `linux-x86_64` | Ubuntu x86_64 | `toolchains/llvm/linux-x86_64.lock` | supported |
| `macos-arm64` | macOS 15 Apple Silicon | `toolchains/llvm/macos-arm64.lock` | supported |
| `windows-x86_64` | none in v0.1 | none | unsupported |

Rust's `aarch64` macOS host name maps to `macos-arm64`; Rosetta is not used as
a substitute for a native producer.

Linux bootstraps the official LLVM 20.1.8
`LLVM-20.1.8-Linux-X64.tar.xz`; macOS bootstraps the official
`LLVM-20.1.8-macOS-ARM64.tar.xz`. Each archive URL and archive digest is
locked, and each native producer measures the compiler binary digests before
creating a bundle. The macOS binary-digest fields remain explicit native
measurement sentinels in the source lock until the first macOS producer run
promotes the measured values; a release bundle is accepted only when its
internal manifest contains the measured values.

On a native Apple Silicon checkout, the measured values can be promoted into
the reviewed source lock with
`tools/release/toolchain/promote-llvm-macos.sh <llvm.env>`. The v0.1 release
workflow refuses publication while those three fields remain sentinels.

## User commands

```bash
jams build
jams build --offline
jams toolchain status --json
jams toolchain install
jams toolchain verify
jams toolchain path
jams doctor
```

Installation may use the network once. Compilation uses the installed bundle;
`--offline` and `JAMSCRIPT_OFFLINE=1` fail clearly when the expected bundle is
missing and never try to download it. A damaged bundle fails verification and
is never replaced by `/usr/bin/clang`, PATH `node`, rustup, or another host
tool.

The CLI bootstrap archive is `.tar.gz` so Linux and macOS can install it with
stock tar and gzip. The managed compiler bundle is `.tar.zst`; zstd is an
internal release/CLI implementation detail and is not an end-user
prerequisite.

The cache is platform-specific and immutable:

```text
<cache>/scriptc-m2-v1/<platform>/<bundle-sha256>/
```

`JAMSCRIPT_TOOLCHAIN_HOME` can relocate the cache for CI or enterprise
installations. `build.json` records the toolchain ID, platform, bundle SHA-256,
and `canonical_toolchain: true`; it never records a user's cache path.

## Contributor and release engineering mode

Contributors may explicitly use repository checkouts with
`JAMSCRIPT_DEV_TOOLCHAIN=1`. Such artifacts are marked
`canonical_toolchain: false` and are not valid release inputs. Docker is
allowed only around release engineering and cross-distro verification; it is
not a user build dependency.

The native producer flow is:

1. Check out the exact JamScript revision on the matching native runner.
2. Bootstrap and verify the locked LLVM distribution with the platform-specific
   `bootstrap-llvm-*` and `verify-llvm-*` scripts.
3. Build the bundle with `build-linux.sh` or `build-macos.sh`.
4. Run `verify-bundle.sh`, the managed execution-closure probe, and the
   compiler-builtins regression.
5. Build two independent archives and compare their bytes.
6. Pass both native clean-consumer kill tests before publication.

The candidate workflow is
[`build-toolchain-bundle.yml`](../.github/workflows/build-toolchain-bundle.yml).
It has separate Linux and macOS producers, checks exact source identity and
native architecture, validates the managed bundle, and uploads short-lived
engineering evidence. An Actions artifact is not a public distribution URL.

## Native ABI and SDK boundary

The managed bundle is compiler-toolchain self-contained on both supported
targets. It does not require host-installed Rust, Cargo, Node, LLVM, Clang,
LLD, ScriptC, or a MiniJAM checkout.

The guest/service path uses the managed target SDK and managed tools. `jams
build` does not compile the separate generated Builder host application. When
that host adapter is compiled, native host binaries still use the host ABI:
Linux uses the Ubuntu/glibc boundary and macOS uses the Apple arm64 loader,
system frameworks, and SDK / Xcode Command Line Tools for host linkage. Those
Apple components are not copied into the bundle. The native macOS execution
closure and clean-consumer tests are the release proof for this boundary.

The bundle contains only the JamScript-owned JAM target SDK under
`targets/jam/sdk`; MiniJAM, Jambda, and deployment services are not bundled.

## Published bytes and consumer validation

The tag workflow
[`release-candidate.yml`](../.github/workflows/release-candidate.yml) builds
both CLI archives and both managed bundles from the exact tag commit. It
assembles one `release-manifest.json` with two supported target entries and an
explicit unsupported Windows entry, plus target-specific toolchain manifests,
metadata, and a complete `SHA256SUMS` index.

The workflow has one publication job. Before it can run, native Linux and
native macOS clean-consumer jobs download the exact assembled bytes and run
[`release-kill-test-001.sh`](../scripts/release/release-kill-test-001.sh).
After publication, separate native jobs repeat the same test against the
GitHub Release URL. The release refuses to replace an existing tag.
