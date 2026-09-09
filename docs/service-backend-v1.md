# JamScript Service Backend V1

Status: implementation contract for the pre-v0.1 backend.

## 1. Scope

The JamScript Service Backend is the single application-facing endpoint between
`@jamscript/client` and a JAM-compatible network. It is not part of JAM
consensus and it is not the Formal RPC.

V1 uses exactly one managed-state trie root per deployed JamScript Service.
Multiple roots/trees per Service are intentionally deferred. Applications that
need multiple logical stores (for example an asset center with many fungible
assets) namespace keys inside the one trie, e.g. by `assetId`.

## 2. Canonical state rule

For every deployed Service `S`, the only canonical managed-state head is the
`ManagedStateCommitmentV1` stored at `MANAGED_STATE_COMMITMENT_KEY_V1` in the
native JAM storage of `S`.

The backend's materialized trie is an availability/proof cache. It is never the
source of canonicality.

## 3. Refine and accumulate are different trust boundaries

### Refine

Refine does not read another Service's JAM storage.

The backend supplies authenticated state witnesses. Refine locally verifies the
trie proof against the supplied root before application execution can consume a
value. A cross-Service read therefore has the form:

```text
Service B state root rB
        +
proof(key -> value under rB)
        |
        v
Service A refine verifies locally
```

A successful refine result must commit every external `(serviceId, root)` it
used.

### Accumulate

Accumulate is the canonicality gate.

For A's own managed state it already performs a compare-and-set transition:

```text
current(A) == refine.parentRoot(A)
    => commit refine.newRoot(A)
```

For every external dependency `(B, rB)` committed by refine, accumulate must
read B's native `MANAGED_STATE_COMMITMENT_KEY_V1` through JAM `READ` with
`serviceId = B` and require:

```text
current(B) == rB
```

Only after all dependency roots still match may A's own root advance. If B
changed between refine and accumulate, A's transition is stale and is not
applied.

This is deliberately not a cross-Service read during refine. Refine proves;
accumulate revalidates canonical roots.

## 4. Cross-Service dependency identity

Consensus-facing dependency identity is the numeric JAM `serviceId`, because
that is what JAM accumulation can address and verify through native Service
storage.

`ServiceKeyV1` remains a stable JamScript logical identity used by the backend
and signing domains. The backend registry binds:

```text
serviceId -> { serviceKey, codeHash, deployment metadata }
```

but a cross-Service root dependency must not rely on an off-chain ServiceKey to
establish chain identity.

## 5. Multi-Service backend invariants

One backend process serves many deployed Services.

All mutable backend state is Service-scoped:

```text
ServiceRegistry
  serviceId -> ServiceRecord

FullStateProvider
  serviceKey -> stateRoot -> materialized trie

pending work
  (serviceId, packageHash) -> predicted transition

predictions
  (serviceId, packageHash) -> predicted transition

recovery log
  serviceId + serviceKey + RuntimeRefineOutputV1
```

A package hash alone is not a backend-global key.

The backend must reject a request when its `serviceId`, registered `serviceKey`
or registered `codeHash` do not agree with the deployment record.

## 6. Developer-facing state model

Application code should not construct or verify `StorageProof` objects.

The desired application boundary is typed state access. The runtime/compiler
turns external state reads into proof requirements; the backend discovers or
receives those requirements, materializes the required witnesses, and the guest
verifies them.

The proof plumbing is therefore below the JamScript application API:

```text
application typed read
        |
compiler/runtime access request
        |
backend witness construction
        |
refine proof verification
        |
accumulate root revalidation
```

## 7. Backend/public boundary

The browser/client talks to one JamScript backend endpoint. The backend owns the
network topology and internally talks to:

- the JAM/MiniJAM node for finalized context and native Service storage;
- Formal RPC for work submission/status;
- its materialized managed-state provider for value/proof availability.

Frontend state queries still receive `value + StorageProof` and verify the
proof locally. A single endpoint does not imply blindly trusting backend query
values.

## 8. Deferred multi-root model

A future protocol can replace the single commitment key with named tree
commitments:

```text
(serviceId, treeId) -> ManagedStateCommitmentVn(root)
```

An asset center could then dedicate one tree to each asset while an identity
service could continue using one tree. V1 intentionally keeps `treeId`
implicit and fixed to the single Service tree so that backend productization is
not blocked by a storage-layout feature.

## 9. Required release gates

The backend is not release-ready until automated tests cover at least:

1. two Services with independent managed roots in one backend process;
2. identical package hashes cannot collide across Services;
3. persistent recovery replays two Services independently;
4. tampered own-state proof is rejected in refine;
5. tampered external-state proof is rejected in refine;
6. external root unchanged between refine and accumulate -> transition applies;
7. external root changed between refine and accumulate -> transition does not apply;
8. own parent root stale at accumulate -> transition does not apply;
9. frontend query proof verifies against the requested Service/root/key;
10. restart preserves proof availability for all registered Services.
