# Ownership SDK and Adapter Boundary

## Status

Accepted

## Context

Stable Ownership identities may authorize actions through different active
controllers. Provider-specific identity systems need deterministic proof
verification, but placing those rules in JamScript Core couples every runtime,
compiler, and release to one identity provider.

## Decision

The dependency direction is:

```text
Application
    ↓
Ownership Adapter
    ↓
Ownership SDK
    ↓
JamScript Core
```

- **JamScript Core** owns service execution, managed state, signed actions,
  Ownership primitives, and provider-neutral cryptography such as Ed25519.
- **Ownership SDK** defines sessions, stable subjects, active controllers,
  provider-neutral authorization envelopes, adapter registration, and helpers
  for service-local grants.
- **Ownership adapters** define provider-specific proof codecs and semantics.
  The Matrix adapter verifies the existing M→S→D proof using generic Ed25519.
- **Applications** select adapters and define business actions. They do not
  implement provider proof canonicalization or verification.

Grants remain service-local. A grant in one service does not authorize a
controller in another service. MiniJAM, Jambda, and the JamScript Backend do
not understand provider adapters.

## Compatibility

The Matrix proof byte format, Ownership encoding, and SignedAction encoding
remain unchanged. Matrix verification still executes deterministically in the
Service/PVM; only its code boundary moves out of Core and application business
logic.
