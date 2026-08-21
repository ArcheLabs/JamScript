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

For production, compose the two endpoints explicitly. Node-backed reads stay
on the MiniJAM node, while Work submission and status tracking use the
standalone formal RPC:

    const transport = new SplitRpcTransport(
      new FetchRpcTransport("https://node.example"),
      new FetchRpcTransport("https://formal.example"),
    );
    const client = new JamScriptClient(deployment, transport);

SplitRpcTransport routes chain_getBlockHash,
minijam_getFinalizedContext, and minijam_getServiceStorageAt to the node,
and routes only minijam_submitWorkV1 and minijam_getWorkStatusV1 to the
formal RPC.

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

The endpoint also exposes GET /health/ready after the chain client connects
and the bundle directory is initialized. It applies an 8 MiB request-body
limit and a bounded 32-request admission semaphore.

The client reads finalized service storage through the existing
minijam_getServiceStorageAt method. Queries decode the ABI state value
locally and return the finalized context used for the read.

## Network E2E

Run:

    ./scripts/minijam-network-e2e.sh

The test uses a pinned MiniJAM checkout, starts a real local node, formal Work
RPC and Workers, provisions a JamScript Service, submits a wallet-signed
action through the production client path, waits for finalized Work, and
verifies finalized Service state.

The test does not use the Playground Work endpoint.
