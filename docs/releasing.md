# JamScript v0 Testnet Developer Preview

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

The v0 release freezes `SignedActionV2`, the generated ABI/state descriptors, Polkadot
`LayoutV1<Blake2Hasher>`, managed-state recovery v1, and Builder artifact v1. A release bundle
contains `protocol-v0.json` and `checksums.json`; `jamscript inspect <bundle>` verifies every
listed artifact before displaying its metadata.

The Builder/Provider process is deployed per Service. It statically compiles the generated host
application and the same native C sources used by the PVM Service. Loading arbitrary native
libraries into a shared daemon is not supported.

Provider persistence is enabled with `JAMSCRIPT_PROVIDER_STORE`. The append-only recovery log is
replayed and cryptographically revalidated on startup. Finalized JAM/MiniJAM storage remains the
only source of canonical roots; the Provider supplies data and proofs for explicit roots.

## Operator workflow

1. Build the Service with `jamscript build`.
2. Verify the deployment bundle with `jamscript inspect <bundle>`.
3. Provision or upgrade the Service through the network operator's deployment control plane.
4. Compile and run the generated Builder application as a per-Service Formal RPC sidecar.
5. Configure the browser client with separate node, work, and managed-state Provider endpoints.
6. Run the tagged network E2E before publishing the release artifacts.

Service provisioning is intentionally not exposed as a fake application RPC. MiniJAM currently
has no formal deployment RPC equivalent to the Work RPC, so v0 deployment remains an explicit
operator action. Wallet calls remain in the TypeScript/browser client so the wallet receives one
standard `signRaw` request and private keys never enter the CLI.

## Explicit exclusions

The preview does not claim mainnet readiness. User gas payment, sponsorship, DoS economics,
distributed Provider replication, garbage collection, generic PVM-only witness discovery, and
cross-Service managed state remain outside the v0 scope.
