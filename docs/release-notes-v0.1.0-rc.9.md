# JamScript v0.1.0-rc.9 (unpublished candidate)

This candidate hardens Ownership action submission, PVM execution, and the managed developer toolchain.

## Ownership and client

- Add a phased Ownership action API so applications can prepare network/state data before opening the wallet signature prompt, then submit the signed action separately.
- Preserve `submitOwnershipAction` as the one-call convenience API.
- Accept Polkadot `signRaw` byte-wrapper responses consistently.

## Runtime and backend

- Raise the bounded PVM guest arena to support valid delegated Ownership transfers that exceed the previous limit.
- Improve backend diagnostics for PVM failures without logging action payloads, signatures, or wallet credentials.
- Retry Work submission when a stale context is detected.

## Toolchain

- Build canonical ScriptC service artifacts with release optimization; unoptimized development output could exceed the PVM gas budget during planning.

## Release requirements

- Publish the matching `backend-v0.1.0-rc.9` release first; the JamScript release workflow checks for its manifest and Linux/macOS binaries.
- Run successful source CI and the canonical MiniJAM network E2E for the exact release commit before dispatching `Release JamScript`.
- Publish only after both native CLI and managed toolchain bundles pass release validation.
