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
contains `protocol-v0.json` and `checksums.json`; `jamscript inspect <bundle>` verifies every
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
produces versioned, checksum-addressed bundles with
[`tools/release/toolchain/build-linux.sh`](../tools/release/toolchain/build-linux.sh)
and publishes them as GitHub Release assets. Users install JamScript and run
`jamscript build`; Docker, LLVM, Rust, Node, and a MiniJAM checkout are not
user requirements. The candidate distribution workflow is
[`build-toolchain-bundle.yml`](../.github/workflows/build-toolchain-bundle.yml).
It produces and verifies two identical Linux x86_64 archives and uploads a
short-lived Actions validation artifact. The separate
[`publish-toolchain.yml`](../.github/workflows/publish-toolchain.yml) workflow
is reserved for an explicit reviewed Release promotion.

The checked-in distribution record is intentionally marked unpublished until
the first bundle has been built and its exact SHA-256 and byte size promoted
into `toolchains/distribution-v1.toml`. This prevents a floating or guessed
compiler identity from entering a canonical build.

The hosted bootstrap addresses the historical Ubuntu runner failure recorded
as Actions run `33781367304`, job `100735570690`: the first bundle build stopped
because the runner supplied LLVM/Clang `20.1.2` while the release contract
required `20.1.8`. The canonical fix is the JamScript-owned official LLVM
20.1.8 archive lock, verified before extraction and injected by absolute tool
paths. The historical failure remains part of the release record and is not
rewritten as a successful run.

1. Build the Service with `jamscript build`.
2. Verify the deployment bundle with `jamscript inspect <bundle>`.
3. Provision or upgrade the Service through the network operator's deployment control plane.
4. Compile and run the generated Builder application as a per-Service Formal RPC sidecar.
5. Configure the browser client with separate node, work, and managed-state Provider endpoints.
6. When a downstream network is available, run the manual MiniJAM network E2E
   as a compatibility check before publishing the release artifacts.

Service provisioning is intentionally not exposed as a fake application RPC. MiniJAM currently
has no formal deployment RPC equivalent to the Work RPC, so v0 deployment remains an explicit
operator action. Wallet calls remain in the TypeScript/browser client so the wallet receives one
standard `signRaw` request and private keys never enter the CLI.

## Explicit exclusions

The preview does not claim mainnet readiness. User gas payment, sponsorship, DoS economics,
distributed Provider replication, garbage collection, generic PVM-only witness discovery, and
cross-Service managed state remain outside the v0 scope.
