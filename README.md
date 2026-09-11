# JamScript

JamScript is a deterministic TypeScript-like application runtime for JAM
services. It compiles an external service project to a canonical PolkaVM/JAM
artifact and keeps MiniJAM and Jambda out of the compiler and release path.

## Supported platforms

The v0.1 release target matrix supports native `linux-x86_64` and native
`macos-arm64` (Apple Silicon). `windows-x86_64` is explicitly outside the v0.1
scope. macOS uses the official LLVM 20.1.8 ARM64 distribution and is produced
and validated on a native Apple Silicon runner; Rosetta is not part of the
support contract.

## Quick install

Choose an existing published version from the repository's GitHub Releases.
The failed `v0.1.0-rc.2` tag is retained for provenance and has no release
assets.

```bash
VERSION='v0.1.0-rc.N'
curl -fsSL \
  "https://raw.githubusercontent.com/ArcheLabs/JamScript/${VERSION}/install.sh" \
  | bash -s -- --version "${VERSION}"
```

Then:

```bash
jams doctor
jams --help
```

The installer does not modify shell profiles. If `~/.local/bin` is not on the
current shell's `PATH`, export it as shown by the installer.

## Manual installation

Download the target-specific CLI archive, managed toolchain bundle,
target-specific toolchain manifest and metadata, `SHA256SUMS`, and
`release-manifest.json` from the immutable GitHub Release tag. Verify the
checksums, extract the CLI, and install the pinned bundle:

```bash
sha256sum -c SHA256SUMS
tar -xzf jamscript-v0.1.0-linux-x86_64.tar.gz
./jams toolchain install
./jams doctor
```

On macOS, use `shasum -a 256 -c SHA256SUMS` and the matching
`jamscript-v0.1.0-macos-arm64.tar.gz` archive. The managed toolchain remains a
`.tar.zst` release asset, but the end-user CLI bootstrap does not require zstd.

The release archive embeds the exact toolchain URL and digest; no repository
checkout or developer toolchain is needed. The public executable is `jams`; the
release does not provide a `jamscript` compatibility alias.

See [`docs/installation.md`](docs/installation.md) for custom destinations,
PATH handling, retry, and manual uninstall details.

## Hello World

Create a project containing an entry TypeScript file and a `.jamscript`
service identity, then run:

```bash
./jams build ./hello --offline --output ./dist
./jams run ./dist/service.pvm
```

`run` executes the generated PVM artifact with the deterministic local
interpreter and prints `PVM_EXECUTION=PASS` on success.

## Build

The canonical build installs the managed bundle once and then compiles without
network access. `jams doctor` reports every resolved compiler path and
fails when a canonical build cannot be satisfied.

The release ABI uses a single typed descriptor for actions, managed state,
queries, and clients.

The supported path uses imports from the `jam` standard library, bounded
primitive input schemas, ABI generation, and generated `no_std` Rust for the
canonical JAM target. JamScript manages its compiler toolchain automatically: the
first canonical build installs the exact platform bundle and verifies its
checksum. `build` emits `service.blob`,
`service.polkavm`, `service.pvm`, and a portable Builder host artifact. The
production backend consumes the linked `service.pvm` through its persistent
PVM loader; the generated Builder artifact remains a legacy compatibility
artifact and is not a backend deployment dependency.

The v0 release boundary uses `SignedActionV1`: canonical bounded encoding,
payload commitments, ServiceKey identity, domain-separated sr25519
verification, sender derivation, expiry, and nonce-context validation.
Formal V1 is the first supported wire/runtime protocol; development generations
before it are not compatibility contracts.

The release ABI uses one typed descriptor and the canonical JAM `jam-codec
0.1.1` encoding rules; JamScript does not maintain an independent binary
codec. See
[`docs/release-conformance.md`](docs/release-conformance.md) for the type
system and canonical vectors.

The runtime layer provides a language-independent managed-state foundation:
SDK LayoutV1 trie roots and proofs, canonical diffs/transitions, managed wallet
nonce keys, proof-backed guest interfaces, and a reference host provider.
`jamscript-runtime` exposes the formal runtime wrapper.

```bash
cargo build --locked --bin jams
cargo run --locked --bin jams -- new hello-jam
cargo run --locked --bin jams -- check examples/counter
cargo run --locked --bin jams -- build examples/counter
cargo run --locked --bin jams -- toolchain status
cargo run --locked --bin jams -- doctor
```

## Run

The `run` command is a release validation aid for the generated
`service.pvm`.

## Deploy

Deployment is a separate, explicit step after artifact creation. Configure
named MiniJAM networks in `jamscript.toml`:

```toml
[deployment]
default_network = "local"

[networks.local]
kind = "minijam"
deployment_rpc = "http://127.0.0.1:8090"
node_rpc = "http://127.0.0.1:9944"
# Optional but recommended for identity verification.
genesis_hash = "0x0000000000000000000000000000000000000000000000000000000000000000"
```

Then inspect and deploy a verified artifact:

```bash
jams network list
jams build ./hello --output ./hello/dist
jams deploy ./hello --network local --artifact ./hello/dist
```

`jams deploy` supports MiniJAM Stage-1 `minijam_createServiceV1` only. It
verifies `service.blob`, `build.json`, and `checksums.json` before submitting,
checks the configured genesis identity before mutation, and writes a local
record under `.jamscript/deployments/`. JAM deployment is reserved for a
future release. See [`docs/deployment.md`](docs/deployment.md) for custom
RPCs, precedence rules, records, and the real-network E2E workflow.

## Toolchain model

JamScript owns Node, ScriptC, Rust, rust-src/compiler-builtins, LLVM/Clang,
PolkaVM linker inputs, vendored Cargo dependencies, and the JAM target SDK in
one digest-addressed, platform-specific bundle. The user does not need to
install Rust, Node, LLVM, Docker, or zstd for the canonical `jams build` path.
The separately compiled native Builder host adapter uses Apple's SDK / Command
Line Tools ABI boundary; the native release gate is the proof of that separate
host-linkage contract.

## Limitations

The v0.1 boundary is a testnet developer preview. Windows remains unsupported.
Mainnet economics, distributed providers, and generic PVM witness discovery
remain out of scope.

## Development and contribution

To run the optional cross-process MiniJAM compatibility path (it requires a
separate MiniJAM checkout):

    ./scripts/minijam-network-e2e.sh

To consume an already-running local MiniJAM Stage-1 provider without managing
its lifecycle:

    ./scripts/minijam-consumer-e2e.sh

If the default npm registry is unreachable, set `JAMSCRIPT_NPM_REGISTRY` for
that run, for example `https://registry.npmmirror.com`.

For contributors building from this repository, use
`JAMSCRIPT_DEV_TOOLCHAIN=1` with the repository's target SDK. Canonical user
builds use the managed bundle and do not require host Node, LLVM, Rust, Docker,
or a MiniJAM checkout. See
[`docs/toolchain-distribution.md`](docs/toolchain-distribution.md).

Managed-state architecture details are in
[`docs/service-runtime-architecture.md`](docs/service-runtime-architecture.md),
[`docs/managed-state.md`](docs/managed-state.md), and
[`docs/state-recovery.md`](docs/state-recovery.md).

The public type and codec references are in [`docs/type-system.md`](docs/type-system.md)
and [`docs/codec.md`](docs/codec.md).

Real applications built with JamScript are maintained in their product
repositories. JAM OS's canonical JNS service is one downstream consumer;
JamScript's own release gates use generic compiler and runtime fixtures.

The v0 testnet release boundary and operator workflow are documented in
[`docs/releasing.md`](docs/releasing.md).

The MiniJamSpec compatibility audit, including the pinned revisions and the
Refine/Accumulate ABI decision, is documented in
[`docs/minijam-spec-compatibility.md`](docs/minijam-spec-compatibility.md).
