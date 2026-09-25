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
codec identifier. Its contents are cryptographic M→S→D evidence only; the
grant created after verification is application state.

Generic JamScript services may define their own authorization state and
protocol. No service receives implicit access to a network-wide ControlClaim
state root.
