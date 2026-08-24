# M0.5 reachability probe

`scalar_root.c` is a host-only linker root. It exists to make the linker resolve the
generated ScriptC scalar entry and produce a map of runtime objects pulled from the
library archive. The host executable is diagnostic only; it is not a PVM artifact and
must never be used as an acceptance result.
