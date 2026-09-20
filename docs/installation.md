# JamScript installation

## Quick install

```bash
curl -fsSL https://install.minijam.xyz/jamscript | bash
```

With no arguments, the installer selects the newest published JamScript release,
including RC/prerelease releases. It then installs three matching components:

1. `jams`, the JamScript CLI;
2. the managed compiler/toolchain used by `jams build`;
3. the native `jamscript-service-backend` from the matching
   `backend-<JamScript version>` release.

The CLI and backend remain independently released artifacts. The installer only
combines them into one developer-facing installation flow.

The installer verifies the SHA-256 checksum published with each release before
installing an executable.

## Supported platforms

JamScript v0.1 currently provides native release assets for:

- Linux x86_64 (`linux-x86_64`)
- macOS Apple Silicon (`macos-arm64`)

Windows is outside the current v0.1 release scope.

## Pin a version

For reproducible environments, pass an immutable release tag:

```bash
curl -fsSL https://install.minijam.xyz/jamscript \
  | bash -s -- --version v0.1.0-rc.7
```

The matching backend release is resolved as `backend-v0.1.0-rc.7`.

A custom binary directory can be selected with:

```bash
curl -fsSL https://install.minijam.xyz/jamscript \
  | bash -s -- --bin-dir "$HOME/bin"
```

The default destination is `~/.local/bin`. The installer does not use `sudo`
and does not edit shell profiles.

## Requirements

The bootstrap installer needs Bash, curl, tar, gzip, awk, and either
`sha256sum` or macOS `shasum`.

It does **not** require a repository checkout or a preinstalled Rust, Cargo,
Node, LLVM, Docker, or zstd toolchain. JamScript installs and verifies its
managed compiler/toolchain itself.

## Verify

```bash
jams --help
jams toolchain verify
```

For a configured local MiniJAM network:

```bash
jams backend start --network local
```

`jams backend start` finds the backend installed next to the `jams` executable.
The existing `PATH` and `JAMSCRIPT_BACKEND_BIN` overrides remain available for
custom backend installations.

## Manual installation

Users who do not want to pipe a script into Bash can install manually:

1. Download the target-specific JamScript CLI archive and `SHA256SUMS` from the
   chosen `v...` GitHub Release.
2. Download the target-specific backend archive and `SHA256SUMS` from the
   matching `backend-v...` GitHub Release.
3. Verify both checksums.
4. Install `jams` and `jamscript-service-backend` into a directory on `PATH`.
5. Run `jams toolchain install` and `jams toolchain verify`.

The managed toolchain remains a digest-addressed `.tar.zst` release asset
consumed by the CLI; users do not need to unpack it manually.

## Retry and uninstall

If the CLI/backend installation succeeds but toolchain installation fails:

```bash
jams toolchain install
jams toolchain verify
```

Re-running the installer is safe and replaces the installed executables only
after downloaded artifacts pass checksum and archive validation.

To remove the installed executables from the default location:

```bash
rm -f ~/.local/bin/jams ~/.local/bin/jamscript-service-backend
```

The managed toolchain cache can be removed separately after checking its path
with:

```bash
jams toolchain path
```

## macOS SDK boundary

`jams build` compiles the canonical JAM guest/service path with the managed
LLVM, Rust, ScriptC, vendored dependencies, and JAM SDK in the bundle. It does
not compile the generated Builder host application.

The generated Builder/native host adapter remains a compatibility/test artifact.
If a user separately compiles that adapter, the host binary links against the
Apple arm64 ABI and therefore needs the macOS SDK/Xcode Command Line Tools (or
an explicitly supplied `SDKROOT`). Those Apple components are not bundled by
JamScript.
