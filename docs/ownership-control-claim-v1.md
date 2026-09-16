# ControlClaim v1

`ControlClaim(subject, controller)` is a network-scoped delegation state. It means that the controller may act as the subject when an action explicitly carries `act_as=subject`.

The controller is itself an Ownership. Claims are explicit, revocable, and non-transitive: `A → B` and `B → C` never imply `A → C`. `addController` and `revokeController` require the effective owner to equal the subject. Revocation is an on-chain state transition and is not inferred from external key or server state.

`SignedActionV2` uses the controller for cryptographic authorization. Without `act_as`, the effective owner is the controller; with `act_as`, an active ControlClaim is required. Nonces follow the effective owner, so controller rotation does not move the owner nonce lane.
