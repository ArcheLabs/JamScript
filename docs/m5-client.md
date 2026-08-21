# M5 production client path

M5 keeps application semantics in generated JamScript services. The client
only derives the M4 ABI, creates the opaque SignedActionV1 payload, and
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

The client reads finalized service storage through the existing
minijam_getServiceStorageAt method. Queries decode the M4 state value locally
and return the finalized context used for the read.
