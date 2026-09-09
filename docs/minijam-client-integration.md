# MiniJAM Client Integration

JamScript application semantics remain in generated services. The client
derives the JamScript ABI, creates the opaque SignedActionV1 payload, and
submits one formal Work request.

## Client package

    cd packages/client
    npm install --offline
    npm run build
    npm test

The browser wallet adapter calls signRaw exactly once with the
SignedActionV1 signing digest. The digest contains the
JAMSCRIPT_ACTION_V1 domain, while sr25519 verification uses the standard
Substrate context.

For production, the browser uses one application-facing backend endpoint:

    const client = new JamScriptClient({
      endpoint: "https://backend.testnet.example",
      deployment,
      signer,
    });

The backend may route finalized context and Service storage reads to a MiniJAM
node, Work submission/status to Formal RPC, and managed-state proofs to its
materialized provider. Those internal connections are not part of frontend
configuration. `SplitRpcTransport` remains available as a compatibility
adapter for local deployments and older infrastructure.

## Formal Work RPC

The backend runs next to a MiniJAM node and Formal RPC. Its public endpoint
owns application work, Service registration, state proofs, and recovery; the
Formal service remains a network/control-plane dependency. Deployments wire
`BackendDaemon` with a network-specific `BackendWorkGateway` and bind it to a
single public listener; Node and Formal URLs remain backend-only settings.

The backend exposes the generic `minijam_submitWorkV1` and
`minijam_getWorkStatusV1` methods together with
`minijam_getManagedStateV1`, `jamscript_getCapabilitiesV1`, and the restricted
`jamscript_registerServiceV1` control-plane method. It routes every request by
`serviceId`; state proofs and recovery are always Service-scoped.

The endpoint also exposes GET `/healthz` and `/readinessz` after the artifact
store is initialized and the configured node is reachable. It applies bounded
headers and request bodies and a bounded connection admission limit.

`queryLatest` first reads the managed-state commitment from finalized Service
storage, then requests that explicit root from the provider. It checks the
response identity and verifies the Polkadot LayoutV1 storage proof locally
before decoding the ABI value. Provider availability never selects the root,
and implicit fallback to legacy Service KV is disabled by default.

## Deployment and Network E2E

Run:

    ./scripts/minijam-network-e2e.sh

The test uses a pinned MiniJAM checkout, starts a real local node, formal Work
RPC and Workers, builds a network-independent JamScript artifact, and deploys
the Service through `jams deploy --network local`. The deployment uses the
formal Stage-1 `minijam_createServiceV1` RPC and verifies the node genesis
identity before mutation. It then submits a wallet-signed action through the
production client path, waits for finalized Work, and verifies finalized
Service state.

The canonical E2E does not use the Playground lifecycle API or its legacy
`/api/v1/*` endpoints. Work, State, and deployment remain separate logical
operations behind the one backend endpoint.
