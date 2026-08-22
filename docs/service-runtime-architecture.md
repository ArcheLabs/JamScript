# JAM Service Runtime Architecture

The Service Runtime is a language-independent foundation for a JAM Service.
JamScript is one frontend; Rust and C applications can target the same runtime
wire formats and guest entry points.

```text
language frontend → Service Runtime Guest → MiniJAM/JAM adapter
                           ↕
                    authenticated state
                           ↕
                    State Provider
```

The dependency direction is one-way: frontend and code generation depend on
the runtime, never the reverse. `service-runtime-core` is `no_std` and defines
the protocol types. `service-runtime-state` wraps the Polkadot SDK trie
primitives. `service-runtime-host` provides full-state/proof recording and a
reference in-memory provider. `service-runtime-guest` contains the final
PVM-facing application execution interface.

JAM remains the canonicality source. A provider only answers requests for an
explicit `(service_id, state_root, key)` and never chooses a latest root.
Refine receives a historical witness and produces a transition; Accumulate
only performs the commitment compare-and-swap. Runtime state is not a second
cross-service time model and does not replace JAM lookup-anchor semantics.

The current `jamscript-runtime` and `jamscript-runtime-core` crates remain as
compatibility surfaces while generated services move to the new foundation.
They no longer include the ServiceId in managed application keys.

## Reserved JAM storage

Normal JAM Service KV contains only the Runtime commitment under
`:jam-service-runtime:managed-state:v1`. Applications use the authenticated
managed trie. Raw JAM storage is a separate capability and must reject writes
to the reserved commitment key; only the runtime-owned accumulate commit path
may publish a verified new root.

## Execution phases

`RefineContext`-style APIs expose managed state, historical lookup and proof
backed execution. `AccumulateContext`-style APIs expose mutable JAM storage,
transfers and service management. This separation is represented by distinct
runtime interfaces; application execution does not receive an arbitrary JAM
storage handle.

## Version domains

The following versions are independent and are emitted in `build.json`:

- Service code/package version;
- Managed State protocol version 1;
- Managed State layout version 1;
- Recovery format version 1;
- SignedActionV1 wallet protocol version.

Changing the SDK trie encoding requires a managed-state layout version change,
not a silent dependency upgrade.
