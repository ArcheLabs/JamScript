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

Backend persistence is enabled with `JAMSCRIPT_BACKEND_DATA`. RocksDB `0.25.0`
stores the registry, per-Service KV, durable heads, and finalized transitions
under `db/`; content-addressed PVM artifacts remain under `artifacts/`. Startup
binds the database to schema v1 and the finalized network genesis, rebuilds
proof caches from durable KV, and validates every Service root. Finalized
JAM/MiniJAM storage remains the only source of canonical roots. The old
append-only recovery log is not part of the production persistence path.

## Operator workflow

The compiler distribution is a JamScript responsibility. Release engineering
produces versioned, checksum-addressed bundles with the native
[`build-linux.sh`](../tools/release/toolchain/build-linux.sh) and
[`build-macos.sh`](../tools/release/toolchain/build-macos.sh) producers and
publishes them as GitHub Release assets. Users install JamScript and run
`jams build`; Docker, LLVM, Rust, Node, and a MiniJAM checkout are not user
requirements for that canonical service build. A separately compiled native
Builder host adapter may require Apple's SDK / Command Line Tools. The
the single manual release workflow is
[`release.yml`](../.github/workflows/release.yml).
It has separate native Linux x86_64 and macOS arm64 producers, builds and
verifies two identical archives per target, and uploads short-lived Actions
validation artifacts. On branches and pull requests, this expensive workflow
is path-filtered to compiler, toolchain, release-engineering, and validation
inputs; documentation-only changes do not rebuild native bundles. Maintainers
can force it at any time with `workflow_dispatch`. It runs only from `main`,
binds every job to the dispatch SHA, and publishes only the validated A bytes;
there is no mutable “latest toolchain” workflow. The normal CI workflow has a
small Linux/macOS producer smoke but never publishes a bundle.

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

## JamScript release gates

The [`release.yml`](../.github/workflows/release.yml) workflow is deliberately
minimal. It has exactly four jobs: `validate`, `build-toolchain`, `build-cli`,
and `publish`. Each native toolchain and CLI archive is built once from the
exact dispatch SHA. Release-time checks only verify archive structure, required
files, exact filenames, and `SHA256SUMS`.

The public release contains two CLI archives, two managed toolchain archives,
and `SHA256SUMS`. It does not publish `release-manifest.json`, toolchain
engineering metadata, Backend assets, Docker images, or GHCR references.
The workflow finishes with `JAMSCRIPT_RELEASE_PUBLISHED=PASS`; all consumer,
compiler, SDK, guest, determinism, and host-environment correctness checks
belong to CI.

The standalone
[`JamScript Release Kill Test 001`](../scripts/release/release-kill-test-001.sh)
is retained for manual release-engineering diagnosis and is not a release gate.

`toolchains/release-targets.toml` is the authoritative v0 platform matrix.
Linux x86_64 and macOS arm64 are supported only with matching native producers,
immutable assets, and native consumer evidence. Windows is outside this
release scope.

## Promotion protocol

Run `Release JamScript` manually from `main` with only the intended version,
for example `v0.1.0-rc.3`. The workflow validates the generic semver and
source identity, builds the release bytes once, checks the exact five-file
public asset set, creates and pushes the annotated tag, and publishes the exact
same bytes. It does not rerun consumer or correctness tests.

## Independent Backend release

The backend is a separately versioned deployable service. Run
`Release Backend` with a tag such as `backend-v0.1.0-rc.1`. Its workflow builds
only the native backend artifacts, creates `backend-manifest.json`, performs
native and Docker health/readiness plus volume-restart checks, publishes the
backend GitHub Release and GHCR image, and emits `BACKEND_RELEASE_READY=PASS`.
Backend Docker failures therefore do not block a JamScript language/toolchain
release.
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
