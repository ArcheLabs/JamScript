# ScriptC M1 conformance

M1 adds an opt-in `ScriptC` backend for language `0.2`. Legacy language
`0.1` remains unchanged and cannot select a compiler backend. The production
path is:

```text
service.ts
  → bounded ScriptC IR/C library
  → stable C adapter
  → selected freestanding ScriptC runtime
  → official PolkaVM Rust cdylib/rust-lld link
  → canonical service.elf
  → JamV1 PVM
```

The backend records the pinned ScriptC revision, Node version, TypeScript
version, package-lock hash, surface-manifest hash, and deterministic runtime
profile in `build.json`. ScriptC's manifest fences cover the detector-backed
clock, randomness, process-environment, and performance-clock surfaces. The
adapter also performs a conservative TypeScript-AST reachability walk for
Date/clocks, randomness, process/environment, filesystem/network access,
timers, promises/async, dynamic loading, `eval`, and the `Function`
constructor: an unreachable helper containing one of these surfaces is allowed,
while a reachable use fails the build. There is no fallback to the Legacy
compiler for language `0.2`.

## Conformance run

With the locked dependencies and Node `v24.15.0`:

```text
SCRIPTC_NODE=/path/to/node-24.15.0/bin/node \
  cargo run --manifest-path tools/pvm-scriptc-m1-runner/Cargo.toml --offline
```

The run compiles the M1 fixture set, builds the official PolkaVM guest, and
executes the control-flow fixture under the normal 5,000,000 gas budget.

| Fixture | Result | Measurement |
| --- | --- | ---: |
| control flow, helpers, nested helpers, bool/comparison | PVM PASS | 380 gas |
| arrays | PVM PASS | 1452 gas / 96 B high-water |
| objects | PVM PASS | 424 gas / 24 B high-water |
| strings | PVM PASS | 483 gas |
| `Uint8Array` | PVM PASS | 651 gas / 42 B high-water |

Every PVM fixture returned the expected value `42` under the normal 5,000,000
gas budget. The diagnostic allocator reports allocation count, requested
bytes, and high-water mark per stage. The final ELF was inspected: `memcpy`
and `memcmp` are bounded byte loops and no recursive lowering remains; the
`memset` symbol is not emitted for this fixture. The generated fixture
artifacts and machine-readable report are written under
`target/pvm-scriptc-m1`.

## JamScript 0.2 counter

`examples/counter-scriptc` and `examples/public-counter-scriptc` exercise the
language-version gate, wallet/public action metadata, stable selector/ABI
generation, ScriptC C adapter generation, and the official PolkaVM production
build. The wallet counter build was verified locally as a canonical ELF/PVM
bundle; its production heap remains the existing 64 KiB guest configuration.

ScriptC compute is kept separate from the accepted protocol/runtime/state
implementation. Generated Rust still owns action decoding, authentication,
managed-state execution, and output encoding; the ScriptC symbol is only the
deterministic compute call.

The real generated counter was also exercised through the MiniJAM interpreter
with a wallet-signed `SignedActionV2`: envelope decode, sr25519 verification,
nonce read/update, ScriptC compute, and managed-state root finalization all
ran under the production 5,000,000 refine-gas item limit. The first transition
used 1,153,060 gas and produced an applied receipt with nonce `1`; the
counter-only matrix also covered replay rejection and the next nonce. The
service ELF contains bounded `memcpy`/`memcmp` loops and a bounded `memset`
implementation, and generated Accumulate contains only commitment reads,
parent-root/expiry checks, and the reserved commitment write.
