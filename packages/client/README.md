# @jamscript/client

TypeScript client for JamScript services, managed state and Ownership.

The package provides application-side primitives for connecting to deployed
JamScript services, querying state, submitting actions, verifying state proofs,
and using JamScript Ownership authentication.

## Install

The first public release is planned as an RC:

```bash
npm install @jamscript/client@0.1.0-rc.3
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
const result = await client.waitForAction(submitted.transactionId, {
  timeoutMs: 120_000,
});
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
await client.waitForAction(submitted.transactionId, { timeoutMs: 120_000 });
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

`JamScriptClient` supports finalized state queries and managed-state proof
verification. Use `stateVerification: "proof"` when the application requires
proof-verified responses from the configured provider.

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
SignedActionV1, SignedActionV2, Matrix bootstrap proof, and the current
MiniJAM Stage-1 deployment target. MiniJAM is a supported deployment target,
not the package boundary.

## Package status

This is the first public release candidate. The client has an independent
version cycle from the JamScript CLI, backend, and network releases.

Detailed protocol and integration documentation is available in the
[JamScript repository](https://github.com/ArcheLabs/JamScript/tree/main/docs).

## License

Apache-2.0. See [LICENSE](./LICENSE).
