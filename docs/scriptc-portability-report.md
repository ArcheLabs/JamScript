# ScriptC portability report

## M0.6 result

**M0.6 PASS.** The deterministic ScriptC scalar profile now has an actual
RISC-V/PVM execution path. This is a probe result, not a production ScriptC
backend and not permission to start npm-static integration.

The accepted Legacy backend was not changed.

The reproducible path is:

```text
Node 24.15.0
  → ScriptC TypeScript → generated C
  → clang rv64emac/lp64e relocatable C objects
  → selected deterministic runtime archive
  → official PolkaVM Rust/rust-lld guest link
  → service.elf
  → polkavm-linker JamV1 conversion
  → PVM interpreter
```

Run it after installing the locked JavaScript dependencies with Node 24:

```text
cd toolchains/scriptc
npm ci --ignore-scripts
cd ../..
SCRIPTC_NODE=/path/to/node-24.15.0/bin/node \
  cargo run --manifest-path tools/pvm-scriptc-m0-runner/Cargo.toml --offline
```

The runner rejects any Node version other than 24.x. The recorded execution
used Node `v24.15.0`, generated the scalar C during the run, built
`target/pvm-scriptc-m0/service.elf`, converted it, and executed the resulting
PVM artifact. The generated artifacts are `scalar.elf`, `scalar.polkavm`, and
`scalar.pvm` in that output directory.

## Measurements

| Stage | Meaning | Result | Gas | Allocations | Requested bytes | High-water |
| ---: | --- | --- | ---: | ---: | ---: | ---: |
| 1 | entry + return | PASS | 12 | 0 | 0 | 0 |
| 2 | ScriptC library init/reset | PASS | 218 | 0 | 0 | 0 |
| 3 | generated scalar `number` add (`20 + 22`) | PASS | 348 | 0 | 0 | 0 |
| 4 | native `uint64_t` add (`41 + 1`) | PASS | 16 | 0 | 0 | 0 |

Gas is measured before the diagnostic metric getter calls. The machine
readable stage report is written to `target/pvm-scriptc-m0/m06.json`.

The final ELF has no undefined symbols. The linked floating-point helpers are
the exact-target compiler-builtins path supplied by the PolkaVM Rust build;
the final image contains the comparison/addition helpers needed by the
scalar profile, including `__adddf3`, `__eqdf2`, `__ltdf2`, `__nedf2`, and
`__unorddf2`. No ad-hoc floating-point arithmetic was added.

## Selected runtime and dependency boundary

Only these ScriptC runtime translation units are compiled for the probe:

| Object source | text | data | bss | Classification |
| --- | ---: | ---: | ---: | --- |
| generated `scalar.lib.c` | 245 | 0 | 1 | ScriptC entry and f64 operation |
| `scr_library.c` | 2,365 | 112 | 3,392 | library entry, reset, trap funnel, result arena |
| `scr_number.c` | 15,795 | 0 | 0 | numeric conversions and helper reachability |
| `scr_string.c` | 22,159 | 0 | 144 | string/trap/error support reached by the shared runtime |
| `scr_array.c` | 5,969 | 0 | 0 | shared value/exception support |
| `scr_bytes.c` | 19,949 | 288 | 0 | library result/reset support |
| `scr_cycle.c` | 2,930 | 4 | 208 | deterministic cycle collector |
| `scr_error.c` | 3,257 | 560 | 9 | error values and type table |
| `scr_exception.c` | 2,658 | 8 | 1,084 | entry exception check/trap path |
| `scr_object.c` | 226 | 0 | 0 | shared error/object support |
| `scr_lib_cleanup.c` | 2 | 0 | 0 | explicit restricted-profile cleanup shim |
| native `u64_add.c` | 4 | 0 | 0 | integer comparison control |

The complete per-object undefined-symbol classification is emitted by the
build into `target/pvm-scriptc-m0/runtime-dependencies.json`. It records the
undefined set, ScriptC references, libc-like references, libm references,
compiler-rt references, and any other references for every object.

The earlier host link-root experiment pulled `scr_map.c` and `scr_console.c`
because those are coarse translation units in the host archive. They are not
present in the final ELF symbol set: section garbage collection removes them
from the actual PVM link. `scr_string`, `scr_array`, `scr_bytes`, and
`scr_object` remain as shared runtime/exception dependencies, not because the
scalar program uses their public application APIs.

`scr_lib.c` is intentionally excluded. It is the ScriptC process/filesystem/
network/POSIX runtime and would cross the M0 stop-loss boundary. The
`scr_lib_session_cleanup` replacement is a diagnostic-only no-op for this
profile; it is valid only because the fixture reaches no process values. A
future profile that reaches process APIs must reject this shim and provide a
real deterministic implementation.

The guest provides diagnostic-only bounded `malloc`, `calloc`, `realloc`, and
`free`, with fixed 2 MiB storage and no OS allocator. It also provides the
required deterministic memory/string primitives and trap-safe stubs. ScriptC
retain/release and cycle operations remain in the linked runtime; ownership
was not removed. The guest allocator records allocation count, requested
bytes, and high-water mark and traps on capacity exhaustion.

## M0 and M0.5 inputs

- ScriptC release: `v0.0.34`.
- Upstream revision: [`toolchains/scriptc/REVISION`](../toolchains/scriptc/REVISION).
- Node pin: [`toolchains/scriptc/NODE_VERSION`](../toolchains/scriptc/NODE_VERSION).
- npm dependency resolution: [`toolchains/scriptc/package-lock.json`](../toolchains/scriptc/package-lock.json), generated and consumed by Node 24.
- Published surface manifest hash: `5bc22383db4e57edf171bfeb6dd518e964844efae54b6a2574e2bd29afb134c9`.

The M0 generator remains [`toolchains/scriptc/m0/run.mjs`](../toolchains/scriptc/m0/run.mjs)
and covers scalar, control flow, arrays, strings, and bytes. The M0.5
isolated target probe remains [`toolchains/scriptc/m0/m05.mjs`](../toolchains/scriptc/m0/m05.mjs);
it records the initial soft-float and target-object evidence. `u64_add.c`
has no undefined symbols, while the f64 probe reaches `__adddf3`, as expected
for `rv64emac/lp64e` software floating point.

## Scope decision

M0.6 demonstrates that the restricted deterministic core is representable and
executable on the current PVM target with a bounded diagnostic runtime. It
does not claim that the complete ScriptC runtime is freestanding: POSIX,
filesystem, networking, TLS, threads, clocks, entropy, and general libm
remain outside this profile. No production integration is started by this
probe result.
