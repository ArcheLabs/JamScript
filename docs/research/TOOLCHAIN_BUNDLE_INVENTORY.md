# JamScript Toolchain Bundle Inventory

This is the closure contract for `scriptc-m2-v1` on `linux-x86_64`. The
producer's `manifest.json` is authoritative for the exact file count, hashes,
and unpacked byte set; `verify-bundle.py` rejects any missing or modified file.

## Required components

| Component | Required | Source of truth / validation |
| --- | --- | --- |
| Node | YES | `bin/node`, pinned `NODE_VERSION`, executable identity check |
| LLVM/Clang | YES | `bin/clang`, `bin/llvm-ar`, `bin/ar`, `bin/ld.lld`, locked hashes |
| Native shared libraries | YES | `build-linux.sh` ldd closure; executable version probes after install |
| Rust compiler | YES | `bin/rustc`, nightly identity, copied `lib/rustlib` |
| Cargo | YES | `bin/cargo`, bundled `cargo/config.toml` |
| rust-src | YES | `lib/rustlib/src/rust`, vendored standard-library sources |
| Cargo vendor | YES | `cargo/vendor`, offline guest build |
| ScriptC | YES | `scriptc/m2`, lock-resolved `node_modules`, package identity |
| ScriptC runtime | YES | `scriptc/node_modules/@scriptc/runtime` and bundled runtime sources |
| JAM SDK | YES | `targets/jam/sdk/include` and `targets/jam/sdk/src` |
| PolkaVM components | YES | bundled Rust dependencies, `toolchains/polkavm.lock`, target spec |
| MiniJAM checkout | NO | forbidden by target and consumer checks |
| Jambda | NO | not referenced or bundled |

The bundle layout is:

```text
manifest.json, Cargo.lock
bin/{node,clang,llvm-ar,ar,ld.lld,rustc,cargo}
lib/{native runtime closure,rustlib}
scriptc/{m2,node_modules,package-lock.json}
runtime/{Cargo.lock,crates}
runtime-scriptc/
targets/jam/sdk/{include/jam,src}
cargo/{config.toml,vendor}
toolchains/polkavm.lock
```

The installed consumer sets `CARGO_HOME` to an empty external directory. The
guest build receives the bundle's own Cargo home and the ScriptC child process
receives the bundle's `bin` directory, so bare `clang`/`ar` resolution cannot
fall through to the host. `doctor --json` asserts that every managed path is
under `JAMSCRIPT_TOOLCHAIN_HOME`.

## Audit commands

```bash
tools/release/toolchain/verify-bundle.sh <bundle.tar.zst>
tar --zstd -tf <bundle.tar.zst> | wc -l
du -sh <installed-bundle-root>
```

The verification workflow records the exact archive SHA-256, byte size, file
count, and unpacked size in its run summary. No release is promoted until the
two independent producer archives and the two consumer artifact sets agree.
