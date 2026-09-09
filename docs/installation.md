# JamScript installation

## Supported platforms

JamScript v0.1 provides native release assets for:

- Linux x86_64 (`linux-x86_64`)
- macOS Apple Silicon (`macos-arm64`)

Windows x86_64 is explicitly outside the v0.1 release scope. macOS support
means a native arm64 process on Apple Silicon; Rosetta is not a supported
release path.

## Quick install

Choose an existing published version from GitHub Releases and keep the
installer URL and requested release on that same immutable tag. The failed
`v0.1.0-rc.2` tag is retained for provenance and has no release assets.

```bash
VERSION='v0.1.0-rc.N'
curl -fsSL \
  "https://raw.githubusercontent.com/ArcheLabs/JamScript/${VERSION}/install.sh" \
  | bash -s -- --version "${VERSION}"
```

The installer detects the host platform, downloads the matching gzip CLI
archive, verifies its SHA-256 entry, installs `jams` atomically, then runs:

```bash
jams toolchain install
jams doctor
```

The user-facing bootstrap needs Bash, curl, tar, gzip, and either `sha256sum`
or macOS `shasum`. It does not require Rust, Cargo, Node, LLVM, zstd, Docker,
or a repository checkout. The managed compiler bundle is downloaded and
verified by JamScript itself.

The installer does not modify shell profiles. If `~/.local/bin` is not on the
current shell's `PATH`, export it as shown by the installer:

```bash
export PATH="$HOME/.local/bin:$PATH"
```

## Manual installation

For users who do not want to pipe a script into Bash:

1. Download the target-specific CLI archive, managed toolchain bundle,
   target-specific toolchain manifest and metadata, `SHA256SUMS`, and
   `release-manifest.json` from the same immutable GitHub Release tag.
2. Verify the downloaded files with `sha256sum -c SHA256SUMS` on Linux or
   `shasum -a 256 -c SHA256SUMS` on macOS.
3. Extract the CLI archive with `tar -xzf jamscript-<VERSION>-<TARGET>.tar.gz`.
4. Install the extracted `jams` into a directory on `PATH`, for example
   `~/.local/bin`, without creating a `jamscript` compatibility alias.
5. Run `jams toolchain install`, then `jams doctor`.

The managed toolchain release asset remains `.tar.zst` because it is an
internal, digest-addressed bundle consumed by the CLI. End users do not need
to invoke zstd or unpack that bundle manually.

## Options and cache

The installer accepts `--version VERSION`, `--bin-dir DIR`, and `--help`. The
default CLI destination is `~/.local/bin/jams`; no `sudo` is used and no shell
profile is changed. A custom destination can be selected with:

```bash
./install.sh --version <VERSION> --bin-dir "$HOME/bin"
```

The managed bundle is cached under a platform-specific, SHA-256-addressed
directory. `JAMSCRIPT_TOOLCHAIN_HOME` can relocate it for CI or enterprise
installations. `jams toolchain path` prints the selected cache path.

## Retry, reinstall, and uninstall

If the CLI is installed but the managed toolchain download fails, retry with:

```bash
~/.local/bin/jams toolchain install
~/.local/bin/jams doctor
```

Re-running the installer is safe and re-verifies the CLI before replacement.
To uninstall the CLI manually:

```bash
rm -f ~/.local/bin/jams
```

The managed toolchain cache can be removed separately after checking its path
with `jams toolchain path`.

## macOS SDK boundary

`jams build` compiles the canonical JAM guest/service path with the managed
LLVM, Rust, ScriptC, vendored dependencies, and JAM SDK in the bundle. It does
not compile the generated Builder host application. The generated Builder and
its native adapter are legacy compatibility/test artifacts; they are not
required by the production backend. If a user separately compiles
`generated_builder_application.rs` into that adapter, the host binary links against Apple's arm64 ABI and therefore needs
the macOS SDK / Xcode Command Line Tools (or an explicitly supplied `SDKROOT`).
Those Apple components are not bundled or redistributable by JamScript. The
native release closure and clean-consumer gate test this boundary explicitly;
JamScript does not claim that the separate host adapter has zero OS SDK
prerequisites.
