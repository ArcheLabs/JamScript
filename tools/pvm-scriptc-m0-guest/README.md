# ScriptC M0.6 guest

This is a diagnostic-only guest. Its build script links the generated scalar
ScriptC C module with the scalar-reachable ScriptC runtime translation units
using the existing `clang` `rv64emac/lp64e` target. It intentionally omits
`scr_lib.c`: that translation unit is the POSIX process/filesystem/network
runtime and is outside the restricted PVM profile.

The allocator and libc symbols in `src/lib.rs` are bounded probe shims. They
are not a production ScriptC runtime or a claim that the complete ScriptC
runtime is freestanding-portable.
