# Guest memory upgrade

This document describes the JamScript guest memory changes prepared for
`v0.1.0-rc.12`, `backend-v0.1.0-rc.12`, and `@jamscript/client@0.1.0-rc.7`.
Release links and checksums belong here only after the release workflows have
published and the installer has verified their assets.

## What changed

The old guest used a 512 KiB bump arena. Rust frees did not reclaim space, and
ScriptC `free` did not return blocks to an allocator. The new guest uses the
no-std `buddy_system_allocator` 0.13.0 allocator for Rust `GlobalAlloc` and
ScriptC `malloc`, `calloc`, `realloc`, and `free`. Buddy blocks are aligned,
returned on free, and coalesced with their free buddies. The allocator grows
through the PVM `sbrk` instruction in deterministic 4 KiB pages and reuses
committed pages for the rest of the invocation.

`realloc` uses the same allocator, preserves the old bytes, and leaves the old
allocation valid if the replacement cannot be allocated. `calloc` checks the
count-times-size multiplication and zeroes the returned bytes. C zero-size
allocation reserves a distinct one-byte payload; Rust zero-sized layouts use
the standard aligned dangling pointer. As with ordinary C allocators, invalid
and stale pointers remain undefined behavior; the header catches some readable
invalid frees but cannot validate arbitrary pointers.

The fixed C allocation header is 16 bytes and 16-byte aligned. It is inside the
managed region, so the heap budget includes allocator metadata, alignment loss,
buddy rounding, and page-rounded `sbrk` growth. The fault record is outside the
managed region.

## Project configuration

An unconfigured project uses a 1 MiB initial heap and a 16 MiB maximum. The
initial region is committed at invocation start; later pages are committed
only when allocation needs them. Projects can override both values:

```toml
[guest.memory]
heap_initial_bytes = 1048576
heap_max_bytes = 16777216
```

Both values are byte counts and must be positive multiples of 4096. The initial
size cannot exceed the maximum. The platform policy cap is 64 MiB, and the
builder also checks the final PVM memory map's heap and stack boundary. If the
artifact cannot provide the declared maximum, the build fails with the
configured and effective limits. The builder never silently increases the
declared maximum. Heap and stack remain separate; this change retains the
existing 2 MiB minimum stack.

The selected values are recorded in `build.json` under `guestMemory`, are
embedded in service descriptor V2, and affect the compiled Service code hash.
The CLI `inspect` output includes the build manifest. Do not copy a memory
budget from a new manifest onto an older Service code hash.

## Why the defaults are 1 MiB / 16 MiB

The complete ScriptC service was built for the actual RV64/LP64E PVM target.
The local PolkaVM interpreter decoded a 530,000-byte action, exceeding the old
512 KiB cap, with a 16 MiB maximum. It committed 2,945,024 bytes; the measured
high-water mark of requested allocator bytes was 530,026, and the live
requested count returned to zero after the invocation. Committed bytes include
the allocator's buddy-block and address-alignment overhead and are not the
same measure as requested live bytes.

The same artifact then ran in the pinned MiniJAM Formal V1 executor. The
530,000-byte ScriptC action returned an `Applied` receipt and the accumulated
state root matched the expected result. That refine used 50,652,196 gas. The
ordinary seed and advance actions used about 1.83 M and 1.92 M gas. The large
action exhausted gas at both 5 M and 20 M limits even though the heap could
serve it; at 100 M it passed. The pinned MiniJAM Stage-1 protocol allows at
most 1,000 M refine gas per WorkItem. The `minItemGas` value set at Service
creation is a fee floor, not that execution ceiling. The Backend and `jams
run` local preflight ceilings now match the 1,000 M platform maximum, while
their PolkaVM meter is still not numerically equivalent to the Formal tariff.
Operators should measure their own signed actions and proofs against the
Formal runner.

At a configured 512 KiB maximum, the local PVM classified the same allocation
as `GUEST_HEAP_LIMIT_EXCEEDED`. MiniJAM emitted the versioned fault record
(`code=1`, refine stage, 530,227-byte request, 512 KiB committed and maximum),
and returned a failed work item. No Accumulate ran; both the service state root
and nonce stayed unchanged. The 16 MiB default provides growth headroom over
the measured 2.81 MiB committed region without approaching the 64 MiB policy
cap. Production workload distributions beyond this tested action remain a
deployment-specific sizing input.

## Formal and local PVM behavior

The pinned MiniJAM/Jambda runtime supports the standard PVM `sbrk` instruction.
It accepts a 64-bit increment, checks the 32-bit address range, rejects growth
past its stack-derived heap limit, adds writable permission at 4 KiB page
boundaries, returns the old break on successful growth, and returns zero on
failure. PolkaVM 0.30.0's local interpreter has the same zero failure sentinel
but returns the new break on success. The allocator checks only the sentinel,
so it does not depend on which successful pointer is returned. PVM addresses
are 32-bit even though guest pointers and C `size_t` use the 64-bit LP64E ABI;
all conversions and end-address calculations are checked.

The invocation bootstrap commits at most one page before generated code installs
the project's memory budget. Generated plan/refine entry points then reset
allocator metadata only at invocation boundaries. They never reset the heap
between actions. Formal execution may retain mapped pages across calls in the
same work item; the runtime reconstructs its free list from the current PVM
break and rejects a budget smaller than already committed memory.

Backend preflight runs with PolkaVM 0.30.0 synchronous gas metering, so it
distinguishes a metered local out-of-gas from an unclassified trap. Its gas
number is not numerically equivalent to MiniJAM's Formal tariff. MiniJAM's
Formal executor is the authority for transaction gas use.

## Fault protocol and RPC

The guest keeps a fixed 44-byte `JSGF` version 1 record outside the managed
heap. It is reset at the start of each runtime invocation and stores only the
first known fatal fault: code, stage, request size, alignment, committed/max
heap, and requested-byte allocator counters. Fault reporting uses a
fixed-size stack buffer and JIP-1 host log call 100; it does not allocate a
`String`, `Vec`, or JSON value after exhaustion. The record is also exposed as
`jamscript_guest_fault_record_v1`; readers check the export, guest memory
bounds, record length, magic, version, stage, and known code before using it.

Known guest codes are `GUEST_HEAP_LIMIT_EXCEEDED`,
`GUEST_MEMORY_GROW_FAILED`, `GUEST_MEMORY_ALLOCATION_FAILED`, `GUEST_PANIC`,
`GUEST_ABORT`, and `GUEST_MEMORY_CONFIG_INVALID`. The execution layer reports
`PVM_OUT_OF_GAS` only when the VM reports gas exhaustion. Any other trap stays
`PVM_TRAP` and keeps its program counter. An old guest that lacks the V2
diagnostic export also stays a generic trap; the Backend does not infer OOM
from a trap location.

A local preflight failure is returned as a definite not-submitted error. RPC
keeps the legacy numeric `code` and readable `message`, with structured details
under `data`:

```json
{
  "code": -32045,
  "message": "NOT_SUBMITTED: GUEST_HEAP_LIMIT_EXCEEDED",
  "data": {
    "code": "NOT_SUBMITTED",
    "cause": {
      "code": "GUEST_HEAP_LIMIT_EXCEEDED",
      "message": "Guest allocation exceeded the configured heap budget",
      "stage": "plan",
      "serviceId": 123,
      "codeHash": "0x…",
      "details": {
        "requestedBytes": 530000,
        "alignment": 16,
        "heapCommittedBytes": 524288,
        "heapMaxBytes": 524288,
        "allocatorLiveRequestedBytes": 24,
        "allocatorHighWaterRequestedBytes": 51,
        "allocatorCumulativeRequestedBytes": 79
      }
    }
  }
}
```

The TypeScript client's `RpcError.structuredError` unwraps the nested cause
while preserving `submissionState: "not_submitted"`. Transaction tracking
retains `errorInfo`, code, stage, and details. A later Formal submission whose
outcome is unknown remains `SUBMISSION_UNKNOWN`; this diagnostic never triggers
an automatic retry. Logs include Service/code-hash routing data, not action
payloads, signatures, or wallet secrets.

## Compatibility and upgrade

| Component | Compatibility |
|---|---|
| New Backend with V2 guest | Reads the structured fault record; validates it before classification. |
| New Backend with an older guest lacking the fault export | Continues to report generic `PVM_TRAP` with PC where available. |
| New CLI with V2 guest | Displays known fault code/stage/budget; gas exhaustion and unknown traps remain distinct. |
| New client with older Backend | Existing RPC fields continue to work; no structured fault data is available. |
| Older client with new Backend | Existing numeric RPC code/message remain; structured data is an additional field. |
| V2 guest with older Backend | Normal execution may still work, but that Backend will not classify the new fault record; upgrade the Backend first when structured fault reporting is required. |

Install the matching JamScript CLI/toolchain and Backend release, and use
`@jamscript/client@0.1.0-rc.7` if the application consumes the new structured
error types. Rebuild each application with the new toolchain and verify the
new `serviceDescriptorVersion: 2`, `guestMemory` settings, and `code_hash` in
its build manifest. Deploy the new blob using that Service's supported upgrade
mechanism.

An existing Service contains its old allocator in its guest blob. Updating the
CLI, toolchain, Backend, or npm package does not replace it. This change does
not migrate Service state or make an immutable Service upgradeable. Preserve
assets, balances, Ownership nonce, and application state through the
application's separate migration/upgrade plan; no production Service was
changed as part of this work.

The guest heap is single-threaded and deterministic. Buddy allocator metadata
is in-band, and freed blocks are reusable/coalesced; memory is not returned to
the host during an invocation. The runtime cannot make arbitrary invalid C
pointers safe, does not provide tracing GC, and does not claim that local
PolkaVM metering numerically predicts Formal gas.
