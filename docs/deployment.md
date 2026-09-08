# JamScript Deployment v0.1

Deployment is an explicit control-plane operation after a successful,
network-independent build. JamScript v0.1 supports MiniJAM Stage-1 only;
JAM is reserved in the configuration model and fails clearly when selected.

## Named networks

`jamscript.toml` may contain named networks and an optional default:

```toml
[deployment]
default_network = "local"

[networks.local]
kind = "minijam"
deployment_rpc = "http://127.0.0.1:8090"
node_rpc = "http://127.0.0.1:9944"
genesis_hash = "0x0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"

[networks.staging]
kind = "minijam"
deployment_rpc = "https://staging-deployment.example"
node_rpc = "https://staging-node.example"
```

For `minijam`, `deployment_rpc` is required and must use `http` or `https`.
`node_rpc` is optional unless `genesis_hash` is pinned. A pinned genesis is
read with `chain_getBlockHash([0])` and compared before any create request.

Inspect the available configuration without exposing URL credentials or query
tokens:

```bash
jams network list
jams network show local
jams network list --json
```

There is no implicit global network. Resolution is explicit and follows:

```text
CLI flags > JAMSCRIPT_* environment variables > jamscript.toml
```

The supported environment overrides are `JAMSCRIPT_NETWORK`,
`JAMSCRIPT_NETWORK_KIND`, `JAMSCRIPT_DEPLOYMENT_RPC`, and
`JAMSCRIPT_NODE_RPC`. `deployment.default_network` is used only when no
network name is supplied by a higher-priority source.

## Deploy a verified artifact

Build first, then deploy the resulting directory:

```bash
jams build ./examples/counter-scriptc --output ./examples/counter-scriptc/dist
jams deploy ./examples/counter-scriptc \
  --network local \
  --artifact ./examples/counter-scriptc/dist
```

The artifact must contain `service.blob`, `build.json`, and `checksums.json`.
The CLI verifies the checksum manifest, the blob's Blake2b-256 `codeHash`, the
build metadata, and the deployment gas fields before submitting anything.
Build metadata carries `minItemGas` and `minMemoGas`; these values are sent
unchanged to the backend.

The MiniJAM request is exactly:

```text
minijam_createServiceV1({
  codeHash,
  blobBase64,
  minItemGas,
  minMemoGas,
})
```

The result must contain an operation ID, service ID, matching code hash, a
finalized flag, and finalized context. A rejected, non-finalized, or
code-hash-mismatched result fails the command. A timeout or transport failure
after a mutation may have been accepted by the network; JamScript therefore
reports `DEPLOYMENT_OUTCOME_UNKNOWN` and never blindly retries the mutation.

For custom one-off networks, provide all deployment settings on the command
line:

```bash
jams deploy ./my-service \
  --kind minijam \
  --deployment-rpc https://deployment.example \
  --node-rpc https://node.example
```

This mode has no implicit network name. The same artifact checks, URL scheme
rules, identity verification, and no-retry behavior apply.

## Deployment records

Successful deployments write a local JSON record at:

```text
.jamscript/deployments/<network>-<service-id>-<timestamp>.json
```

The format-1 record includes the selected network, observed identity and
verification state, service ID, artifact code hash/build ID/service key when
present, finalized block context, and operation ID. RPC URLs, query strings,
credentials, and tokens are never written to the record.

Use `--json` for automation. Successful stdout is one JSON document; human
diagnostics and failures go to the normal error channel. The JSON result
includes `serviceId`, `codeHash`, finalization, identity status, operation ID,
and the record path.

## Error taxonomy

Automation can branch on the stable prefix printed by the CLI:

```text
NETWORK_NOT_FOUND
NETWORK_CONFIG_INVALID
UNSUPPORTED_NETWORK_KIND
NETWORK_UNREACHABLE
NETWORK_IDENTITY_MISMATCH
ARTIFACT_NOT_FOUND
ARTIFACT_INVALID
ARTIFACT_CODE_HASH_MISMATCH
RPC_INVALID_RESPONSE
RPC_METHOD_NOT_FOUND
DEPLOYMENT_REJECTED
DEPLOYMENT_NOT_FINALIZED
DEPLOYMENT_CODE_HASH_MISMATCH
DEPLOYMENT_OUTCOME_UNKNOWN
DEPLOYMENT_RECORD_WRITE_FAILED
```

## Real MiniJAM E2E

`./scripts/minijam-network-e2e.sh` is the canonical cross-process check. It
uses a pinned sibling MiniJAM checkout and separate Work and Provider paths.
It builds before deployment, obtains the local genesis hash, appends a local
named network to the temporary project, calls `jams deploy --network local`,
and then runs the client Work/Provider verification. It does not use the
Playground lifecycle API and does not require MiniJAM or Docker for ordinary
JamScript builds.
