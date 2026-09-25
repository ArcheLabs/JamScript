# ADR: Ownership controller authorization scope

## Status

Superseded by the local-identity architecture.

## Context

An earlier design treated `ControlClaim(subject, controller)` as a
network-scoped delegation owned by a dedicated Ownership Control system
service. That design coupled consumer services to an external state root and
required a network descriptor, external witnesses, and platform ingress APIs.

That boundary is not part of the JamScript architecture.

## Decision

JamScript authenticates the cryptographic controller in `SignedActionV2` and
exposes provider-neutral deterministic cryptographic primitives. Matrix M→S→D
proof verification belongs to the Ownership Matrix adapter, built on generic
Ed25519 verification. JamScript does not maintain a network-wide controller
registry and does not require an Ownership Control system service.

Application services own their authorization policy and state. Locus keeps
controller grants, Matrix bootstrap tombstones, assets, balances, and
allowances in one managed state root. Locus derives the stable `subject` from
its SDK session and authorizes the authenticated `ctx.controller` locally.

The `actAs` field remains in `SignedActionV2` for wire compatibility, but the
Locus service does not use it and the JamScript client no longer exposes
platform ControlClaim ingress APIs.

## Consequences

- MiniJAM and Jambda remain unaware of Ownership controller policy.
- No network descriptor contains an Ownership Control service identity.
- No consumer service performs an external ControlClaim lookup.
- Matrix proof bytes and their Rust/TypeScript codec remain compatible.
- A future service-specific authorization protocol must be expressed in that
  service's managed state, not as an implicit network service.

## Rejected alternative

A dedicated system service with HostCall/external-state coupling is rejected
for this architecture because it splits identity authorization from the
asset state transition and introduces an avoidable external-root consistency
boundary.
