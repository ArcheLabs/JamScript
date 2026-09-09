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

The append-only recovery format is length-delimited and includes ServiceId
and ServiceKey. Startup replay fails closed on truncation, oversized entries,
malformed output, unknown Services, or identity mismatch.

The older `tools/managed-state-network-adapter` is a legacy compatibility/test
wrapper for the generated Builder path. It is not the production topology.
