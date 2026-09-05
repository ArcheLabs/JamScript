# JAM Target Differential Golden Fixtures

These fixtures cover the migration gate for the ELF converter. For each
fixture, the existing legacy build output is treated as the frozen input and
reference, and `jamscript-target-jam::elf_to_jam_blob` is run on the exact same
ELF bytes. This isolates converter identity from unrelated differences in
guest generation or managed-toolchain provenance.

The legacy implementation is the pinned MiniJAM-side converter at
`service-toolchain/compiler/polkavm-to-jam/src/main.rs`; the new implementation
is `crates/jamscript-target-jam/src/lib.rs::elf_to_jam_blob`.

| Fixture | ELF SHA-256 | Legacy PolkaVM SHA-256 | New PolkaVM SHA-256 | Legacy blob SHA-256 | New blob SHA-256 | Result |
| --- | --- | --- | --- | --- | --- | --- |
| `dynamic-state-scriptc` | `706d1ecdef01c9f914c022b731095d85f53d703848e138063e5dba5cc03875c5` | `9344cbb36ec1ee8aa44f58f0379c668661a08059b0179f015132be72f1b32d61` | `9344cbb36ec1ee8aa44f58f0379c668661a08059b0179f015132be72f1b32d61` | `5b98881ff8adb513fb5dd585c2aa8ebb3b32876e2b588ea65c59499121cf4e85` | `5b98881ff8adb513fb5dd585c2aa8ebb3b32876e2b588ea65c59499121cf4e85` | PASS |
| `counter-scriptc` | `a03bd12582fc1d5f1a628d1e5173f5ca1d67486f7dc1bdaf35fb89bd25b49e7e` | `05f0f420354a94cdad34c8bf4313d28f0a71a9cef3f4566c130d35ae59f7053f` | `05f0f420354a94cdad34c8bf4313d28f0a71a9cef3f4566c130d35ae59f7053f` | `72fda32bfe6085400eff4d47e3ce26545027c5e4f1c80fb79b044283ff0c7bc8` | `72fda32bfe6085400eff4d47e3ce26545027c5e4f1c80fb79b044283ff0c7bc8` | PASS |

The new converter uses the locked `polkavm-linker = 0.30.0` and
`jam-program-blob-common = 0.1.28` dependencies. A mismatch in either output
is a promotion blocker.
