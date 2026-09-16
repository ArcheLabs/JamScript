# Matrix Ownership Binding v1

Matrix User IDs are discovery inputs only. A `/keys/query` result resolves the cross-signing master public key `M` into ordinary `Ownership(ED25519_KEY, M)`. A device key `D` is also ordinary `ED25519_KEY` Ownership.

The Matrix cross-signing chain `M → S → D` is used once as bootstrap evidence for `ControlClaim(M, D)`. Normal transactions contain only `controller=D`, `act_as=M`, and the device action signature. Matrix JSON, username, and the full cross-signing proof are not normal transaction inputs.

The initial bootstrap is one-time per master Ownership. Matrix server or device changes do not automatically revoke an on-chain claim.
