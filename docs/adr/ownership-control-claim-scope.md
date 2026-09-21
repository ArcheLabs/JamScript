# ADR: Ownership ControlClaim scope

## Status

Accepted for the ownership-platform closure work.

## Context

The normative Ownership documentation describes `ControlClaim(subject,
controller)` as network-scoped. The implementation present before this ADR did
not provide that scope:

- `jamscript-runtime::ControlClaimState` is an in-memory helper used by unit
  tests and has no chain ingress;
- generated service execution derives `control_claim_key(subject, controller)`
  and reads it through the current service's managed-state witness;
- `jamscript-service-backend` plans and witnesses that key against the service
  being submitted to; and
- no reserved/global state namespace or fixed Ownership platform service is
  defined in the current network descriptor.

Consequently, an active claim in the current implementation is visible only
inside the managed state of the service that stores it. Treating this as
network-scoped would be an authorization boundary bug.

## Decision

ControlClaim remains normatively network-scoped. It will not be redefined as
service-scoped to make the existing implementation appear complete.

The platform implementation will use a dedicated Ownership Control system
service, distinct from every consumer service such as Locus. Consumer service
execution will read the system service through the existing proof-backed
external-state mechanism. The system service's service identity must be part of
the network descriptor and deployment provenance; it must not be inferred from
the Locus service ID.

The system service owns the following state transitions:

- one-time Matrix bootstrap with a controller-possession signature;
- add controller;
- revoke controller; and
- read-only active-claim and bootstrap-used queries.

The service state records `bootstrapUsed[subject]` independently from the
active claim. Revocation therefore cannot make an old bootstrap proof usable a
second time. Claims remain non-transitive.

## Consequences

The runtime and client need a platform ingress that targets the configured
Ownership Control service. Locus service source remains unchanged. The
consumer-service planner must include the configured control-service witness
whenever an action carries `act_as`, and refine must verify the witness root as
an external dependency.

The network descriptor must fail closed when the control-service identity is
missing or mismatched. A backend-local map or an unverified HTTP resolver is
not an acceptable substitute for this state.

This decision does not require a MiniJAM consensus change: it uses ordinary
service state and the existing external-state dependency mechanism. If a future
implementation attempts to provide the same guarantee without a deployed
system service, that design must be reviewed against this ADR first.

## Rejected alternatives

### Quietly documenting service-scoped claims

Rejected because it changes the authorization meaning and would allow the same
subject/controller pair to have different status in two services.

### Backend-only or browser-only claims

Rejected because those claims are not finalized chain state and cannot protect
SignedActionV2 execution.

### MiniJAM consensus storage

Not selected for this release. The existing external-state path is sufficient
for a dedicated system service, so changing MiniJAM consensus would broaden the
platform boundary unnecessarily.
