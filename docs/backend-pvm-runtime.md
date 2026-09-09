# Production backend PVM runtime

The production backend is one process for arbitrary Services. It loads linked
`service.pvm` bytes from a persistent content-addressed store and executes the
PVM interpreter already used by the JamScript target. A registration is bound
to `(ServiceId, canonical on-chain codeHash)`; the loader converts the linked
PVM to its canonical JAM ProgramBlob form and rejects a mismatch.

Every artifact exposes two bounded entries in addition to the frozen v1
`minijam_refine` and `minijam_accumulate` entries:

- `jamscript_backend_metadata_v1` returns version, ServiceKey, ABI, planner,
  and managed-state versions in a strict binary record.
- `jamscript_plan_v1` runs against a planning state and returns a bounded
  NeedState envelope. The backend canonicalizes discovered keys, builds the
  corresponding LayoutV1 proof, and preflights the exact RuntimeRefineInput
  before submission.

The backend accepts `jamscript_putArtifactV1` only with
`format = "jamscript-pvm-v1"`, a Blake2 content digest, and base64 bytes. Files
are written atomically under `JAMSCRIPT_BACKEND_DATA/artifacts`; existing
bytes are verified on idempotent upload and every read rechecks the digest.
No filesystem path or generated Rust source is part of the registration
protocol.

## Running

```bash
JAMSCRIPT_BACKEND_BIND=0.0.0.0:8090 \
JAMSCRIPT_NODE_RPC=http://node:9944 \
JAMSCRIPT_FORMAL_RPC=http://formal:8090 \
JAMSCRIPT_BACKEND_DATA=/var/lib/jamscript \
cargo run --release -p jamscript-service-backend --bin jamscript-service-backend
```

The public endpoint includes `/healthz` and `/readinessz`. Node and Formal RPC
URLs remain backend-only. `jams deploy` first tries permissionless discovery;
if the node exposes only the canonical preimage, it uploads the linked PVM as
the content-addressed fallback and retries registration. An admin token is
not required for the normal deployment path.

Registry records, artifacts, and the length-delimited recovery log survive a
restart. Startup replays recovery entries and leaves a Service unbound when a
persisted artifact is missing or corrupt, so repair is explicit rather than a
silent fallback to a local source path.
