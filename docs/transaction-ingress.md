# JamScript Transaction Ingress

JamScript transactions and MiniJAM Work are different layers.

## Ownership of the boundary

A JamScript Transaction is a backend-local logical submission. It is not a
MiniJAM protocol object. The JamScript Backend owns:

- `transactionId` generation and lookup;
- nonce scheduling, queueing, and batching;
- `actionIndex` assignment;
- `RuntimeRefineInputV1` construction and preflight; and
- mapping finalized Work receipts back to logical transactions.

The Formal RPC owns only physical MiniJAM Work. Its mutation and status
methods are:

- `minijam_submitWorkV1`; and
- `minijam_getWorkStatusV1`.

Formal RPC does not understand JamScript actions, transaction IDs, batching,
action indexes, or JamScript receipt policy.

## IDs and mapping

The public transaction API is exposed by the JamScript Backend:

- `jamscript_submitTransactionV1` returns a backend `transactionId`;
- `jamscript_getTransactionStatusV1` resolves that ID through the Work status;
- `packageHash` identifies the physical MiniJAM Work; and
- `actionIndex` identifies the transaction's action within that Work.

One Work may contain multiple logical transactions:

```text
T1 ─┐
T2 ─┼──> packageHash=P
T3 ─┘
```

The Backend retains `T1/T2/T3 -> P` and the corresponding action indexes.
There is no intermediate Formal transaction ID.

## Batching and nonce admission

The backend keeps one insertion-ordered queue per managed Service and permits
at most one in-flight batch for that Service. The default batch limit is four
actions and the flush window is 50 ms; deployments may set
`JAMSCRIPT_BATCH_MAX_ACTIONS` and `JAMSCRIPT_BATCH_FLUSH_MS` explicitly.
Actions selected for one batch are built with one `build_actions` call and
become one physical MiniJAM WorkItem. A second batch is not built until the
first batch has been materialized against the finalized managed-state root.

Wallet-authenticated `submitAction()` calls reserve nonces in a short local
lane, then sign and submit independently. Ownership V2 preparations read one
best snapshot and reserve distinct nonces in their local Ownership lane.
`submitOwnershipAction()` serializes wallet requests until each backend
transaction ID is received; the separate prepare/sign/submit methods allow
callers to coordinate those phases explicitly. These in-memory lanes cover
one `JamScriptClient` instance only. Backend nonce admission remains the
cross-client authority.

The waiting queue is bounded by `JAMSCRIPT_MAX_QUEUED_PER_SERVICE` (default
1024) and `JAMSCRIPT_MAX_QUEUED_PER_SENDER` (default 64). Rejected admissions
return `QUEUE_FULL` with a `retryAfterMs` hint. The active in-flight batch is
separate from those waiting-queue counts.

## Status and finality

The application RPC now exposes `minijam_getBestContext` alongside
`minijam_getFinalizedContext`. A best context is a single block header snapshot
and may be reorged. `minijam_getServiceStorageAt` reads at the exact supplied
block hash. `minijam_getManagedStateV1` accepts that context and verifies the
Service's committed managed-state root before returning data. A best root that
belongs to a known, verified Work prediction can be read from an immutable
snapshot; it does not move the Backend's finalized materialized head.

For its internal node connection, the Backend reads the best hash with
standard Substrate `chain_getBlockHash`, then fetches the header at that exact
hash with `chain_getHeader`. This keeps the hash, block number, and state root
tied to one block even if the head moves between calls. Node RPC remains
private; the application gateway does not forward arbitrary node methods.

Work submission still uses the finalized context required by Formal RPC.
Transactions receive their Backend ID when they enter the bounded Service
queue, and the backend can build best-context reads and report verified best
inclusion. A later Work on the same Service cannot use an unfinalized earlier
Work as its Formal submission context. State-dependent follow-up writes may
therefore wait for finality even though unrelated submissions are accepted and
tracked immediately.

After a Work is submitted, a temporary Work-not-found response is reported
to the logical client as `packaged`, not as a permanent failure. Work status is
mapped by the Backend as follows:

```text
insufficient_workers  -> packaged
awaiting_candidate    -> refining
voting/accepted       -> reported
failed                -> failed
imported               -> reported until the canonical root is materialized
imported + canonical new root -> imported
```

An imported Work is not sufficient by itself to publish an application
receipt. The Backend compares the finalized managed-state commitment with the
predicted parent and new roots, materializes only a canonical transition, and
then exposes the corresponding `ActionReceipt`.

Transaction status includes `bestChainStatus` and `finalized`. `imported` with
`bestChainStatus: "included"` means the predicted managed-state root was
observed at a verified best context. It remains reorgable and carries no
finalized action receipt. Finalized receipts are exposed only after the
finalized managed-state commitment matches the prediction. If the backend
cannot verify the requested best snapshot, it reports an unknown best-chain
status or returns `BEST_CONTEXT_UNAVAILABLE`; it never substitutes finalized
state for a best query.

Mutation transport failures are not retried automatically. An ambiguous
Ownership submission blocks that local nonce lane until a best-context nonce
read proves the nonce advanced. The waiting queue, transaction ID map,
idempotency keys, and in-flight Work mapping are not yet durably restored on
Backend restart, and submit requests do not yet accept a durable idempotency
key. A lost response or restart can therefore leave a transaction requiring
manual reconciliation; do not blindly replay its signed action.

## Client use

`submitAction()` and `submitOwnershipAction()` both submit through
`jamscript_submitTransactionV1`. `waitForAction(transactionId, ...)` polls
`jamscript_getTransactionStatusV1` and selects the caller's receipt by
`actionHash`.

## Stage-1 baseline snapshot

Read-only sampling of the public application `/rpc` on 2026-10-02 returned
finalized block 8277 in 134 ms. `minijam_getBestContext` was not exposed by that
deployment at the time of the sample. This confirms the missing best-context
API boundary, but it does not measure submission-to-best or best-to-finalized
latency. No signed transaction was submitted for this snapshot.
