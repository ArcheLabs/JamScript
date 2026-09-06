# JamScript installation

## Supported platform

The v0.1 installer supports Linux x86_64 only. Windows and macOS Apple Silicon
are not supported by this release.

## Quick install

The immutable RC installer source and requested release are the same tag:

```bash
curl -fsSL \
  https://raw.githubusercontent.com/ArcheLabs/JamScript/v0.1.0-rc.2/install.sh \
  | bash -s -- --version v0.1.0-rc.2
```

The installer downloads the CLI archive from the GitHub Release, verifies its
SHA-256 entry, installs `jams` atomically, then runs:

```bash
jams toolchain install
jams doctor
```

The managed toolchain remains owned and verified by JamScript. The installer
does not require Rust, Cargo, Node, LLVM, or a repository checkout.

## Manual installation

For users who do not want to pipe a script into Bash:

1. Download the matching `jamscript-<VERSION>-linux-x86_64.tar.zst`,
   `SHA256SUMS`, managed toolchain bundle, `toolchain-manifest.json`, and
   `release-manifest.json` from the immutable GitHub Release tag.
2. Verify the downloaded files with `sha256sum -c SHA256SUMS`.
3. Extract the CLI archive with
   `tar --zstd -xf jamscript-<VERSION>-linux-x86_64.tar.zst`.
4. Install the extracted `jams` into a directory on `PATH`, for example
   `~/.local/bin`, without adding a `jamscript` compatibility alias.
5. Run `jams toolchain install`, then `jams doctor`.

## Options and PATH

The installer accepts `--version VERSION`, `--bin-dir DIR`, and `--help`.
The default CLI destination is `~/.local/bin/jams`; no `sudo` is used and no
shell profile is changed. A custom destination can be selected with:

```bash
install.sh --version v0.1.0-rc.2 --bin-dir "$HOME/bin"
```

If the destination is not currently on `PATH`, add it for the current shell:

```bash
export PATH="$HOME/.local/bin:$PATH"
```

## Retry, reinstall, and uninstall

If the CLI is installed but the managed toolchain download fails, retry with:

```bash
~/.local/bin/jams toolchain install
~/.local/bin/jams doctor
```

Re-running the installer is safe and re-verifies the CLI before replacement.
To inspect the managed toolchain before uninstalling the CLI, run:

```bash
jams toolchain path
```

Manual CLI uninstall is:

```bash
rm -f ~/.local/bin/jams
```
