# ScriptC M0 portability report

Status: M0 feasibility gate **not passed**; investigation artifacts are in place.

## Pinned inputs

- ScriptC release: `v0.0.34`
- ScriptC upstream commit: `d7b4480` (recorded in [`toolchains/scriptc/REVISION`](../toolchains/scriptc/REVISION))
- npm package: `@scriptc/compiler@0.0.34`
- Node version selected for the reproducible lane: `24.15.0` (see [`toolchains/scriptc/NODE_VERSION`](../toolchains/scriptc/NODE_VERSION)). ScriptC release notes require Node 24 or newer for this baseline.
- Current workspace probe host: Node 22.22.2; ScriptC v0.0.34 requires Node 24 or newer, so the result below is exploratory and is not a production M0 pass.
- Published surface manifest SHA-256: `5bc22383db4e57edf171bfeb6dd518e964844efae54b6a2574e2bd29afb134c9`.

## Probe

The reproducible probe is [`toolchains/scriptc/m0/run.mjs`](../toolchains/scriptc/m0/run.mjs). It runs ScriptC library mode with dynamic fallback disabled and covers:

1. scalar arithmetic;
2. control flow (`for` and `if`);
3. arrays;
4. deterministic strings;
5. `Uint8Array` bytes.

Run it from the repository root with:

```text
cd toolchains/scriptc
npm install
node m0/run.mjs
```

The npm lockfile is deliberately not claimed yet: the current environment could not resolve all
ScriptC transitive metadata reproducibly. A lockfile is required before M1 or any CI integration.

The probe emits C, an archive, and ScriptC IR for every sample. It verifies the pinned surface
manifest before compiling and writes a machine-readable summary to
`toolchains/scriptc/m0/out/results.json`.

The exploratory run on the available environment produced:

| Sample | ScriptC library mode | Notes |
| --- | --- | --- |
| scalar | PASS | C archive emitted |
| control-flow | PASS | `for` + `if` emitted |
| arrays | PASS | local array path emitted; array ABI parameters are not claimed |
| strings | PASS | string ABI path emitted |
| bytes | PASS | `Uint8Array` ABI path emitted |

## Required next measurements

For every successful archive, inspect undefined symbols with:

```text
llvm-nm -u toolchains/scriptc/m0/out/<sample>/<sample>.lib.a
```

The exploratory archive set contains ScriptC runtime objects such as `scr_number.o`,
`scr_string.o`, `scr_array.o`, `scr_bytes.o`, `scr_library.o`, and `scr_library.program.o`.
The union of undefined symbols includes deterministic-looking memory/math/allocator symbols
(`malloc`, `calloc`, `realloc`, `free`, `memcpy`, `memmove`, `memset`, `memcmp`, `fmod`,
`floor`, `ldexp`, `trunc`) and also host/OS symbols (`clock_gettime`, `arc4random_buf`,
`fopen`, `read`, `write`, `getenv`, `pthread_sigmask`, `uname`, `socket-related helpers`,
and filesystem/network/process helpers). This is an archive-level dependency inventory, not yet
a reached-code proof; it is sufficient to block claiming freestanding PVM portability.

The generated objects are host `x86-64` objects in the default profile run. Setting
`SCRIPTC_TARGET=riscv64-unknown-elf` reaches ScriptC's cross-target gate, which requires
`SCRIPTC_CC=zigcc`; Zig is not installed in this workspace. The existing JamScript clang/rust-lld
pipeline therefore cannot consume these archives as-is. This is the concrete M0 blocker, not a
reason to modify the accepted PolkaVM backend.

Classify each symbol as compiler/runtime, memory/string libc, math, allocation, OS, thread/TLS,
clock/random, or unsupported target ABI. Then repeat the build with the JamScript target driver
and the existing MiniJAM/PVM converter. The M0 gate is not passed by a host archive alone.

## Current conclusion

No ScriptC backend has been added to JamScript yet. The legacy backend remains the only accepted
production backend. M0 passes only after the probes are built under the pinned Node version, the
reachable runtime symbols are classified, no dynamic engine or OS dependency is present, and at
least the scalar sample links as `riscv64-unknown-elf` and converts to PVM. The current result
therefore stops at investigation: the next required work is Node 24 execution, reached-object
dead-code analysis, a freestanding runtime subset, and an actual RISC-V/PVM link-and-convert
probe.

## M0.5 initial evidence

The isolated target probe is [`toolchains/scriptc/m0/m05.mjs`](../toolchains/scriptc/m0/m05.mjs).
It bypasses ScriptC's `zigcc` driver and compiles generated C with the existing JamScript target
flags. The first scalar compilation produced a valid RISC-V relocatable object and its
undefined-symbol set included:

```text
__adddf3
scr_error_vts
scr_library_check_exc
scr_library_entry
scr_library_reset
scr_library_set_sink
```

The `__adddf3` result confirms that ordinary ScriptC `number` arithmetic reaches software
floating point on `rv64emac/lp64e`. A standalone `uint64_t` addition probe is included for the
comparison. The generated scalar C still requires the ScriptC library runtime; its RISC-V object
does not constitute a link or PVM acceptance.

After the M0 host artifacts exist, run:

```text
SCRIPTC_M0_RUNTIME=toolchains/scriptc/node_modules/@scriptc/runtime \\
  node toolchains/scriptc/m0/m05.mjs
node toolchains/scriptc/m0/reachability.mjs
```

The first command records object sizes and undefined compiler/runtime symbols for the double,
u64, and generated ScriptC scalar probes. The second command is explicitly host-only and records
which archive members are pulled by a scalar link root; it is not allowed to stand in for a PVM
execution.

The host-only link-root diagnostic is [`toolchains/scriptc/m0/reachability.mjs`](../toolchains/scriptc/m0/reachability.mjs).
It produces `scalar-link.map`, `scalar-reachable-symbols.txt`, and
`scalar-unresolved-symbols.txt`. Its current map pulls these runtime units for the scalar entry:

```text
scr_exception.o scr_error.o scr_cycle.o scr_library.o scr_number.o
scr_string.o scr_array.o scr_bytes.o scr_object.o scr_lib.o scr_map.o
scr_console.o
```

It does not pull the network, filesystem, URL, or async runtime units at the object level. This is
the first useful reachability result, but it is not yet a final freestanding proof because several
pulled units are compiled as coarse translation units and still expose broad libc dependencies.

M0.5 is therefore currently **CONDITIONAL**, pending the reachable runtime subset, explicit
soft-float/compiler-rt decision, and a real freestanding ELF/PVM execution. No production
ScriptC backend or production allocator has been changed.
