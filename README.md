# JamScript

JamScript's MiniJAM Stage-1 production boundary is the neutral node RPC,
Formal Work RPC, and proof-aware StateProvider. It has no Playground runtime
or compiler dependency.

JamScript is a deterministic TypeScript-like application runtime for JAM
services. The release ABI uses a single typed descriptor for actions, managed
state, queries, and clients.

The supported path uses imports from the `jam` standard library, bounded
primitive input schemas, ABI generation, and generated `no_std` Rust for the
canonical JAM target. JamScript manages its compiler toolchain automatically: the
first canonical build installs the exact platform bundle and verifies its
checksum. `build` emits `service.blob`,
`service.polkavm`, `service.pvm`, and a portable Builder host artifact. The
PVM guest and Builder artifact embed the same compiler-generated
`ServiceApplication`; native imports use the same declared C sources compiled
once for PolkaVM and once for the host.

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
cargo build --locked --bin jamscript
cargo run --locked --bin jamscript -- new hello-jam
cargo run --locked --bin jamscript -- check examples/counter
cargo run --locked --bin jamscript -- build examples/counter
cargo run --locked --bin jamscript -- toolchain status
cargo run --locked --bin jamscript -- doctor
```

To run the optional cross-process MiniJAM compatibility path (it requires a
separate MiniJAM checkout):

    ./scripts/minijam-network-e2e.sh

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
