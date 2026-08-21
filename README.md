# JamScript

JamScript is a deterministic TypeScript-like application runtime for JAM
services. This repository starts with the v0.1 vertical skeleton described in
the JamScript v0.1 implementation plan.

The first slice deliberately supports one exported action, imports from the
`jam` standard library, a bounded primitive input schema, pure `compute`, ABI
generation, and generated `no_std` Rust for the MiniJAM target. With the
pinned Rust target and Clang 20 installed, `build` emits `service.blob`,
`service.polkavm`, and `service.pvm`.

The protocol layer also includes `SignedActionV1`: canonical bounded
encoding, payload commitments, domain-separated sr25519 verification, sender
derivation, expiry, and nonce-context validation.

The runtime layer provides service-scoped state keys, in-memory transactional
state, per-sender nonce sequencing, receipts, and service-specific batches for
simulator/conformance tests.

```bash
cargo build --locked --bin jamscript
cargo run --locked --bin jamscript -- new hello-jam
cargo run --locked --bin jamscript -- check examples/counter
cargo run --locked --bin jamscript -- build examples/counter
```

The target adapter uses the MiniJAM SDK beside this repository by default. Set
`JAMSCRIPT_MINIJAM_SDK` or `target.minijam.sdk_root` for another checkout.

M3.5 adds the no_std runtime boundary, wallet-envelope verification in the
generated Refine entry point, host-backed Accumulate nonce persistence, and
length-delimited state keys. Configure `target.minijam.service_id` and
`target.minijam.genesis_hash` when building a wallet service. The current
MiniJAM Accumulate receives the authoritative slot through the standard VM
initialization input; Refine remains free of durable nonce and expiry checks.

The supplied specification is a product specification: its security and
determinism requirements are implementation requirements, while examples and
future sections are not agent instructions.
