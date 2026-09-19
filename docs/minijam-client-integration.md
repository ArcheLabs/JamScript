# MiniJAM Client Integration

JamScript application semantics remain in generated services. The client
derives the JamScript ABI, creates the opaque SignedActionV1 payload, and
submits one logical transaction to the JamScript Backend. The backend owns
nonce scheduling, batching, and the mapping from the logical transaction to
the resulting MiniJAM Work.

## Client package

    cd packages/client
    npm install --offline
    npm run build
    npm test

The browser wallet adapter calls signRaw exactly once with the
SignedActionV1 signing digest. The digest contains the
JAMSCRIPT_ACTION_V1 domain, while sr25519 verification uses the standard
Substrate context.

For production, send a transaction-capable `JamScriptClient` to the JamScript
Backend endpoint. The backend is the only endpoint that accepts
`jamscript_submitTransactionV1` and `jamscript_getTransactionStatusV1`.
If the application also performs low-level chain or Work reads, compose those
endpoints explicitly. Finalized chain reads stay on the MiniJAM node, physical
Work submission and status tracking use the standalone Formal RPC, and
managed-state snapshots may be served by an independent state provider:

    const transport = new SplitRpcTransport(
      new FetchRpcTransport("https://node.example"),
      new FetchRpcTransport("https://formal.example"),
      new FetchRpcTransport("https://state.example"),
    );
    const client = new JamScriptClient(deployment, transport);

SplitRpcTransport routes chain_getBlockHash,
minijam_getFinalizedContext, and minijam_getServiceStorageAt to the node,
and routes only minijam_submitWorkV1 and minijam_getWorkStatusV1 to the
formal RPC. It does not route logical transaction methods; those must use the
Backend endpoint. It routes jamscript_getStateV1,
jamscript_getStateProofV1, and minijam_getManagedStateV1 to the state
provider.

## Formal Work RPC

Run the standalone service next to a MiniJAM node and configure the worker
bundle gateway to use the same HTTP endpoint:

    MINIJAM_RPC_URL=ws://127.0.0.1:9944 \
    MINIJAM_RELAYER_URI='//Alice' \
    MINIJAM_FORMAL_RPC_BIND=127.0.0.1:8080 \
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

The test pulls the exact digest-pinned aggregate MiniJAM image from
`toolchains/minijam.lock` and starts it with `--dev`. The image owns the
canonical local node, Formal RPC on `8080`, and exactly one Worker on `8082`;
JamScript does not start a custom MiniJAM Compose topology or checkout MiniJAM
source. It builds a network-independent JamScript artifact, deploys the
Service through `jams deploy --network local`, submits wallet-signed actions,
waits for finalized Work, verifies batched receipts and finalized state, and
restarts the backend before querying durable state again.

The canonical E2E does not use the Playground lifecycle API or its legacy
`/api/v1/*` endpoints. Logical transactions enter through the Backend;
physical Work, State, and deployment endpoints remain separate.
