# Guest memory upgrade design record

Date: 2026-10-03. This record is written before implementation and is based on
the checked-in release source and the MiniJAM source revision pinned by
`toolchains/minijam.lock` (`1000bd7504a61010b5e83b1cae5a651b9373ae08`, with
Jambda gitlink `440ea20528bf16c716c249bcdf8c9f781ad0b087`).

## Execution and memory interface audit

- JamScript production refine runs through MiniJAM's Jambda `InnerInterpMemory`;
  its `sbrk` accepts a `u64` increment, rejects values that do not fit `u32`,
  checks addition and the stack-derived `break_limit`, makes page-rounded RW
  permissions available, returns the old break for a successful non-zero
  increment, returns the current break for zero, and returns zero on failure.
  `VM_PAGE_SHIFT` is 12 (4 KiB pages). The pinned guest target is RV64/LP64E,
  so Rust pointers and C `size_t` are 64-bit; PVM virtual addresses and the
  runner's `sbrk` address/increment fields are limited to unsigned 32-bit
  values. The allocator therefore uses `usize` layouts but checks each break,
  increment, end address, and C header conversion before narrowing.
- The local Backend uses PolkaVM 0.30.0's interpreter. Its `sbrk` takes a
  `u32` increment, checks the blob memory map's maximum heap size, grows RW
  memory to page boundaries, and returns the new break on success or zero on
  failure. The implementation will use `sbrk(0)` for the base and treat the
  increment result only as a success/failure sentinel, so it does not rely on
  the two runners' different successful return values.
- Stack, static RW data, and the dynamic heap occupy separate PVM regions; the
  canonical runner bounds heap growth below the stack. JamScript declares its
  2 MiB minimum stack independently. Heap pages persist only for one VM
  invocation and are reset by the execution environment. We found no separate
  per-page fee in the pinned Jambda memory implementation; `sbrk` and guest
  memory instructions are still subject to the PVM instruction gas rules.
- The local Backend uses PolkaVM 0.30.0 synchronous gas metering with its
  1,000,000,000-unit preflight ceiling, matching the pinned MiniJAM Stage-1
  `MAX_REFINE_GAS` per WorkItem. The 5,000,000 `minItemGas` value is a service
  fee floor, not the WorkItem execution limit. Local PolkaVM metering is not
  claimed to be numerically identical to MiniJAM's Formal tariff; canonical
  MiniJAM E2E remains the gas compatibility check.
- Jambda recognizes JIP-1 log host-call 100. Guest-side fatal diagnostics can
  therefore emit a fixed, versioned record without allocating from the guest
  heap. The Backend's PVM instance can additionally expose the same static
  record through a bounded diagnostic entry and validate it after a local
  preflight trap.

## Implementation decisions

- Use `buddy_system_allocator`'s deterministic no-std buddy heap. Its in-heap
  intrusive free lists avoid recursive metadata allocation, `dealloc` returns
  and coalesces buddies, and rescue growth can add PVM `sbrk` regions without
  invalidating earlier pointers. Rust `GlobalAlloc` and ScriptC malloc family
  will share this allocator; C headers will retain the original size/alignment
  needed by `realloc`.
- Default budget: 1 MiB initial, 16 MiB maximum, both rounded/validated in
  4 KiB pages. These are runtime policy limits, not the PVM address-space
  maximum. The default gives twice the old 512 KiB cap initially and up to
  32 times the old cap on demand. A complete ScriptC guest was measured below;
  its successful large-action run used 2.81 MiB of committed heap, leaving
  headroom under the 16 MiB default.
- Add `[guest.memory] heap_initial_bytes` and `heap_max_bytes`; encode both in
  a version-2 service descriptor and `build.json`, so the service artifact
  identity changes with its budget. Keep the stack declaration unchanged.
- Keep a fixed-size fault record outside the managed heap. It is cleared at
  each invocation start and records only known allocator/config/panic failures,
  with stage, request, alignment, committed and maximum bytes, and measured
  allocation counters. Emit it using the established JIP-1 log path; unknown
  traps and out-of-gas remain unclassified unless the execution environment
  identifies them. Older artifacts without the record remain generic traps.
- Preserve old readable RPC error fields and add a versioned structured error
  object. Planner failures remain pre-submission failures. Formal/unknown
  submission results retain the existing `submission_unknown` lifecycle and
  are never retried based on a guest diagnostic.

## Release boundary

This requires a new JamScript CLI/toolchain and Backend release. The typed
structured-error surface also changes, so the Client is versioned with it. It
does not update MiniJAM, alter Locus, retry transactions, touch a production
Service, or migrate business state. Existing guest blobs retain their embedded
allocator and must be rebuilt and redeployed separately.

## Final verification results

- Unit coverage for the shared allocator ABI verifies alignment, C zero-size
  semantics, free/reuse, buddy coalescing, calloc zeroing/overflow, realloc
  content preservation, and failure preserving the old block.
- The complete PVM guest decoded a 530,000-byte action with a 16 MiB maximum.
  The local PVM reported 2,945,024 committed bytes, 530,026 requested-byte
  high-water, and zero live requested bytes after completion.
- The same ScriptC service succeeded in the pinned MiniJAM Formal V1 runner
  with an `Applied` receipt at 100 M refine gas; measured refine gas was
  50,669,370. A 5 M or 20 M gas limit exhausted gas independently of heap
  availability.
- At 512 KiB, both local PVM and Formal MiniJAM classified the 530 KiB request
  as `GUEST_HEAP_LIMIT_EXCEEDED`. The Formal work item failed before
  Accumulate, and the harness confirmed unchanged state root and nonce.
- `sbrk` behavior and return values were verified against the pinned MiniJAM
  source and PolkaVM 0.30.0. The allocator checks only the zero failure
  sentinel because the two runners return different successful break values.
- Release, installer, registry, and clean-environment checks remain required
  before calling the candidate published.
