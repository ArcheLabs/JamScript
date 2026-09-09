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

The public backend is one multi-Service process. Each deployment is registered
by `serviceId`, `serviceKey`, `codeHash`, ABI version, and a digest-addressed
planner/application artifact. The backend registry routes work and proof
construction by Service identity; deploying another Service does not require
rebuilding the daemon. The production artifact is the content-addressed PVM;
the older generated Builder adapter is a legacy MiniJAM compatibility wrapper.

Backend persistence is enabled with `JAMSCRIPT_BACKEND_DATA`. The registry,
content-addressed PVM artifacts, and append-only recovery log are replayed and
cryptographically revalidated on startup. Finalized JAM/MiniJAM storage
remains the only source of canonical roots; the Provider supplies data and
proofs for explicit roots.

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
validation artifacts. On branches and pull requests, this expensive workflow
is path-filtered to compiler, toolchain, release-engineering, and validation
inputs; documentation-only changes do not rebuild native bundles. Maintainers
can force it at any time with `workflow_dispatch`. The tag workflow
[`release-candidate.yml`](../.github/workflows/release-candidate.yml) is the
only publication path; there is no mutable “latest toolchain” workflow.
Release tags are never path-filtered: the tag workflow always rebuilds and
validates the exact tagged source before immutable publication, rather than
reusing a branch validation artifact.

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
3. Select a named MiniJAM network and deploy with `jams deploy --network <name>`.
4. Run one `jamscript-service-backend` process and configure its internal Node and Formal RPCs.
5. Let the backend discover each deployed Service and prewarm its PVM artifact.
6. Configure the browser client with the single public backend endpoint.
7. When a downstream network is available, run the manual MiniJAM network E2E
   as a compatibility check before publishing the release artifacts.

The deployment command is intentionally a separate control-plane operation, not an application
RPC. In v0.1 it targets the formal MiniJAM Stage-1
`minijam_createServiceV1` method, verifies the artifact and optional node genesis identity before
mutation, waits for the finalized result returned by the deployment RPC, and stores a local
deployment record. Wallet calls remain in the TypeScript/browser client so the wallet receives one
standard `signRaw` request and private keys never enter the CLI. JAM deployment is recognized in
configuration but remains unsupported in v0.1.

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

The release candidate workflow is never a substitute for preflight. The
manual [`release-preflight.yml`](../.github/workflows/release-preflight.yml)
must first validate one exact candidate ref and emit
`RELEASE_PREFLIGHT_READY=PASS`. Only then may an operator create and push the
next immutable candidate tag. The tag workflow checks out that exact tag,
rebuilds the managed toolchain and `jams` CLI, runs the compiler-builtins
regression, assembles immutable assets, runs Release Kill Test 001 with
`--asset-dir`, and only then publishes the GitHub prerelease. Fresh native
consumers download the published bytes and run the same test with
`--release-url` before the workflow emits `RELEASE_READY=PASS`.
The checked-in distribution record stays unpublished until a reviewed release
promotion records the exact URL, digest, and byte size; changing it to
`published = true` without those bytes is rejected by the toolchain manager.

The installer published in user documentation must be fetched from the exact
published release tag and request that same release version. The retained
failed tag `v0.1.0-rc.2` has no GitHub Release assets; documentation must not
construct download URLs for it or combine a mutable branch installer with a
different release tag.

See [`release-process.md`](release-process.md) for the release state machine,
rerun rules, and failure classification.

## Explicit exclusions

The preview does not claim mainnet readiness. User gas payment, sponsorship,
DoS economics, distributed Provider replication, and garbage collection remain
outside the v0 scope. Cross-Service managed state follows the frozen
proof/dependency protocol; application DSL surface coverage remains narrow in
this preview.
