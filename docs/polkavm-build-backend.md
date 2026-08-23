# PolkaVM service build backend

JamScript services use the PolkaVM 0.30 compatibility domain pinned in
`toolchains/polkavm.lock`. The target JSON is selected by
`polkavm-linker::target_json_path(TargetJsonArgs::default())`; it is never
copied into this repository.

The MiniJAM target adapter compiles the host ABI and native C modules with
Clang 20 into PIC static archives. `service-build-polkavm` then builds the
guest as a `cdylib` with the linker-owned target. Cargo/rust-lld produces the
canonical RISC-V ELF; the MiniJAM converter consumes that ELF afterward.

The generated guest only contains service dispatch and application code.
Allocator, panic behavior, memory primitives, minimum stack declaration, and
diagnostic stage reporting live in `service-runtime-guest::guest_support`.

The `-z notext` linker override is deliberately owned by the MiniJAM target
adapter, not the language-agnostic backend. MiniJAM metadata contains
function-pointer relocations which must survive the final `rust-lld` ELF link;
the adapter records the exact override as `finalElfLinkerOverrides` and has a
regression test for its two flags. Generic PolkaVM guests do not inherit it.

## Local verification

```text
cargo test --workspace --locked --offline
JAMSCRIPT_MINIJAM_SDK=/path/to/minijam-client \
  cargo run -p jamscript -- build examples/counter --output /tmp/counter-polkavm
bash tools/pvm-minimal-probe/run.sh
```

The diagnostic build emits `readelf.txt`, `relocations.txt`, and
`symbols.txt`. The backend rejects non-RISC-V/non-RVE ELF or non-PIE ELF and
always rejects undefined system symbols. MiniJAM-specific required exports
and relocation requirements are supplied by the target adapter; generic
guests may omit them.

The C compiler is not a final linker. If an archive cannot satisfy the
official PolkaVM `rust-lld` link contract, the build fails.
