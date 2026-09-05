# JAM target architecture

JamScript compiles services for the canonical JAM target. MiniJAM and Jambda
consume the resulting JAM ProgramBlob; neither repository is a compiler input.

```text
JamScript source -> IR -> backend -> RISC-V ELF
  -> PolkaVM JamV1 linker -> ProgramParts -> canonical JAM ProgramBlob
       -> Jambda or MiniJAM
```

The target implementation is owned by
[`crates/jamscript-target-jam`](../../crates/jamscript-target-jam). It owns the
JAM target SDK and calls the locked `polkavm-linker` and
`jam-program-blob-common` APIs directly. `jamscript build` therefore produces
`service.elf`, `service.polkavm`, and `service.blob` without a node checkout,
RPC connection, or deployment environment.

`service.polkavm` is the PolkaVM-native debug/intermediate serialization.
`service.blob` is the canonical JAM ProgramBlob used for deployment and
execution. The existing `minijam_*` entry and host-call names remain stable ABI
names during the migration; their ownership is now the JAM target SDK.

MiniJAM network deployment and Jambda conformance belong to optional downstream
integration workflows. They do not gate ordinary compiler correctness or the
managed toolchain build.
