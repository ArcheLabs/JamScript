# Ownership Control system service

JamScript's ControlClaim namespace is owned by the first-party Ownership
Control system service. Consumer services do not copy claims into their own
managed state. A delegated consumer action (`actAs`) reads the active claim
from the network-scoped external service through the proof-backed external
state interface and fails closed when the network has no configured control
service.

The service has a stable logical key, while its numeric service ID, genesis,
network domain and code hash are deployment-specific. Build the canonical
artifact with:

```bash
cargo run -p ownership-control-service-build -- target/ownership-control-service
```

The command writes the reproducible `service.elf`, `service.blob`,
`service.pvm`, and `build.json` provenance files. Register that deployment in
the backend before enabling delegated consumer actions. The backend accepts
the complete network-scoped descriptor through:

```text
JAMSCRIPT_OWNERSHIP_CONTROL_SERVICE_ID
JAMSCRIPT_OWNERSHIP_CONTROL_GENESIS_HASH
JAMSCRIPT_OWNERSHIP_CONTROL_NETWORK_DOMAIN
JAMSCRIPT_OWNERSHIP_CONTROL_SERVICE_KEY
JAMSCRIPT_OWNERSHIP_CONTROL_CODE_HASH
```

All five values are required together. Startup rejects a descriptor whose
genesis, network domain, service key, or code hash does not match the active
network or registered service. No consumer or browser code may hardcode a
numeric control service ID.
