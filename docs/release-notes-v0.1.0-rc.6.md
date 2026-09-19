# JamScript v0.1.0-rc.6

This release closes the released Ownership consumer path. Ownership-authenticated
JamScript actions can now be checked, inspected with `jams abi`, and built using
only the released `jams` executable and managed ScriptC toolchain.

Added:

- `ownership()` action authentication and Ownership ABI values.
- `ctx.owner` for the effective owner and `ctx.controller` for the actual signer.
- deterministic `ownershipKey()` for canonical 32-byte state indexes.
- multiple ownership-only actions in one ScriptC service.
- release-consumer validation using the published CLI and managed bundle.

Wallet `SignedActionV1` services and public actions remain supported. The
backend release remains independent at `backend-v0.1.0-rc.5`.
