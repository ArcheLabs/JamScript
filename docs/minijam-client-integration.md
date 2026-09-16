# MiniJAM Client Integration

JamScript application semantics remain in generated services. The client
derives the JamScript ABI, creates the opaque SignedActionV1 payload, and
submits one formal transaction. The Formal service batches queued
transactions into WorkPackages.

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
on the MiniJAM node, transaction submission and status tracking use the
standalone Formal RPC, and managed-state snapshots may be served by an independent state
provider:

    const transport = new SplitRpcTransport(
      new FetchRpcTransport("https://node.example"),
      new FetchRpcTransport("https://formal.example"),
      new FetchRpcTransport("https://state.example"),
    );
    const client = new JamScriptClient(deployment, transport);

SplitRpcTransport routes chain_getBlockHash,
minijam_getFinalizedContext, and minijam_getServiceStorageAt to the node,
and routes only minijam_submitTransactionV1 and
minijam_getTransactionStatusV1 to the Formal RPC. It routes
minijam_getManagedStateV1 to the state provider.

## Formal transaction RPC

Run the standalone service next to a MiniJAM node and configure the worker
bundle gateway to use the same HTTP endpoint:

    MINIJAM_RPC_URL=ws://127.0.0.1:9944 \
    MINIJAM_RELAYER_URI='//Alice' \
    MINIJAM_FORMAL_RPC_BIND=127.0.0.1:8090 \
    MINIJAM_BUNDLE_DIR=./bundles \
    cargo run -p minijam-formal-rpc

The service exposes the generic minijam_submitTransactionV1 and
minijam_getTransactionStatusV1 JSON-RPC methods plus the verified bundle
route. `submitAction` returns `{ transactionId, actionHash }` immediately;
the package hash is assigned later when the durable queue is batched. The
status response progresses through `queued`, `packaged`, `refining`,
`reported`, `imported`, or `failed`, and includes `packageHash`, `itemIndex`,
`executionReceipt`, and `error` when available.

The transaction ID is deterministic over the Service ID, code hash, opaque
payload, and ordered external data, so an HTTP retry is idempotent. Formal
RPC does not decode `SignedActionV1`, the JamScript ABI, or business payloads.
`waitForAction(transactionId)` resolves the returned `itemIndex` against the
ordered action receipts and verifies the exact action hash recorded by the
client.

The endpoint also exposes GET /health/ready after the chain client connects
and the bundle directory is initialized. It applies an 8 MiB request-body
limit and a bounded 32-request admission semaphore.

`queryLatest` first reads the managed-state commitment from finalized Service
storage, then requests that explicit root from the provider. It checks the
response identity and verifies the Polkadot LayoutV1 storage proof locally
before decoding the ABI value. Provider availability never selects the root,
and implicit fallback to legacy Service KV is disabled by default.

## Deployment and Network E2E

Run:

    ./scripts/minijam-network-e2e.sh

The test uses a pinned MiniJAM checkout, starts a real local node, Formal RPC
and one Worker, builds a network-independent JamScript artifact, and deploys
the Service through `jams deploy --network local`. The deployment uses the
formal Stage-1 `minijam_createServiceV1` RPC and verifies the node genesis
identity before mutation. It then submits wallet-signed actions through the
production transaction path, waits for Imported transaction status, and
verifies finalized Service state.

The canonical E2E does not use the Playground lifecycle API or its legacy
`/api/v1/*` endpoints. Work, State, and deployment endpoints remain separate.
