# JamScript Formal V1 Testnet Developer Preview

JamScript v0 is released as a testnet developer preview. Its supported product path is:

```text
JamScript source
  -> canonical PolkaVM Service artifact
  -> generated, statically linked Builder application
  -> Formal Work RPC
  -> finalized managed-state commitment
  -> proof-verified client query
```

## Release boundary

The Formal V1 release freezes `SignedActionV1`, `RuntimeRefineInputV1`,
`RuntimeRefineOutputV1`, `ManagedStateWitnessV1`, the generated ABI/state descriptors, Polkadot
`LayoutV1<Blake2Hasher>`, managed-state recovery v1, and Builder artifact v1. A release bundle
contains `protocol-v0.json` and `checksums.json`; `jams inspect <bundle>` verifies every
listed artifact before displaying its metadata.

MiniJamSpec compatibility is an execution-boundary property. JamScript does
not target JAM FullSpec directly and does not embed MiniJamSpec constants. It
ships the JAM target ABI and its own target SDK. MiniJAM compatibility is
checked separately by the optional downstream network E2E; a MiniJAM or
Jambda checkout is not part of a JamScript build or release input.

The Builder/Provider process is deployed per Service. It statically compiles the generated host
application and the same native C sources used by the PVM Service. Loading arbitrary native
libraries into a shared daemon is not supported.

Provider persistence is enabled with `JAMSCRIPT_PROVIDER_STORE`. The append-only recovery log is
replayed and cryptographically revalidated on startup. Finalized JAM/MiniJAM storage remains the
only source of canonical roots; the Provider supplies data and proofs for explicit roots.

## Operator workflow

The compiler distribution is a JamScript responsibility. Release engineering
produces versioned, checksum-addressed bundles with the native
[`build-linux.sh`](../tools/release/toolchain/build-linux.sh) and
[`build-macos.sh`](../tools/release/toolchain/build-macos.sh) producers and
publishes them as GitHub Release assets. Users install JamScript and run
`jams build`; Docker, LLVM, Rust, Node, and a MiniJAM checkout are not user
requirements for that canonical service build. A separately compiled native
Builder host adapter may require Apple's SDK / Command Line Tools. The
candidate distribution workflow is
[`build-toolchain-bundle.yml`](../.github/workflows/build-toolchain-bundle.yml).
It has separate native Linux x86_64 and macOS arm64 producers, builds and
verifies two identical archives per target, and uploads short-lived Actions
validation artifacts. The tag workflow
[`release-candidate.yml`](../.github/workflows/release-candidate.yml) is the
only publication path; there is no mutable “latest toolchain” workflow.

The checked-in distribution record is intentionally marked unpublished until
each release bundle has been built and its exact SHA-256 and byte size promoted
into `toolchains/distribution-v1.toml`. This prevents a floating or guessed
compiler identity from entering a canonical build.

The hosted bootstrap addresses the historical Ubuntu runner failure recorded
as Actions run `33781367304`, job `100735570690`: the first bundle build stopped
because the runner supplied LLVM/Clang `20.1.2` while the release contract
required `20.1.8`. The canonical fix is the JamScript-owned official LLVM
20.1.8 archive lock, verified before extraction and injected by absolute tool
paths. The historical failure remains part of the release record and is not
rewritten as a successful run.

1. Build the Service with `jams build`.
2. Verify the deployment bundle with `jams inspect <bundle>`.
3. Provision or upgrade the Service through the network operator's deployment control plane.
4. Compile and run the generated Builder application as a per-Service Formal RPC sidecar.
5. Configure the browser client with separate node, work, and managed-state Provider endpoints.
6. When a downstream network is available, run the manual MiniJAM network E2E
   as a compatibility check before publishing the release artifacts.

Service provisioning is intentionally not exposed as a fake application RPC. MiniJAM currently
has no formal deployment RPC equivalent to the Work RPC, so v0 deployment remains an explicit
operator action. Wallet calls remain in the TypeScript/browser client so the wallet receives one
standard `signRaw` request and private keys never enter the CLI.

## Release gates

The tag workflow [`release-candidate.yml`](../.github/workflows/release-candidate.yml)
builds native CLI archives and managed bundles for Linux x86_64 and macOS arm64
from the exact tag commit. It writes one immutable `release-manifest.json` and
complete `SHA256SUMS`, runs native pre-publication clean-consumer tests, and
then allows exactly one publication job to create the GitHub Release. It
refuses to replace an existing tag's assets. After publication, separate native
jobs download the published bytes and run
[`JamScript Release Kill Test 001`](../scripts/release/release-kill-test-001.sh)
against the release URL; the local asset test never substitutes for this R4
check.

For `v0.1.0-rc.*`, publication passes both `--prerelease` and
`--latest=false` to GitHub CLI. The stable `v0.1.0` path does not set
`--prerelease`; an existing release is always rejected before upload.

The kill test starts with isolated `HOME`, Cargo, Rustup, and JamScript cache
directories. It hides host Rust, Cargo, rustup, Node, npm, Clang, LLVM, and Zig
behind a restricted `PATH`, installs the digest-pinned bundle, runs `doctor`,
builds an external fixture twice with network disabled, executes the resulting
PVM artifact, and compares the two artifact hashes. It runs natively on both
Linux x86_64 and macOS arm64; the macOS native Builder linkage is where the
Apple SDK / Xcode Command Line Tools boundary is exercised. Its JSON result is
the machine-readable R1/R4 decision.

The explicit
[`compiler-builtins-regression.sh`](../scripts/release/compiler-builtins-regression.sh)
keeps the earlier offline failure covered: the managed `rust-src` tree must
contain `compiler-builtins`, and the same isolated execution-closure probe
must build a PolkaVM cdylib guest. The old failure occurred when the consumer
scan treated binary PVM files as text; the current gate verifies execution and
managed paths instead of grepping generated binaries.

`toolchains/release-targets.toml` is the authoritative v0 platform matrix.
Linux x86_64 and macOS arm64 are supported only with matching native producers,
immutable assets, and native kill-test evidence. Windows is outside this
release scope.

## Promotion protocol

The pre-installer tag `v0.1.0-rc.1` remains immutable. The intended sequence
for this installer release is branch validation, merge to `main`, main
validation, then tag `v0.1.0-rc.2`. The tag workflow checks out the exact source, builds
the managed toolchain and `jams` CLI, runs the compiler-builtins regression,
assembles the immutable assets, runs Release Kill Test 001 with `--asset-dir`,
and only then creates the GitHub prerelease. A fresh runner downloads the
published bytes and runs the same test with `--release-url` before the workflow
emits `RELEASE_READY`.
The checked-in distribution record stays unpublished until a reviewed release
promotion records the exact URL, digest, and byte size; changing it to
`published = true` without those bytes is rejected by the toolchain manager.

The installer published in user documentation must be fetched from the exact
release tag and request that same release version. Since `v0.1.0-rc.1` already
exists, the installer will ship in `v0.1.0-rc.2`; its RC path uses
`v0.1.0-rc.2/install.sh` and `--version v0.1.0-rc.2`. Documentation must not
combine a mutable branch installer with a different release tag.

## Explicit exclusions

The preview does not claim mainnet readiness. User gas payment, sponsorship, DoS economics,
distributed Provider replication, garbage collection, generic PVM-only witness discovery, and
cross-Service managed state remain outside the v0 scope.
