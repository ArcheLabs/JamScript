# JamScript Toolchain Distribution v1

JamScript canonical builds resolve one immutable compiler distribution:

```text
JamScript CLI + source + target
  -> exact toolchain bundle
  -> deterministic service artifact
```

The distribution owns Node, LLVM/Clang, `llvm-ar`, `ld.lld`, Rust, `rust-src`,
ScriptC's prepared npm tree, the compiler/runtime source crates, Cargo's
vendored dependencies, and the JamScript-owned JAM target SDK. It is described by
[`toolchains/distribution-v1.toml`](../toolchains/distribution-v1.toml) and
the Linux LLVM closure by
[`toolchains/llvm/linux-x86_64.lock`](../toolchains/llvm/linux-x86_64.lock).
The Linux release path bootstraps the immutable LLVM 20.1.8 official
`LLVM-20.1.8-Linux-X64.tar.xz` archive into `$RUNNER_TEMP`; it does not use
Ubuntu's `clang-20` package as the canonical compiler. The archive SHA-256 and
the `clang`, `llvm-ar`, and `ld.lld` SHA-256 values are checked before and after
extraction, and `ldd` must report a complete LLVM runtime closure.

## User commands

```bash
jamscript build
jamscript build --offline
jamscript toolchain status --json
jamscript toolchain install
jamscript toolchain verify
jamscript toolchain path
jamscript doctor
```

Installation may use the network once. Compilation uses the installed bundle;
`--offline` and `JAMSCRIPT_OFFLINE=1` fail clearly when the expected bundle is
missing and never try to download it. A damaged bundle fails verification and
is never replaced by `/usr/bin/clang`, PATH `node`, rustup, or any other host
tool.

The cache is platform-specific and immutable:

```text
<cache>/scriptc-m2-v1/linux-x86_64/<bundle-sha256>/
```

`JAMSCRIPT_TOOLCHAIN_HOME` can relocate the cache for CI or enterprise
installations. `build.json` records the toolchain ID, platform, bundle SHA-256,
and `canonical_toolchain: true`; it never records a user's cache path.

## Contributor and release engineering mode

Contributors may explicitly use repository checkouts with
`JAMSCRIPT_DEV_TOOLCHAIN=1`. Such artifacts are marked
`canonical_toolchain: false` and are not valid release inputs. Docker is
allowed only around release engineering and cross-distro verification.

The release flow is:

1. Check out the exact JamScript revision.
2. Bootstrap and verify the exact LLVM distribution with
   `tools/release/toolchain/bootstrap-llvm-linux.sh`.
3. Build the bundle with `tools/release/toolchain/build-linux.sh`.
4. Run `tools/release/toolchain/verify-bundle.sh`.
5. Publish the archive and internal manifest to a GitHub Release.
6. Promote the exact archive URL, SHA-256, and byte size in the distribution
   manifest in a separate commit.

The hosted production gate is
[`build-toolchain-bundle.yml`](../.github/workflows/build-toolchain-bundle.yml).
It runs on Ubuntu 24.04, checks the exact source SHA and x86_64 architecture,
installs the locked ScriptC packages, builds and verifies two independent
`tar.zst` archives, and compares their bytes before uploading the validation
artifact `toolchain-linux-x86_64` for seven days. The artifact includes the
archive, `SHA256SUMS`, `bundle-status.json`, `bundle-metadata.json`, and the
internal toolchain manifest.

The workflow's candidate consumer smoke test uses a temporary manifest only in
an ephemeral source copy. It installs the archive through `ToolchainManager`,
then runs `jamscript build --offline` from a fresh cache. The checked-in
`published = false` record is never edited or promoted by Actions; an Actions
artifact is not a public distribution URL.

The bundle contains only the JamScript-owned JAM target SDK under
`targets/jam/sdk`; MiniJAM, Jambda, and deployment services are not bundled.

The native bundle scope starts with `linux-x86_64`. Windows, macOS, and
Linux ARM bundles use the same manifest and cache model when published.
