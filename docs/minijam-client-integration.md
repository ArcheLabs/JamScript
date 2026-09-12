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

For production, compose the endpoints explicitly. Finalized chain reads stay
on the MiniJAM node, Work submission and status tracking use the standalone
formal RPC, and managed-state snapshots may be served by an independent state
provider:

    const transport = new SplitRpcTransport(
      new FetchRpcTransport("https://node.example"),
      new FetchRpcTransport("https://formal.example"),
      new FetchRpcTransport("https://state.example"),
    );
    const client = new JamScriptClient(deployment, transport);

SplitRpcTransport routes chain_getBlockHash,
minijam_getFinalizedContext, and minijam_getServiceStorageAt to the node,
and routes only minijam_submitWorkV1 and minijam_getWorkStatusV1 to the
formal RPC. It routes jamscript_getStateV1,
jamscript_getStateProofV1, and minijam_getManagedStateV1 to the state
provider.

## Formal Work RPC

Run the standalone service next to a MiniJAM node and configure the worker
bundle gateway to use the same HTTP endpoint:

    MINIJAM_RPC_URL=ws://127.0.0.1:9944 \
    MINIJAM_RELAYER_URI='//Alice' \
    MINIJAM_FORMAL_RPC_BIND=127.0.0.1:8090 \
    MINIJAM_BUNDLE_DIR=./bundles \
    cargo run -p minijam-formal-rpc

The service exposes only the generic minijam_submitWorkV1 and
minijam_getWorkStatusV1 JSON-RPC methods plus the verified bundle route.
The submit method checks the supplied finalized context and finalized service
code hash, builds one WorkItem, stores its bundle, then submits the opaque
package through the configured ingress relayer. It does not decode JamScript
payloads or provide application query execution.

The endpoint also exposes GET `/healthz` and `/readinessz` after the artifact
store is initialized and the configured node is reachable. It applies bounded
headers and request bodies and a bounded connection admission limit.

`queryLatest` first reads the managed-state commitment from finalized Service
storage and asks the backend for the value through `jamscript_getStateV1` by
default. The backend rejects a stale or unavailable materialization with
`STATE_NOT_MATERIALIZED`; no proof object is exposed to application code in
trusted mode. Set `stateVerification: "proof"` to request
`jamscript_getStateProofV1` semantics and verify the LayoutV1 storage proof
locally. Provider availability never selects the root, and implicit fallback to
legacy Service KV is disabled by default.

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
`/api/v1/*` endpoints. Work, State, and deployment endpoints remain separate.
