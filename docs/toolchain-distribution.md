# JamScript Toolchain Distribution v1

JamScript canonical builds resolve one immutable compiler distribution per
native target:

```text
JamScript CLI + source + target
  -> exact platform bundle
  -> deterministic service artifact
```

The distribution owns managed Node, LLVM/Clang, `llvm-ar`, one guest linker,
ScriptC's prepared npm tree, precompiled guest/runtime archives, the locked
PolkaVM target specification, and the JAM target SDK. It is described by
[`toolchains/distribution-v1.toml`](../toolchains/distribution-v1.toml).

Release engineering may use Rust and Cargo to produce the immutable guest
runtime archives. They are producer-only inputs and are not shipped to
consumer projects.

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
jams toolchain install --archive /path/to/toolchain.tar.zst
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
4. Run the structural archive checks; compiler correctness and managed guest
   closure checks run in CI or Toolchain Maintenance.
5. Publish only the exact archive bytes and their `SHA256SUMS` entry. Backend
   artifacts are produced and released by the independent Backend workflow.

The release workflow is
[`release.yml`](../.github/workflows/release.yml). It has separate native
Linux and macOS producers, checks exact source identity and native
architecture, builds each toolchain and CLI once, and uploads only the exact
producer bytes as release input. An Actions artifact is not a public
distribution URL.

## Native ABI and SDK boundary

The managed bundle is compiler-toolchain self-contained on both supported
targets. It does not require host-installed Rust, Cargo, Node, LLVM, Clang,
LLD, ScriptC, or a MiniJAM checkout. A consumer build uses only managed Node,
Clang, `llvm-ar`, the guest linker, ScriptC, and these immutable archives:

```text
runtime/libjamscript_guest_runtime.a
runtime/libjamscript_scriptc_runtime.a
runtime/libjamscript_jam_runtime.a
```

The consumer path is therefore:

```text
service.ts -> ScriptC -> generated C + descriptor C -> managed clang
           -> precompiled runtime archives + guest linker -> service.pvm
```

It does not invoke `cargo`, `rustc`, `rust-src`, Cargo vendor, host Rust
linkers, GCC, or host development libraries. The legacy Cargo guest path is
retained only for contributor/migration parity and is not selected for an
installed managed toolchain.

The guest/service path uses the managed target SDK and managed tools. `jams
build` emits a bounded Service Descriptor C object and compiles only
service-specific C before the final guest link. No host Builder application,
host proc-macro, or host Rust linkage is part of the consumer build.

The bundle contains only the JamScript-owned JAM target SDK under
`targets/jam/sdk`; MiniJAM, Jambda, and deployment services are not bundled.

## Published bytes and consumer validation

The manual JamScript release workflow
[`release.yml`](../.github/workflows/release.yml) builds both CLI archives and
both managed bundles from the exact dispatch commit. The public release has
exactly those four files plus `SHA256SUMS`; `release-manifest.json`, toolchain
engineering metadata, Backend archives, Docker, and GHCR are not part of the
public JamScript release. Backend artifacts are owned by
[`backend-release.yml`](../.github/workflows/backend-release.yml).

The release refuses to replace an existing GitHub Release. CI validates the
installed-bundle closure and the released consumer topology without exposing
the producer Rust/Cargo toolchain to the consumer environment.
