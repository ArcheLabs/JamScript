# JamScript Ownership Binding v1

`wallet()` and `SignedActionV1` remain compatible. `ownership()` selects `SignedActionV2`, whose envelope carries canonical `controller` Ownership, optional canonical `act_as` Ownership, nonce, validity, payload hash, authorization proof, and payload.

The action commitment is domain-separated with `JAMSCRIPT_ACTION_V2`. For generic Ed25519 authorization the signed message is `JAMSCRIPT_ACTION_V2:` followed by base64url-without-padding of the commitment. Application code receives the effective owner as `ctx.owner` and the actual signer as `ctx.controller`; application code never performs cryptographic verification.

The released language surface is:

```typescript
import { action, ownership, ownershipKey } from "jam";

export const example = action({
  auth: ownership(),
  input: { to: ownership },
  execute(ctx, input) {
    const ownerIndex = ownershipKey(ctx.owner);
    const recipientIndex = ownershipKey(input.to);
  },
});
```

`ownership()` is both the Ownership action-auth constructor and the ABI value
type. `ownershipKey()` is deterministic and uses the same canonical encoding
and `OWNERSHIP_ABSTRACTION_KEY_V1` BLAKE2b-256 domain as `ownership-core`.
Ownership ABI values are encoded directly as `version || kind ||
public_length_le_u16 || public`; there is no additional compact length prefix.
The released managed backend supports ownership-only services with multiple
actions. Existing `wallet()`/`SignedActionV1` and public actions remain
compatible.
