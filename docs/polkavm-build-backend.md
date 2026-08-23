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

## Local verification

```text
cargo test --workspace --locked --offline
JAMSCRIPT_MINIJAM_SDK=/path/to/minijam-client \
  cargo run -p jamscript -- build examples/counter --output /tmp/counter-polkavm
bash tools/pvm-minimal-probe/run.sh
```

The diagnostic build emits `readelf.txt`, `relocations.txt`, and
`symbols.txt`. The backend rejects non-RISC-V/non-RVE ELF, non-PIE ELF,
missing MiniJAM exports, undefined symbols, or an ELF without relocations.

The C compiler is not a final linker. If an archive cannot satisfy the
official PolkaVM `rust-lld` link contract, the build fails.
