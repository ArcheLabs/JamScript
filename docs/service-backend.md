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

Registration validates the Service identity before binding its digest-addressed
planner artifact. A `ServiceRegistrationValidator` implementation connects
that check to canonical node metadata. `ApplicationArtifactLoader` is the
portable loading boundary: a WASM/PVM/ScriptC loader can be supplied without
compiling the backend daemon for a new Service.

Deployment can provision the backend after on-chain creation by configuring
`backend_rpc` in the selected network (or `JAMSCRIPT_BACKEND_RPC`) and setting
`JAMSCRIPT_BACKEND_ADMIN_TOKEN`. `jams deploy` records the chain deployment
before attempting this separate registration call, so registration can be
retried without recreating the Service.

The backend builds proof witnesses only at the root reported by canonical
Service storage. Refine receives external witnesses as read-only views and
emits JAM `serviceId`/root dependencies. Accumulate performs the final
ServiceId-scoped commitment checks. Local state is materialized only after the
predicted root is observed canonically.

The append-only recovery format is length-delimited and includes ServiceId
and ServiceKey. Startup replay fails closed on truncation, oversized entries,
malformed output, unknown Services, or identity mismatch.

The older `tools/managed-state-network-adapter` remains a compatibility/test
wrapper for the current MiniJAM generated Builder path. It should not be used
as the public topology for dynamic multi-Service deployments.
