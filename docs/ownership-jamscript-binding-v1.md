# JamScript Ownership Binding v1

`wallet()` and `SignedActionV1` remain compatible. `ownership()` selects `SignedActionV2`, whose envelope carries canonical `controller` Ownership, optional canonical `act_as` Ownership, nonce, validity, payload hash, authorization proof, and payload.

The action commitment is domain-separated with `JAMSCRIPT_ACTION_V2`. For generic Ed25519 authorization the signed message is `JAMSCRIPT_ACTION_V2:` followed by base64url-without-padding of the commitment. Application code receives the effective owner as `ctx.owner` and the actual signer as `ctx.controller`; application code never performs cryptographic verification.
