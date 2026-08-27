# JamScript

JamScript is a deterministic TypeScript-like application runtime for JAM
services. This repository starts with the v0.1 vertical skeleton described in
the JamScript v0.1 implementation plan.

The first slice deliberately supports one exported action, imports from the
`jam` standard library, a bounded primitive input schema, one `execute` action
body, ABI generation, and generated `no_std` Rust for the MiniJAM target. With the
pinned Rust target and Clang 20 installed, `build` emits `service.blob`,
`service.polkavm`, `service.pvm`, and a portable Builder host artifact. The
PVM guest and Builder artifact embed the same compiler-generated
`ServiceApplication`; native imports use the same declared C sources compiled
once for PolkaVM and once for the host.

The protocol layer also includes `SignedActionV1`: canonical bounded
encoding, payload commitments, domain-separated sr25519 verification, sender
derivation, expiry, and nonce-context validation.

The runtime layer provides a language-independent managed-state foundation:
SDK LayoutV1 trie roots and proofs, canonical diffs/transitions, managed wallet
nonce keys, proof-backed guest interfaces, and a reference host provider.
`jamscript-runtime` remains as a compatibility wrapper during migration.

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

The v0 testnet release boundary and operator workflow are documented in
[`docs/releasing.md`](docs/releasing.md).
