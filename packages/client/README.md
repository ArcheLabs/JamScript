# @jamscript/client

TypeScript client for JamScript services, managed state and Ownership.

The package provides application-side primitives for connecting to deployed
JamScript services, querying state, submitting actions, verifying state proofs,
and using JamScript Ownership authentication.

## Install

Lifecycle tracking in this change requires Client `0.1.0-rc.5` and a Backend
that advertises the required lifecycle capabilities. RC5 is not yet published;
install it after the matching package release is available:

```bash
npm install @jamscript/client@0.1.0-rc.5
```

The package is network-neutral. Deployment and transport configuration are
supplied by the JamScript environment rather than encoded in the package name.

## Basic usage

`DeploymentDescriptor` is normally produced by the JamScript deployment flow.
The client accepts an application-provided signer for wallet-authenticated
actions.

```ts
import {
  FetchRpcTransport,
  JamScriptClient,
  type DeploymentDescriptor,
  type JamSigner,
} from "@jamscript/client";

declare const deployment: DeploymentDescriptor;
declare const signer: JamSigner;

const client = new JamScriptClient(
  deployment,
  new FetchRpcTransport("https://example.invalid/jamscript-rpc"),
);

await client.validateDeployment();

const submitted = await client.submitAction("increment", {}, signer);
const result = await client.waitForFinalized(submitted.transactionId, {
  actionHash: submitted.actionHash,
  timeoutMs: 120_000,
});
if (result.actionReceipt?.status !== "applied") {
  throw new Error("The action was finalized but not applied");
}
const state = await client.query("getValue");

console.log(result.status, state.value);
```

## Ownership actions

Ownership-authenticated actions use SignedActionV2. Applications provide an
`OwnershipSigner`; the client handles action encoding, nonce lookup,
commitment construction, and transaction submission.

```ts
import {
  FetchRpcTransport,
  JamScriptClient,
  type CodecValue,
  type DeploymentDescriptor,
  type OwnershipSigner,
} from "@jamscript/client";

declare const deployment: DeploymentDescriptor;
declare const signer: OwnershipSigner;

const client = new JamScriptClient(
  deployment,
  new FetchRpcTransport("https://example.invalid/jamscript-rpc"),
);
const input: Record<string, CodecValue> = {};

const submitted = await client.submitOwnershipAction("transfer", input, signer);
const result = await client.waitForFinalized(submitted.transactionId, {
  actionHash: submitted.actionHash,
  timeoutMs: 120_000,
});
if (result.actionReceipt?.status !== "applied") {
  throw new Error("The Ownership action was finalized but not applied");
}
```

`OwnershipSigner` is the browser-wallet extension point:

```ts
import type { JamScriptOwnershipSignRequest, Ownership } from "@jamscript/client";

interface OwnershipSigner {
  getController(): Promise<Ownership>;
  signJamScriptAction(request: JamScriptOwnershipSignRequest): Promise<Uint8Array>;
}
```

## State queries and proofs

`queryBest()` reads one reorgable best-chain snapshot and is also the default
for `query()`. `queryFinalized()` reads the finalized snapshot for accounting
and audit. `queryLatest()` remains a deprecated alias for finalized reads; it
does not switch context implicitly. Use `stateVerification: "proof"` when the
application requires proof-verified responses from the configured provider.

Ownership V2 nonce preparation reads the best-context managed state and
reserves a distinct nonce in the current client instance. This does not
coordinate separate tabs or devices. A Best inclusion or ambiguous submission
blocks the affected local nonce lane until a later finalized-state read proves
the nonce advanced. HTTP 5xx, timeouts, and connection failures are treated as
unknown submission outcomes; the client never signs a replacement action.

Transaction tracking requires a Backend that advertises
`transactionLifecycleVersion: 1`, `bestChainTracking: true`, and
`strictFinalizedReceipts: true`. Missing capabilities fail explicitly. The
Backend currently reports `durableTransactionLookup: false`; transaction IDs
and action mappings cannot be recovered after a Backend restart unless the
Backend is separately upgraded to persist them. Save the returned `actionHash`
and pass it back in `waitForFinalized(transactionId, { actionHash })` to resume
on a new Client instance while the Backend mapping still exists.

`waitForBest()` resolves only when the exact transaction is verified in a
Best snapshot or is finalized. This is an inclusion observation, not action
success. `waitForFinalized()` and `waitForAction()` resolve only when
`finalized === true` and the receipt matches the target action hash and action
index (or uniquely matches by hash when no index is supplied). A finalized
`failed` or `rejected` receipt is a finalized result but not business success.
If finality is reported before the receipt is indexed, tracking continues and
eventually times out with the last status and a `RECEIPT_UNAVAILABLE` code.

The `onUpdate` callback receives deduplicated lifecycle observations. It can
report Best inclusion followed by reorganization and later reinclusion.
`AbortSignal` stops only the local poll; it does not cancel the submitted
transaction. A timeout or abort leaves the outcome unresolved and is not a
chain failure. `watchTransaction()` is an alias for continuous tracking to a
finalized action result.

## Matrix

`MatrixOwnershipResolver` and `MatrixDeviceController` are exported as
protocol-neutral Matrix primitives. The package does not depend on
`matrix-js-sdk`; applications own their Matrix transport and device lifecycle.
The client preserves `MatrixControlClaimProofV1` codec vectors for the
M→S→D cryptographic evidence format. Controller authorization and bootstrap
actions belong to the consuming Locus service, not to the JamScript client.
## Runtime support

The package is ESM-only and is intended for modern browsers, Vite-based
applications, and current Node.js consumers. Runtime source does not import
Node built-ins, and cryptographic/network dependencies remain external npm
dependencies so the application bundler can tree-shake them.

This release candidate targets the JamScript Ownership v1 protocol,
SignedActionV1, SignedActionV2, Matrix M→S→D cross-signing proof, and the current
MiniJAM Stage-1 deployment target. MiniJAM is a supported deployment target,
not the package boundary.

## Package status

This Client version has an independent release cycle from the JamScript CLI,
Backend, and network releases. RC5 is a pending artifact in this branch; it
must not be treated as published until the npm registry contains the exact
package and its integrity matches the reviewed tarball.

Detailed protocol and integration documentation is available in the
[JamScript repository](https://github.com/ArcheLabs/JamScript/tree/main/docs).

## License

Apache-2.0. See [LICENSE](./LICENSE).
