# JamScript v0.1.0-rc.12 (candidate)

This release candidate upgrades the guest allocator and surfaces known guest
memory failures through the CLI, Backend RPC, and TypeScript client.

## Guest memory

- Replace the fixed 512 KiB bump arena with a deterministic buddy allocator
  shared by Rust and ScriptC's malloc family. Frees are reusable and buddy
  blocks coalesce; `realloc` preserves content and leaves the old block intact
  when growth fails.
- Use a default 1 MiB initial heap and 16 MiB maximum, with 4 KiB validation
  and page growth. Projects can set `[guest.memory]` in `jamscript.toml`.
- Embed budget values in service descriptor V2 and `build.json`; validate the
  final artifact's effective heap separately from the existing 2 MiB stack.
- Add fixed-width JSGF v1 fatal fault diagnostics. New Backend and CLI
  distinguish known memory faults, explicit out-of-gas, and unknown traps.
- Preserve allocator details in JSON-RPC and typed client errors. A preflight
  resource failure stays `NOT_SUBMITTED`; unknown Formal submission outcome
  keeps its existing pending/`SUBMISSION_UNKNOWN` behavior.

## Verification evidence

- Unit tests cover C ABI alignment and zero-size semantics, free/reuse,
  coalescing, calloc zeroing/overflow, and realloc success/failure content
  preservation.
- Actual target PVM decoded an action with 530,000 bytes under the new default;
  2,945,024 bytes were committed and the requested-byte high-water mark was
  530,026 bytes.
- The pinned MiniJAM Formal V1 executor applied the same large ScriptC action
  with a receipt at 100 M refine gas. It used 50,669,370 gas. At 5 M and 20 M,
  the same workload exhausted gas; increasing heap does not remove that gas
  requirement.
- Backend and CLI local preflight gas metering use the pinned MiniJAM Stage-1
  maximum of 1,000 M per WorkItem. This is separate from `minItemGas`, which
  is the Service fee floor; local PolkaVM and Formal gas values remain
  different measures.
- A 512 KiB artifact returned `GUEST_HEAP_LIMIT_EXCEEDED` in both local PVM and
  Formal MiniJAM. The Formal work item failed before Accumulate, and the E2E
  harness verified unchanged state root and nonce.
- Rust tests and `-D warnings` Clippy pass for the guest, codegen, target,
  Backend, and CLI packages; Client build and flow tests pass.

## Upgrade

Install JamScript `v0.1.0-rc.12` with its matching Backend
`backend-v0.1.0-rc.12`. Install `@jamscript/client@0.1.0-rc.7` when using the
typed structured-error API. Rebuild the application with the new toolchain,
review `guestMemory` in `build.json`, and deploy the new Service blob through
the Service's own upgrade mechanism.

Updating the CLI, Backend, toolchain, or npm client does not replace the guest
allocator in an existing Service blob. No production Service or business state
was changed in this release work.

Before describing this candidate as published, complete the repository's
release gates for the exact `main` commit: successful push CI, successful
canonical MiniJAM network E2E, Backend release assets, JamScript toolchain
assets, installer verification, and npm tarball installation. Add the final
release URLs and checksums after those checks finish.
