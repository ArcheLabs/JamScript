# Managed State V1

Managed state is one authenticated trie per Service. The trie uses
Substrate `LayoutV1<Blake2Hasher>` through the Polkadot SDK 2606 line:

```text
sp-core          = 43.0.0
sp-trie          = 46.0.0
sp-state-machine = 0.53.0
```

The state root is a 32-byte protocol value. The commitment stored in JAM KV
is exactly 34 bytes:

```text
u8 protocol_version = 1
u8 layout_version   = 1
[u8; 32] state_root
```

The empty root is computed by the selected SDK trie layout, not represented by
zeroes. The host and verifier use the same root implementation.

## Keys

Managed application keys are not hashed and do not include ServiceId because
each Service owns a separate trie:

```text
0x01
u16_le(namespace_length)
namespace
canonical_user_key
```

Runtime internal keys use:

```text
0x00
module_id
module_specific_key
```

Wallet nonce is module `0x01`, followed by the 32-byte account key.

## Transitions and proofs

Host execution records one multiproof from the full trie. The verifier opens
the supplied `StorageProof` at the declared parent root. A missing node is a
witness error; a valid non-inclusion path is a normal `None` value.

`StateDiffV1` is sorted by raw key, forbids duplicate keys, and has explicit
encoding. `StateTransitionV1` carries parent root, new root and the diff hash.
The ordinary receipt carries action hash/status/error only; it does not carry
the complete diff.

The reference implementation includes fixtures for host/verifier agreement,
missing witness rejection, and transactional overlay behavior.

## Query contract

Provider queries always include the explicit service ID and root:

```text
get(service_id, state_root, key) → value + StorageProof
```

The client first resolves the finalized JAM commitment, then asks the provider
for that explicit root and key. It validates the returned service/root/key
tuple and verifies the returned `StorageProof` against that root with the same
`LayoutV1<Blake2Hasher>` before decoding the application value. Provider
availability is not canonicality: only the finalized JAM commitment selects
the canonical root, while a provider may retain and serve any number of
historical `(ServiceKey, StateRoot)` snapshots.

Managed-state reads never implicitly fall back to JAM Service KV. The client
retains an explicit `legacyServiceKvFallback` compatibility option, defaulting
to `false`; applications should leave it disabled for authenticated managed
state.

## Work builder

`ManagedStateWorkBuilder` anchors every build to a newly resolved finalized
context. It reads `MANAGED_STATE_COMMITMENT_KEY_V1` from that context's native
Service storage, opens the provider snapshot at the resulting explicit root,
and executes the application once against the full state. The host transaction
records all actual trie accesses, including dynamic keys and valid
non-inclusion reads, and turns them into the `RuntimeRefineInputV1` witness.

The builder then executes the identical input through `ProofState`. Producer
and proof-backed outputs must match exactly before the work is returned. A
provider's `materialized_root` is never consulted for canonicality. If the
finalized root is unavailable, malformed, or does not match the supplied full
state, the build fails.

Each builder invocation resolves its context and commitment again. Retrying
after a stale Work anchor therefore reuses the signed action but rebuilds the
root, witness, refine input, and Work payload. `build_one` is a convenience
wrapper over the batch-shaped `build_actions` API.

## Generated Builder application

`jamscript build` emits both `generated_service.rs` for the PolkaVM guest and
`generated_builder_application.rs` for producer-side witness discovery. Both
embed the same generated `ServiceApplication` semantics: selector, application
ABI decoder, wallet authentication, nonce transitions, state keys, business
transactions, and native ABI calls. `builder.json` lists the generated host
application and the native source/include inputs required to compile it.

The Builder must consume these artifacts rather than maintain a handwritten
companion implementation for each Service. Native modules are compiled from
the same declared source files for the PolkaVM and host targets. This artifact
does not contain PVM hostcalls or Accumulate logic.

JamScript application ABI encoding remains separate from JAM/PVM protocol
encoding. In particular, `Bytes<N>` uses a fixed little-endian `u32` length in
the application ABI; JAM FnEncode is used only by the JAM protocol boundary.
