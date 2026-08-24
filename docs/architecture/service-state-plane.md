# Service State Plane (M0)

`service-state-plane` is the stable facade between a service and managed state.
It exposes a versioned commitment, read request, witness, and provider seam.
The existing `service-runtime-state::{FullState, ProofState}` implementations
remain the storage and proof adapters.  This M0 deliberately does not define a
distributed network, ordering, sequencer, or consensus protocol: a host gives a
service a finalized commitment and a witness, and the service verifies that the
witness roots at that commitment before reading it.

The adapter test proves a `FullState` proof can be consumed by `ProofState` via
the facade and that a forged root is rejected.
