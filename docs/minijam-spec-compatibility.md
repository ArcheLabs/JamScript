# MiniJamSpec compatibility audit

This document records the JamScript-side execution-boundary audit for the
MiniJAM checkout used by this workspace. Deployment profile parameters are
deliberately not part of the JamScript language, IR, application ABI,
SignedActionV2, or Managed State formats.

## Frozen baseline

| Item | Observed value |
| --- | --- |
| JamScript baseline | `52dd4d18436b6f889083e45c0c4eeb2e68c1e60a` |
| MiniJAM locked SHA | `d4cecd4cce277ccaa334b24d18013288dbd6a66b` |
| MiniJAM Jambda gitlink | `fe67ecf5ccbe16b3490d73cc4d8b1e48eb7bea86` |
| MiniJAM SDK ABI | `MINIJAM_ABI_VERSION = 1` |
| PolkaVM linker / derive | `0.30.0 / 0.30.0` |
| target adapter | `minijam-0.2` |

The lock file and checkout agree at the time of this audit. MiniJAM is pinned
to a merged Jambda integration commit containing the independent
`jambda-minijam-spec::MiniJamSpec` profile. JamScript remains spec-agnostic and
targets the MiniJAM ABI; it does not target JAM FullSpec directly.

## ABI boundary

| Boundary | JamScript adapter | Observed Jambda / SDK behavior | Result |
| --- | --- | --- | --- |
| Refine entry | `minijam_refine()` | SDK export metadata: input registers `0`, output registers `2` | PASS |
| Refine input | SDK `FETCH` mode `13` | Jambda exposes the payload through the refine host context | PASS |
| Refine output | returned pointer + size in `a0/a1` | VM result reads the returned byte sequence from those registers | PASS |
| Accumulate entry | `minijam_accumulate()` | SDK export metadata: input registers `0`, output registers `0` | PASS after generated-entry repair |
| Accumulate init input | initial A memory, with initial `a0/a1` pointer and length | `VmEngine::new` → `VmState::new` → `VmMemory::new` and register initialization | PASS; no undefined register contents are used |
| Accumulate init fields | FnEncode `(tick, service_id, item_count)` | `AccCtx::build_init_input` emits exactly these three fields | PASS |
| authoritative tick | first init-input FnEncode | `AccCtx::now()` is encoded as the first field | PASS |
| result count | `FETCH` mode `15` until `HOST_NONE` | Jambda returns accumulation operands in order | PASS |
| result payload | packed operand extraction | SDK validates the four hashes/gas/status envelope and returns only the payload | PASS |
| storage read | `READ` | accumulate host call reads the requested JAM service key | PASS |
| storage write/delete | `WRITE`; zero value size deletes | SDK maps zero-sized writes to delete | PASS |
| gas | host call `0` | supplied execution gas is consumed by Jambda | PASS |
| logging | host call `100` | SDK forwards log bytes | PASS |
| dispatch | converter dispatches refine then accumulate | canonical MiniJAM converter uses the two export names | PASS |
| FnEncode | JAM boundary only | used for accumulate init and operand fields, not application ABI | PASS |

The generated Accumulate entry now captures the VM-initialized `a0/a1` pair
at entry and strictly decodes all three init fields. It does not interpret the
zero-input export metadata as a C function with arguments, and it rejects
truncated, overlong, or trailing init input deterministically.

## Runtime semantics

Refine verifies SignedActionV2 network domain, ServiceKey, selector, signature,
payload hash, and nonce. It carries the action's `valid_until` into
`RuntimeRefineOutputV2`; Accumulate compares that value to the authoritative
tick. The comparison is inclusive: `tick < valid_until` and
`tick == valid_until` may commit, while `tick > valid_until` does not. Parent
root comparison is a compare-and-swap guard, and only the runtime-owned
`:jam-service-runtime:managed-state:v1` commitment key is written.

## Versions

| Boundary | Before | After |
| --- | --- | --- |
| SignedAction protocol | V2 | V2 |
| Application ABI | 1 | 1 |
| Managed State protocol | 1 | 1 |
| Managed State layout | 1 | 1 |
| Recovery format | 1 | 1 |
| Builder artifact | 1 | 1 |
| MiniJAM target adapter | 0.2 | 0.2 |

The repair makes the generated Rust declaration match the already-published
zero-input PVM export ABI; it does not change the wire ABI, so the target
adapter version remains `minijam-0.2`.

## Validation

The ABI fixture is in the `jamscript-codegen-rust` test module and covers the
real Jambda init-input shape, tick decoding, malformed input rejection, and
trailing-byte rejection without application logic. The following checks are
required for a release:

```text
cargo fmt --check
cargo test --locked --workspace
cargo clippy --locked --workspace --all-targets --all-features -- -D warnings
cargo build --locked --bin jamscript
./scripts/minijam-network-e2e.sh
```

The local checkout used for this audit is a Stage 0 network and its real
network E2E remains an upstream integration gate. It must be reported as
`REAL_MINIJAM_E2E=FAIL` until an actual MiniJAM node executes Refine,
Accumulate, finalization, managed-state updates, and proof verification.
