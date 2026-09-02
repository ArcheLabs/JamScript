# JamScript Toolchain Distribution v1

JamScript canonical builds resolve one immutable compiler distribution:

```text
JamScript CLI + source + target
  -> exact toolchain bundle
  -> deterministic service artifact
```

The distribution owns Node, LLVM/Clang, `llvm-ar`, `ld.lld`, Rust, `rust-src`,
ScriptC's prepared npm tree, the compiler/runtime source crates, Cargo's
vendored dependencies, and the pinned MiniJAM target SDK. It is described by
[`toolchains/distribution-v1.toml`](../toolchains/distribution-v1.toml) and
the Linux LLVM closure by
[`toolchains/llvm/linux-x86_64.lock`](../toolchains/llvm/linux-x86_64.lock).

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

1. Check out the exact JamScript and MiniJAM lock revisions.
2. Build the bundle with `tools/release/toolchain/build-linux.sh`.
3. Run `tools/release/toolchain/verify-bundle.sh`.
4. Publish the archive and internal manifest to a GitHub Release.
5. Promote the exact archive URL, SHA-256, and byte size in the distribution
   manifest in a separate commit.

The native bundle scope starts with `linux-x86_64`. Windows, macOS, and
Linux ARM bundles use the same manifest and cache model when published.
