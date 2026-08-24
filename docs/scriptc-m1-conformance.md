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
profile in `build.json`. `Date`, clocks, randomness, process/environment,
filesystem/network access, timers, promises, dynamic loading, `eval`, and the
`Function` constructor are rejected before compilation. There is no fallback
to the Legacy compiler for language `0.2`.

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
| control flow, helpers, nested helpers, bool/comparison | PVM PASS | 368 gas |
| arrays | compile PASS | — |
| objects | compile PASS | — |
| strings | compile PASS | — |
| `Uint8Array` | compile PASS | — |

The PVM fixture returned the expected value `42`; allocation count, requested
bytes, and high-water mark were all `0` for this scalar/control-flow case.
The generated fixture artifacts and machine-readable report are written under
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
