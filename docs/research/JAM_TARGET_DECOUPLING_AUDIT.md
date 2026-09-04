# JAM Target Decoupling Audit

Baseline: `b4b69b009eb3aa4e54d3a92daf8e59f9e7c8d456`

Scope: compiler, target, toolchain distribution, and CI paths. MiniJAM RPC
clients and live-network tests are downstream integration concerns and remain
outside the compiler target boundary.

## Classification

| Path | Current owner | Actual purpose | Class | New owner | Migration action |
| --- | --- | --- | --- | --- | --- |
| `crates/jamscript-target-minijam/src/lib.rs` | JamScript | ScriptC build orchestration, native archive compilation, guest ELF production, and ELF -> PolkaVM -> JAM blob conversion | A/B mixed | `crates/jamscript-target-jam` | Migrate the generic build and conversion code; use frozen legacy artifacts for the differential gate before removing the legacy crate |
| `service-toolchain/sdk/include/minijam/*` | MiniJAM checkout | C declarations for host calls, exports, status values, and Blake2 helper | B, with legacy ABI names | `crates/jamscript-target-jam/sdk/include/jam` | Copy only the required target SDK inputs; preserve exported ABI symbols and call numbers |
| `service-toolchain/sdk/src/{host,minijam,crypto}.c` | MiniJAM checkout | Freestanding native shim and PolkaVM metadata records | B | `crates/jamscript-target-jam/sdk/src` | Move the three required source files; do not vendor the SDK checkout |
| `service-toolchain/compiler/polkavm-to-jam` | MiniJAM checkout | `Config::default`, dispatch table, `program_from_elf(JamV1)`, `ProgramParts`, and `ProgramBlob::from_pvm` | B | `crates/jamscript-target-jam` | Replace subprocess/binary invocation with a library API and a small CLI only if needed |
| `crates/service-build-polkavm` | JamScript | Official PolkaVM target selection, guest build, ELF diagnostics, lock validation | B | `crates/service-build-polkavm` | Retain; remove no target semantics from this generic builder |
| `crates/jamscript-codegen-rust` | JamScript | Generated guest/runtime source and current ABI entry symbols | A/B | JamScript runtime/codegen | Retain; entry symbols remain `minijam_*` during compatibility window because they are ABI names |
| `crates/jamscript-cli/src/main.rs` | JamScript | CLI target selection and build orchestration | A | `jamscript-target-jam` | Default build uses `JamTarget`; remove SDK discovery and `JAMSCRIPT_MINIJAM_SDK` from compiler path |
| `crates/jamscript-toolchain/src/lib.rs` | JamScript | Managed bundle layout and provenance | A/B | JamScript toolchain manager | Rename installed target path and metadata to `targets/jam`; remove MiniJAM revision from compiler identity |
| `tools/release/toolchain/build-linux.sh` | JamScript | Reproducible compiler distribution producer | A/B | JamScript release tooling | Build the in-tree JAM target and vendor only its dependencies |
| `.github/workflows/build-toolchain-bundle.yml` | JamScript | Hosted bundle gate | A | JamScript CI | Remove MiniJAM checkout; keep exact LLVM, A/B, and offline smoke gates |
| `tools/minijam-e2e/*`, `scripts/minijam-network-e2e.sh`, `packages/client/*` | JamScript integration | MiniJAM RPC/network and Jambda-facing execution tests | C | Optional MiniJAM integration | Keep separate; never make these compiler/build dependencies |
| `toolchains/minijam.lock` | JamScript | MiniJAM checkout pin used by compiler distribution and integration | C/obsolete for compiler | Optional integration workflow | Retain only until integration workflow no longer needs it, then remove with evidence |
| `docs/minijam-*.md` | JamScript docs | Downstream protocol/integration history and compatibility record | C | Integration documentation | Keep and clarify that MiniJAM is a consumer, not the JAM compiler target |

## Boundary findings

The current converter does not use MiniJAM node semantics. Its generic core is
the locked PolkaVM linker plus `jam-program-blob-common`:

```text
ELF -> TargetInstructionSet::JamV1 -> ProgramParts -> ProgramBlob::from_pvm
```

The current dispatch names `minijam_refine` and `minijam_accumulate` are kept in
the first extraction because they are linker/export metadata and generated ABI
symbols. Renaming them would mix API migration with artifact semantics and is
not part of this change.

The current native C shim uses MiniJAM-prefixed identifiers, but its behavior is
host-call ABI glue rather than node/deployment logic. It is therefore moved as
target SDK source while preserving call numbers, registers, memory conventions,
and entry-point metadata.

## Required migration gates

1. New `JamTarget` builds without a MiniJAM checkout or environment variable.
2. The new converter emits canonical JAM `ProgramBlob`, not raw `.polkavm`.
3. The old and new converter outputs are byte-identical for frozen fixtures.
4. The bundle contains `targets/jam`, never a MiniJAM checkout or Jambda tree.
5. MiniJAM/Jambda tests remain optional downstream integration tests.

The converter gate is recorded separately in
[`JAM_TARGET_DECOUPLING_GOLDEN.md`](JAM_TARGET_DECOUPLING_GOLDEN.md). It uses
the legacy converter's frozen output as the reference and compares both
PolkaVM and canonical blob bytes.

## Known unrelated failure

The pre-existing `jamscript-codegen-rust` workspace test failure is recorded as
unrelated and must be compared against the baseline rather than fixed here.
