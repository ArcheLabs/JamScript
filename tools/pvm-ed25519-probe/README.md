# Ed25519 PolkaVM probe

This probe exercises an RFC 8032 Ed25519 verification vector in a no-std
guest. Run it, together with the other compiler conformance probes, with:

```text
cargo run --manifest-path tools/pvm-conformance/Cargo.toml --offline
```
