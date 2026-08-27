# Native C PolkaVM probe

This probe verifies that a freestanding C module is compiled as an
`rv64emac/lp64e` PIC archive and is resolved by the official Rust/rust-lld
guest link. It must not invoke Clang as a final ELF linker. The conformance
runner executes the Rust entry, the C call, and the Rust continuation under a
fixed 5M gas limit.
