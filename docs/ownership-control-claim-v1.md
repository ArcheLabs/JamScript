# Ownership controller authorization compatibility note

This document records the compatibility boundary for the former
`ControlClaim(subject, controller)` design.

The `SignedActionV2` envelope still carries `controller` and the optional
`actAs` field so existing wire encodings remain readable. A non-null `actAs`
value is rejected as unsupported legacy delegation by the protocol/runtime;
it is never resolved through reserved ControlClaim state. These fields do
not create a network-scoped controller registry. In particular, a JamScript
consumer must not infer authorization from an external Ownership Control
service.

For Locus:

- `subject` is the stable asset identity;
- `ctx.controller` is the cryptographically authenticated signer;
- controller grants and Matrix bootstrap tombstones are Locus-local state;
- the Locus SDK injects `subject` into business action payloads; and
- Locus does not submit `actAs`.

The Matrix `MatrixControlClaimProofV1` name is retained as a wire-compatible
codec identifier. JamScript Core exposes provider-neutral deterministic
cryptographic primitives. Ownership adapters are SDK/standard-library
components built on those primitives; Matrix M→S→D verification belongs to the
Matrix Ownership adapter, not the runtime. The grant created after verification
remains application state.

Generic JamScript services may define their own authorization state and
protocol. No service receives implicit access to a network-wide ControlClaim
state root.
