# JamScript

JamScript is a deterministic TypeScript-like application runtime for JAM
services. The release ABI uses a single typed descriptor for actions, managed
state, queries, and clients.

The supported path uses imports from the `jam` standard library, bounded
primitive input schemas, ABI generation, and generated `no_std` Rust for the
MiniJAM target. With the pinned Rust target and Clang 20 installed, `build` emits `service.blob`,
`service.polkavm`, `service.pvm`, and a portable Builder host artifact. The
PVM guest and Builder artifact embed the same compiler-generated
`ServiceApplication`; native imports use the same declared C sources compiled
once for PolkaVM and once for the host.

The v0 release boundary uses `SignedActionV1`: canonical bounded encoding,
payload commitments, ServiceKey identity, domain-separated sr25519
verification, sender derivation, expiry, and nonce-context validation.
Formal V1 is the first supported wire/runtime protocol; development generations
before it are not compatibility contracts.

The release ABI uses one typed descriptor and Jambda's `jam-codec 0.1.1`
encoding rules; JamScript does not maintain an independent binary codec. See
[`docs/release-conformance.md`](docs/release-conformance.md) for the type
system and canonical vectors.

The runtime layer provides a language-independent managed-state foundation:
SDK LayoutV1 trie roots and proofs, canonical diffs/transitions, managed wallet
nonce keys, proof-backed guest interfaces, and a reference host provider.
`jamscript-runtime` exposes the formal runtime wrapper.

```bash
cargo build --locked --bin jamscript
cargo run --locked --bin jamscript -- new hello-jam
cargo run --locked --bin jamscript -- check examples/counter
cargo run --locked --bin jamscript -- build examples/counter
```

To run the real cross-process MiniJAM path:

    ./scripts/minijam-network-e2e.sh

The target adapter uses the MiniJAM SDK beside this repository by default. Set
`JAMSCRIPT_MINIJAM_SDK` or `target.minijam.sdk_root` for another checkout.

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
