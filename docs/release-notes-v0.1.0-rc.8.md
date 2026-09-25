# JamScript v0.1.0-rc.8 (unpublished candidate)

This candidate separates provider-neutral execution and cryptography from
identity-specific authorization adapters.

## JamScript Core

- Exposes deterministic generic Ed25519 verification to ScriptC services and
  the PVM.
- Keeps Ownership encoding, signed actions, and transaction wire formats
  unchanged.
- Does not include provider-specific identity verification in the runtime.

## Ownership SDK

- Adds Ownership sessions, subject/controller separation, service-local grants,
  and a provider-neutral adapter registry to `@jamscript/client`.
- Adds a Matrix Ownership adapter for the existing M→S→D proof format. The
  adapter is SDK functionality built on generic cryptographic primitives; it
  is not a JamScript Core or runtime feature.

## Release validation

- The release workflow builds the candidate CLI and canonical toolchain, then
  builds and executes a generic Ed25519 consumer using those candidate
  artifacts. It checks both valid and invalid signatures and rejects runtime
  fatal results.
- This file describes an unpublished candidate. Publishing remains a separate
  release action.
