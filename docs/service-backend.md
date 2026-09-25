# JamScript Service Backend

`crates/jamscript-service-backend` contains the multi-Service backend core.
It keeps the canonical chain root outside the local cache and routes every
materialized snapshot, prediction, planner, and recovery record by
`(serviceId, serviceKey)`.

The backend surface is one JSON-RPC endpoint. The core handler exposes:

- `jamscript_getCapabilitiesV1`
- `jamscript_listServicesV1`
- restricted `jamscript_registerServiceV1`
- `minijam_submitWorkV1` and `minijam_getWorkStatusV1` through a network gateway
- `jamscript_getStateV1` (trusted proofless state)
- `jamscript_getStateProofV1` (explicit proof state)
- `jamscript_getServiceStateStatusV1`
- `minijam_getManagedStateV1`
- `jamscript_getPredictionV1`

Registration discovers canonical Service metadata through `BackendNetwork` and
binds the content-addressed PVM loader only after metadata and code-hash
verification. See [`backend-pvm-runtime.md`](backend-pvm-runtime.md).

Deployment can provision the backend after on-chain creation by configuring
`backend_rpc` in the selected network (or `JAMSCRIPT_BACKEND_RPC`). `jams
deploy` records the chain deployment before attempting the separate backend
prewarm call, so a backend outage is reported independently and can be
retried without recreating the Service. The normal path does not require an
admin token.

The backend builds proof witnesses only at the root reported by canonical
Service storage. Refine receives external witnesses as read-only views and
emits JAM `serviceId`/root dependencies. Accumulate performs the final
ServiceId-scoped commitment checks. Local state is materialized only after the
predicted root is observed canonically.

Network profile names are a CLI/configuration concern. `jams backend start
--network <name>` resolves `[networks.<name>]`, checks its configured genesis
pin against Node RPC, and launches the backend with concrete Node and Formal
RPC endpoints. The backend binary has no `--network` flag or local/testnet
branching. Keep Node RPC, Formal RPC, and the backend's host-published port on
loopback or trusted private infrastructure; do not expose Node RPC to the
public Internet. For the backend Compose deployment on the same host as the
MiniJAM compact profile, the backend container joins the internal
`minijam-testnet-chain` Docker network and uses `node:9944` and
`formal-rpc:8080`. The network can be overridden with `MINIJAM_CHAIN_NETWORK`
for an operator-managed private network. The backend host port defaults to
`127.0.0.1:8090`; its container process listens on `0.0.0.0:8090` so the
loopback-published port can reach it.

Production persistence is RocksDB `0.25.0` under `<data-dir>/db` with fixed
`meta`, `services`, `heads`, `state`, and `transitions` column families. State
keys are `[serviceId BE u32] || application key`; a finalized transition
writes state changes, the durable head, and the transition envelope in one
synced WriteBatch. Startup binds the database to schema v1 and the node's
genesis hash, rebuilds in-memory proof caches from durable KV, and isolates a
corrupt Service without hiding healthy Services. The old length-delimited
recovery format remains only as a compatibility API and is not used by the
production binary.

The TypeScript client uses `jamscript_getStateV1` by default. Applications that
need independent verification set `stateVerification: "proof"`; the legacy
`minijam_getManagedStateV1` explicit-root endpoint remains available.

The older `tools/managed-state-network-adapter` is a legacy compatibility/test
wrapper for the generated Builder path. It is not the production topology.
# Transaction coordinator lifecycle

The backend coordinator advances queued transactions independently of browser
polling. A background loop reconciles submitted work against MiniJAM/Formal,
releases finalized or terminally failed batches, and then dispatches the next
eligible queue. Reconciliation runs every 250 ms by default and can be tuned
with `JAMSCRIPT_RECONCILE_INTERVAL_MS` (minimum 100 ms). Transaction status
requests are observations; clients do not need to poll to release scheduler
locks.

Signed actions with an expired `validUntil` slot fail before dispatch, and
actions with a stale Ownership nonce fail with `STALE_NONCE`. Future nonces
remain queued until the missing earlier nonce is finalized. Non-final Formal
states and temporarily missing work remain in flight and are retried.

The coordinator queue, transaction-to-batch mappings, and in-flight metadata
remain process-local and are lost on backend restart. Chain state is durable;
clients recovering an unknown transaction must verify the action's effect on
chain before deciding whether to resubmit. Full queue persistence is not part
of this backend version.

The optional `jamscript_getCoordinatorStatusV1` diagnostic RPC requires
`JAMSCRIPT_BACKEND_ADMIN_TOKEN` in the backend process and the same token in
the request's `adminToken` field. It returns transaction IDs, sender key
hashes, nonces, package hashes, and ages only; it never returns signed action
bytes or authorization proofs. Do not provide this token to browser clients.
