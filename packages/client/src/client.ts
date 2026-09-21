import { actionByName, queryByName, stateByName, type DeploymentDescriptor } from "./abi.js";
import { decodeStateValue, decodeValue, encodeActionPayload, encodeValue, type CodecValue } from "./codec.js";
import {
  actionSelector,
  controlClaimBootstrapKey,
  controlClaimKey,
  encodeSignedActionV1,
  encodeSignedActionV2,
  MANAGED_STATE_COMMITMENT_KEY_V1,
  nonceKey,
  ownershipNonceKey,
  parseHex,
  signingDigestV1,
  signingMessageV2,
  stateKey,
  toHex,
  type SignedActionV1,
  type Ownership,
  type SignedActionV2,
} from "./crypto.js";
import {
  CONTROL_CLAIM_ACTIONS,
  encodeControlClaimActionV1,
  type ControlClaimDeployment,
  type MatrixControlBootstrapInput,
  type MatrixControlBootstrapSigner,
} from "./control-claim.js";
import {
  encodeMatrixControlBootstrapV1,
  encodeMatrixControlClaimProofV1,
  matrixControlBootstrapSigningMessage,
  type MatrixControlBootstrapV1,
  type MatrixControlClaimProofV1,
} from "./matrix.js";
import { asWorkRpc, RpcError, type ActionReceipt, type FinalizedContext, type RpcTransport, type SubmitActionResult, type SubmitTransactionResult, type TransactionStatusResult, type WorkRpc, type WorkStatusResult } from "./rpc.js";
import type { JamSigner, OwnershipSigner } from "./signer.js";
import { blake2AsU8a } from "@polkadot/util-crypto";
import { verifyManagedStateProof } from "./proof.js";
import {
  ProofStateProvider,
  TrustedStateProvider,
  type StateProvider,
} from "./state-provider.js";

const EMPTY_STATE_ROOT_V1 = "0x03170a2e7597b7b7e3d84c05391d139a62b157e78786d8c082f29dcf4c111314";

export type QueryResult = {
  value: CodecValue | null;
  context: FinalizedContext;
  stateRoot: string;
};

export type JamScriptClientOptions = {
  stateProvider?: StateProvider;
  stateVerification?: "trusted-backend" | "proof";
};

export type WaitForActionResult = Omit<TransactionStatusResult, "status"> & {
  status: ActionReceipt["status"];
  transactionStatus: TransactionStatusResult["status"];
  actionHash: string;
  errorCode: number | null;
  actionReceipt: ActionReceipt;
};

export class JamScriptClient {
  private readonly rpc: WorkRpc;
  private readonly actionHashes = new Map<string, string>();
  private readonly nextNonces = new Map<string, bigint>();
  private readonly nonceTails = new Map<string, Promise<void>>();
  private readonly ownershipTails = new Map<string, Promise<void>>();

  constructor(
    private readonly deployment: DeploymentDescriptor,
    transport: RpcTransport,
    private readonly options: JamScriptClientOptions = {},
  ) {
    if (deployment.abiVersion !== 1 || deployment.abi.abiVersion !== 1) {
      throw new Error("unsupported JamScript ABI version");
    }
    this.rpc = asWorkRpc(transport);
    this.stateProvider = options.stateProvider
      ?? (options.stateVerification === "proof"
        ? new ProofStateProvider(transport)
        : new TrustedStateProvider(transport));
    this.verifyProofs = options.stateProvider !== undefined || options.stateVerification === "proof";
  }

  private readonly stateProvider: StateProvider;
  private readonly verifyProofs: boolean;

  async validateDeployment(): Promise<void> {
    const genesis = await this.rpc.genesisHash();
    if (!sameHex(genesis, this.deployment.genesisHash)) {
      throw new Error("deployment genesis hash does not match the chain");
    }
  }

  async readNonce(publicKey: Uint8Array, context?: FinalizedContext): Promise<bigint> {
    if (publicKey.length !== 32) throw new Error("sr25519 public key must be 32 bytes");
    const finalized = context ?? (await this.rpc.finalizedContext());
    const root = await this.managedStateRoot(finalized);
    const key = nonceKey(publicKey);
    const valueBytes = await this.readManagedValue(root, key);
    if (valueBytes === null) return 0n;
    const value = decodeValue("u64", valueBytes);
    if (typeof value !== "bigint") throw new Error("nonce storage is not u64");
    return value;
  }

  async submitAction(
    actionName: string,
    input: Record<string, CodecValue>,
    signer: JamSigner,
    options: { ttl?: bigint; extrinsics?: Uint8Array[]; staleRetries?: number } = {},
  ): Promise<SubmitActionResult> {
    if (signer.publicKey.length !== 32) throw new Error("sr25519 public key must be 32 bytes");
    const signerKey = toHex(signer.publicKey).toLowerCase();
    const prepared = await this.prepareAction(
      actionName,
      input,
      signer,
      options,
    );
    let submitted: SubmitTransactionResult;
    try {
      submitted = await this.rpc.submitTransaction(prepared.request);
    } catch (error) {
      // Only rewind a reservation when no later concurrent action has already
      // reserved a nonce.  This keeps the common single-admission-failure
      // recovery while avoiding duplicate nonces for an active batch.
      if (this.nextNonces.get(signerKey) === prepared.nonce + 1n) {
        this.nextNonces.delete(signerKey);
      }
      throw error;
    }
    this.actionHashes.set(submitted.transactionId.toLowerCase(), prepared.actionHash);
    return { ...submitted, actionHash: prepared.actionHash };
  }

  async submitOwnershipAction(
    actionName: string,
    input: Record<string, CodecValue>,
    signer: OwnershipSigner,
    options: { actAs?: Ownership; ttl?: bigint; extrinsics?: Uint8Array[] } = {},
  ): Promise<SubmitActionResult> {
    const controller = await signer.getController();
    const signerLane = toHex(controller.public).toLowerCase();
    const previous = this.ownershipTails.get(signerLane) ?? Promise.resolve();
    let release!: () => void;
    const lane = new Promise<void>((resolve) => { release = resolve; });
    this.ownershipTails.set(signerLane, lane);
    await previous;
    try {
      await this.validateDeployment();
      const action = actionByName(this.deployment.abi, actionName);
      if (!isOwnershipAuth(action.auth)) throw new Error("submitOwnershipAction requires an ownership-authenticated action");
      const payload = encodeActionPayload(this.deployment.abi, actionName, input);
      const selector = actionSelector(actionName);
      if (!sameHex(toHex(selector), action.selector)) throw new Error("deployment ABI selector does not match the canonical selector");
      const context = await this.rpc.finalizedContext();
      const effectiveOwner = options.actAs ?? controller;
      const root = await this.managedStateRoot(context);
      const nonceBytes = await this.readManagedValue(root, ownershipNonceKey(effectiveOwner));
      const chainNonce = nonceBytes === null ? 0n : decodeValue("u64", nonceBytes);
      if (typeof chainNonce !== "bigint") throw new Error("ownership nonce storage is not u64");
      const unsigned: Omit<SignedActionV2, "authorizationProof"> = {
        version: 2,
        networkDomain: parseHex(this.deployment.networkDomain, 32),
        serviceKey: parseHex(this.deployment.serviceKey, 32),
        actionSelector: selector,
        controller,
        actAs: options.actAs ?? null,
        nonce: chainNonce,
        validUntil: BigInt(context.slot) + (options.ttl ?? 64n),
        payloadHash: blake2(payload),
        payload,
      };
      const message = signingMessageV2(unsigned);
      const authorizationProof = await signer.signJamScriptAction({ ...unsigned, message });
      if (authorizationProof.length === 0 || authorizationProof.length > 65536) throw new Error("invalid Ownership authorization proof");
      const signed = encodeSignedActionV2({ ...unsigned, authorizationProof });
      const actionHash = toHex(blake2(signed));
      const submitted = await this.rpc.submitTransaction({
        serviceId: this.deployment.serviceId,
        serviceCodeHash: this.deployment.codeHash,
        payloadBase64: toBase64(signed),
        extrinsicsBase64: (options.extrinsics ?? []).map(toBase64),
      });
      this.actionHashes.set(submitted.transactionId.toLowerCase(), actionHash);
      return { ...submitted, actionHash };
    } finally {
      release();
      if (this.ownershipTails.get(signerLane) === lane) this.ownershipTails.delete(signerLane);
    }
  }

  async bootstrapMatrixControlClaim(
    input: MatrixControlBootstrapInput & { deployment: ControlClaimDeployment },
  ): Promise<SubmitActionResult> {
    const controllerSigner: MatrixControlBootstrapSigner = input.controllerSigner;
    const controller = await controllerSigner.getController();
    const matrixProof = input.proof instanceof Uint8Array
      ? input.proof.slice()
      : encodeMatrixControlClaimProofV1(input.proof);
    const draft: MatrixControlBootstrapV1 = {
      networkDomain: parseHex(input.deployment.networkDomain, 32),
      subject: input.subject,
      controller,
      matrixProof,
      controllerProof: new Uint8Array(),
    };
    const controllerProof = await controllerSigner.signBootstrapMessage(
      matrixControlBootstrapSigningMessage(draft),
    );
    if (controllerProof.length !== 64) throw new Error("invalid Matrix controller possession proof");
    const payload = encodeMatrixControlBootstrapV1({ ...draft, controllerProof });
    return this.submitControlClaimAction(
      input.deployment,
      CONTROL_CLAIM_ACTIONS.bootstrapMatrixController,
      payload,
      controllerSigner,
    );
  }

  async addController(input: {
    deployment: ControlClaimDeployment;
    subject: Ownership;
    controller: Ownership;
    signer: OwnershipSigner;
  }): Promise<SubmitActionResult> {
    return this.submitControlClaimAction(
      input.deployment,
      CONTROL_CLAIM_ACTIONS.addController,
      encodeControlClaimActionV1("add", input.subject, input.controller),
      input.signer,
      { actAs: input.subject },
    );
  }

  async revokeController(input: {
    deployment: ControlClaimDeployment;
    subject: Ownership;
    controller: Ownership;
    signer: OwnershipSigner;
  }): Promise<SubmitActionResult> {
    return this.submitControlClaimAction(
      input.deployment,
      CONTROL_CLAIM_ACTIONS.revokeController,
      encodeControlClaimActionV1("revoke", input.subject, input.controller),
      input.signer,
      { actAs: input.subject },
    );
  }

  async isControllerActive(
    deployment: ControlClaimDeployment,
    subject: Ownership,
    controller: Ownership,
  ): Promise<boolean> {
    await this.validateControlClaimDeployment(deployment);
    const context = await this.rpc.finalizedContext();
    const root = await this.managedStateRootFor(context, deployment);
    const value = await this.readManagedValueFor(root, deployment, controlClaimKey(subject, controller));
    return value?.length === 1 && value[0] === 1;
  }

  async hasBootstrapCompleted(
    deployment: ControlClaimDeployment,
    subject: Ownership,
  ): Promise<boolean> {
    await this.validateControlClaimDeployment(deployment);
    const context = await this.rpc.finalizedContext();
    const root = await this.managedStateRootFor(context, deployment);
    const value = await this.readManagedValueFor(root, deployment, controlClaimBootstrapKey(subject));
    return value?.length === 1 && value[0] === 1;
  }

  private async submitControlClaimAction(
    deployment: ControlClaimDeployment,
    actionName: string,
    payload: Uint8Array,
    signer: OwnershipSigner,
    options: { actAs?: Ownership; ttl?: bigint } = {},
  ): Promise<SubmitActionResult> {
    const controller = await signer.getController();
    const signerLane = `${deployment.serviceId}:${toHex(controller.public).toLowerCase()}`;
    const previous = this.ownershipTails.get(signerLane) ?? Promise.resolve();
    let release!: () => void;
    const lane = new Promise<void>((resolve) => { release = resolve; });
    this.ownershipTails.set(signerLane, lane);
    await previous;
    try {
      await this.validateControlClaimDeployment(deployment);
      const context = await this.rpc.finalizedContext();
      const effectiveOwner = options.actAs ?? controller;
      const root = await this.managedStateRootFor(context, deployment);
      const nonceBytes = await this.readManagedValueFor(root, deployment, ownershipNonceKey(effectiveOwner));
      const chainNonce = nonceBytes === null ? 0n : decodeValue("u64", nonceBytes);
      if (typeof chainNonce !== "bigint") throw new Error("ControlClaim nonce storage is not u64");
      const unsigned: Omit<SignedActionV2, "authorizationProof"> = {
        version: 2,
        networkDomain: parseHex(deployment.networkDomain, 32),
        serviceKey: parseHex(deployment.serviceKey, 32),
        actionSelector: actionSelector(actionName),
        controller,
        actAs: options.actAs ?? null,
        nonce: chainNonce,
        validUntil: BigInt(context.slot) + (options.ttl ?? 64n),
        payloadHash: blake2(payload),
        payload,
      };
      const authorizationProof = await signer.signJamScriptAction({ ...unsigned, message: signingMessageV2(unsigned) });
      if (authorizationProof.length === 0 || authorizationProof.length > 65536) throw new Error("invalid Ownership authorization proof");
      const signed = encodeSignedActionV2({ ...unsigned, authorizationProof });
      const actionHash = toHex(blake2(signed));
      const submitted = await this.rpc.submitTransaction({
        serviceId: deployment.serviceId,
        serviceCodeHash: deployment.codeHash,
        payloadBase64: toBase64(signed),
        extrinsicsBase64: [],
      });
      this.actionHashes.set(submitted.transactionId.toLowerCase(), actionHash);
      return { ...submitted, actionHash };
    } finally {
      release();
      if (this.ownershipTails.get(signerLane) === lane) this.ownershipTails.delete(signerLane);
    }
  }

  private async validateControlClaimDeployment(deployment: ControlClaimDeployment): Promise<void> {
    const genesis = await this.rpc.genesisHash();
    if (!sameHex(genesis, deployment.genesisHash)) throw new Error("ControlClaim deployment genesis does not match the chain");
    if (!sameHex(deployment.networkDomain, this.deployment.networkDomain)) throw new Error("ControlClaim deployment network domain does not match the consumer deployment");
  }

  private async reserveWalletNonce(
    publicKey: Uint8Array,
  ): Promise<{ context: FinalizedContext; nonce: bigint }> {
    const signerKey = toHex(publicKey).toLowerCase();
    const previous = this.nonceTails.get(signerKey) ?? Promise.resolve();
    let release!: () => void;
    const lane = new Promise<void>((resolve) => { release = resolve; });
    this.nonceTails.set(signerKey, lane);
    await previous;
    try {
      const context = await this.rpc.finalizedContext();
      const localNonce = this.nextNonces.get(signerKey);
      const chainNonce = localNonce === undefined
        ? await this.readNonce(publicKey, context)
        : localNonce;
      const nonce = localNonce === undefined || localNonce < chainNonce ? chainNonce : localNonce;
      this.nextNonces.set(signerKey, nonce + 1n);
      return { context, nonce };
    } finally {
      release();
      if (this.nonceTails.get(signerKey) === lane) this.nonceTails.delete(signerKey);
    }
  }

  private async prepareAction(
    actionName: string,
    input: Record<string, CodecValue>,
    signer: JamSigner,
    options: { ttl?: bigint; extrinsics?: Uint8Array[]; staleRetries?: number },
  ): Promise<{ request: Parameters<WorkRpc["submitTransaction"]>[0]; actionHash: string; nonce: bigint }> {
    await this.validateDeployment();
    const action = actionByName(this.deployment.abi, actionName);
    if (action.auth !== "wallet") throw new Error("submitAction requires a wallet-authenticated action");
    if (signer.publicKey.length !== 32) throw new Error("sr25519 public key must be 32 bytes");
    const payload = encodeActionPayload(this.deployment.abi, actionName, input);
    const selector = actionSelector(actionName);
    if (!sameHex(toHex(selector), action.selector)) {
      throw new Error("deployment ABI selector does not match the canonical selector");
    }
    const { context: initialContext, nonce } = await this.reserveWalletNonce(signer.publicKey);
    const ttl = options.ttl ?? 64n;
    const validUntil = BigInt(initialContext.slot) + ttl;
    const unsigned: Omit<SignedActionV1, "signature"> = {
      version: 1,
        networkDomain: parseHex(this.deployment.networkDomain, 32),
      serviceKey: parseHex(this.deployment.serviceKey, 32),
      actionSelector: selector,
      signerScheme: 0,
      publicKey: signer.publicKey,
      nonce,
      validUntil,
      payloadHash: blake2(payload),
      payload,
    };
    const signature = await signer.signRaw(signingDigestV1(unsigned));
    if (signature.length !== 64) throw new Error("sr25519 signRaw must return a 64-byte signature");
    const signed = encodeSignedActionV1({ ...unsigned, signature });
    const actionHash = toHex(blake2(signed));
    const requestBase = {
      serviceId: this.deployment.serviceId,
      serviceCodeHash: this.deployment.codeHash,
      payloadBase64: toBase64(signed),
      extrinsicsBase64: (options.extrinsics ?? []).map(toBase64),
    };

    return { request: requestBase, actionHash, nonce };
  }

  async queryLatest(queryName: string, key?: CodecValue): Promise<QueryResult> {
    const query = queryByName(this.deployment.abi, queryName);
    const state = stateByName(this.deployment.abi, query.state);
    const keyBytes = isUnitType(state.keyType)
      ? new Uint8Array()
      : encodeValue(state.keyType, key === undefined ? null : key);
    if (JSON.stringify(query.keyType) !== JSON.stringify(state.keyType)) throw new Error("query key type does not match state key type");
    const context = await this.rpc.finalizedContext();
    const root = await this.managedStateRoot(context);
    const valueBytes = await this.readManagedValue(
      root,
      stateKey(state.schema, keyBytes),
    );
    return {
      value: valueBytes === null
        ? null
        : decodeValue(query.output.type, valueBytes),
      context,
      stateRoot: toHex(root),
    };
  }

  async query(queryName: string, key?: CodecValue): Promise<QueryResult> {
    return this.queryLatest(queryName, key);
  }

  private async managedStateRoot(context: FinalizedContext): Promise<Uint8Array> {
    return this.managedStateRootFor(context, this.deployment);
  }

  private async managedStateRootFor(
    context: FinalizedContext,
    deployment: Pick<ControlClaimDeployment, "serviceId">,
  ): Promise<Uint8Array> {
    const encoded = await this.rpc.serviceStorageAt(context.blockHash, deployment.serviceId, toHex(MANAGED_STATE_COMMITMENT_KEY_V1));
    if (encoded === null) return parseHex(EMPTY_STATE_ROOT_V1);
    const commitment = decodeStateValue(parseHex(encoded));
    if (commitment.length !== 34 || commitment[0] !== 1 || commitment[1] !== 1) {
      throw new Error("invalid ManagedStateCommitmentV1");
    }
    return commitment.slice(2);
  }

  private async readManagedValue(
    root: Uint8Array,
    key: Uint8Array,
  ): Promise<Uint8Array | null> {
    return this.readManagedValueFor(root, this.deployment, key);
  }

  private async readManagedValueFor(
    root: Uint8Array,
    deployment: Pick<ControlClaimDeployment, "serviceId" | "serviceKey">,
    key: Uint8Array,
  ): Promise<Uint8Array | null> {
    const response = await this.stateProvider.get({ serviceId: deployment.serviceId, serviceKey: deployment.serviceKey, stateRoot: toHex(root), key });
    if (
      response.serviceId !== this.deployment.serviceId
      || response.stateRoot.toLowerCase() !== toHex(root).toLowerCase()
      || !sameBytes(response.key, key)
    ) {
      throw new Error("managed-state provider response does not match the requested query");
    }
    if (this.verifyProofs) return verifyManagedStateProof(root, key, response.value, response.proof);
    return response.value;
  }

  workStatus(packageHash: string): Promise<WorkStatusResult> {
    return this.rpc.workStatus(packageHash, this.deployment.serviceId);
  }

  transactionStatus(transactionId: string): Promise<TransactionStatusResult> {
    return this.rpc.transactionStatus(transactionId);
  }

  async waitForTransaction(
    transactionId: string,
    options: { intervalMs?: number; timeoutMs?: number } = {},
  ): Promise<TransactionStatusResult> {
    const intervalMs = options.intervalMs ?? 1_000;
    const deadline = Date.now() + (options.timeoutMs ?? 120_000);
    for (;;) {
      try {
        const status = await this.transactionStatus(transactionId);
        if (status.status === "imported" || status.status === "failed") return status;
      } catch (error) {
        if (!(error instanceof RpcError) || error.code !== -32013) throw error;
      }
      if (Date.now() >= deadline) throw new Error("timed out waiting for transaction");
      await new Promise((resolve) => setTimeout(resolve, intervalMs));
    }
  }

  async waitForWork(
    packageHash: string,
    options: { intervalMs?: number; timeoutMs?: number } = {},
  ): Promise<WorkStatusResult> {
    const intervalMs = options.intervalMs ?? 1_000;
    const deadline = Date.now() + (options.timeoutMs ?? 120_000);
    for (;;) {
      try {
        const status = await this.workStatus(packageHash);
        if (status.status === "imported" || status.status === "failed") return status;
      } catch (error) {
        if (
          !(error instanceof RpcError)
          || (error.code !== -32013 && error.message !== "work not found")
        ) throw error;
      }
      if (Date.now() >= deadline) throw new Error("timed out waiting for finalized Work");
      await new Promise((resolve) => setTimeout(resolve, intervalMs));
    }
  }

  async waitForAction(
    transactionId: string,
    optionsOrActionHash: { intervalMs?: number; timeoutMs?: number } | string = {},
    legacyOptions: { intervalMs?: number; timeoutMs?: number } = {},
  ): Promise<WaitForActionResult> {
    const options = typeof optionsOrActionHash === "string" ? legacyOptions : optionsOrActionHash;
    const transaction = await this.waitForTransaction(transactionId, options);
    if (transaction.status === "failed") {
      throw new RpcError("transaction failed before an action receipt was produced", -32040, transaction);
    }
    const expected = this.actionHashes.get(transactionId.toLowerCase())
      ?? (typeof optionsOrActionHash === "string" ? optionsOrActionHash : undefined);
    if (!expected) throw new Error("action hash is unavailable for this client instance");
    const actionIndex = transaction.actionIndex;
    const receipt = actionIndex === null ? undefined : transaction.actionReceipts?.[actionIndex];
    if (!receipt) {
      throw new Error("canonical action receipt is missing for the transaction action index");
    }
    if (!sameHex(receipt.actionHash, expected)) {
      throw new Error("transaction action index resolved to a different action hash");
    }
    return {
      ...transaction,
      status: receipt.status,
      transactionStatus: transaction.status,
      actionHash: receipt.actionHash,
      errorCode: receipt.errorCode,
      actionReceipt: receipt,
    };
  }
}

function isUnitType(type: string | { kind: string }): boolean {
  return typeof type === "string" ? type === "unit" : type.kind === "unit";
}

function isOwnershipAuth(auth: string | { kind: "ownership"; version: 1 }): auth is { kind: "ownership"; version: 1 } {
  return typeof auth === "object" && auth.kind === "ownership" && auth.version === 1;
}

function blake2(bytes: Uint8Array): Uint8Array {
  return blake2AsU8a(bytes, 256);
}

function toBase64(bytes: Uint8Array): string {
  let binary = "";
  for (const byte of bytes) binary += String.fromCharCode(byte);
  return btoa(binary);
}

function fromBase64(value: string): Uint8Array {
  const binary = atob(value);
  const output = new Uint8Array(binary.length);
  for (let index = 0; index < binary.length; index += 1) output[index] = binary.charCodeAt(index);
  return output;
}

function sameBytes(left: Uint8Array, right: Uint8Array): boolean {
  return left.length === right.length && left.every((byte, index) => byte === right[index]);
}

function sameHex(left: string, right: string): boolean {
  return left.toLowerCase().replace(/^0x/, "") === right.toLowerCase().replace(/^0x/, "");
}
