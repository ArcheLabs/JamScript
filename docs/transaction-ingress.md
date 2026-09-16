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

## Status and finality

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

Mutation transport failures are not retried automatically. If submission
outcome is unknown, the Backend preserves the diagnostic rather than creating
another Work blindly.

## Client use

`submitAction()` and `submitOwnershipAction()` both submit through
`jamscript_submitTransactionV1`. `waitForAction(transactionId, ...)` polls
`jamscript_getTransactionStatusV1` and selects the caller's receipt by
`actionHash`.
