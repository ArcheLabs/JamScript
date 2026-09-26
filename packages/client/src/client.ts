import { actionByName, queryByName, stateByName, type DeploymentDescriptor } from "./abi.js";
import { decodeStateValue, decodeValue, encodeActionPayload, encodeValue, type CodecValue } from "./codec.js";
import {
  actionSelector,
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
import { asWorkRpc, RpcError, type ActionReceipt, type FinalizedContext, type RpcTransport, type SubmitActionResult, type SubmitTransactionRequest, type SubmitTransactionResult, type TransactionStatusResult, type WorkRpc, type WorkStatusResult } from "./rpc.js";
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

/** Opaque action data prepared before requesting a wallet signature. */
export type PreparedOwnershipAction = {
  readonly phase: "prepared";
  readonly actionName: string;
};

/** Opaque signed action data ready for one backend submission attempt. */
export type SignedOwnershipAction = {
  readonly phase: "signed";
  readonly actionName: string;
  readonly actionHash: string;
  readonly submittedSlot: number;
  readonly validUntil: number;
};

export type OwnershipPreparationPhase =
  | "VALIDATING_DEPLOYMENT"
  | "READING_FINALIZED_CONTEXT"
  | "READING_MANAGED_STATE"
  | "READING_NONCE";

type PreparedOwnershipActionData = {
  signer: OwnershipSigner;
  signerLane: string;
  unsigned: Omit<SignedActionV2, "authorizationProof">;
  message: Uint8Array;
  requestOptions: Pick<SubmitTransactionRequest, "extrinsicsBase64">;
  submittedSlot: number;
  validUntil: number;
};

type SignedOwnershipActionData = {
  signerLane: string;
  request: SubmitTransactionRequest;
  actionHash: string;
  submittedSlot: number;
  validUntil: number;
};

const preparedOwnershipActions = new WeakMap<PreparedOwnershipAction, PreparedOwnershipActionData>();
const signedOwnershipActions = new WeakMap<SignedOwnershipAction, SignedOwnershipActionData>();

export class TransactionWaitTimeoutError extends Error {
  constructor(
    readonly transactionId: string,
    readonly lastStatus?: TransactionStatusResult,
  ) {
    super(
      lastStatus
        ? `timed out waiting for transaction ${transactionId} (last status: ${lastStatus.status})`
        : `timed out waiting for transaction ${transactionId}`,
    );
    this.name = "TransactionWaitTimeoutError";
  }
}

export class JamScriptClient {
  private readonly rpc: WorkRpc;
  private readonly actionHashes = new Map<string, string>();
  private readonly nextNonces = new Map<string, bigint>();
  private readonly nonceTails = new Map<string, Promise<void>>();
  private readonly ownershipTails = new Map<string, Promise<void>>();
  private readonly ownershipReservations = new Map<string, object>();

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
    return {
      ...submitted,
      actionHash: prepared.actionHash,
      submittedSlot: prepared.submittedSlot,
      validUntil: prepared.validUntil,
    };
  }

  async submitOwnershipAction(
    actionName: string,
    input: Record<string, CodecValue>,
    signer: OwnershipSigner,
    options: { actAs?: Ownership; ttl?: bigint; extrinsics?: Uint8Array[] } = {},
  ): Promise<SubmitActionResult> {
    const prepared = await this.prepareOwnershipAction(actionName, input, signer, options);
    const signed = await this.signPreparedOwnershipAction(prepared);
    return this.submitSignedOwnershipAction(signed);
  }

  /**
   * Resolve deployment, action encoding, finalized context, and Ownership
   * nonce before the caller asks the user for the final signature.
   */
  async prepareOwnershipAction(
    actionName: string,
    input: Record<string, CodecValue>,
    signer: OwnershipSigner,
    options: { actAs?: Ownership; ttl?: bigint; extrinsics?: Uint8Array[]; onProgress?: (phase: OwnershipPreparationPhase) => void } = {},
  ): Promise<PreparedOwnershipAction> {
    const controller = await signer.getController();
    const signerLane = toHex(controller.public).toLowerCase();
    const previous = this.ownershipTails.get(signerLane) ?? Promise.resolve();
    let release!: () => void;
    const lane = new Promise<void>((resolve) => { release = resolve; });
    this.ownershipTails.set(signerLane, lane);
    await previous;
    try {
      if (this.ownershipReservations.has(signerLane)) {
        throw new Error("another Ownership action is already awaiting signature or submission for this controller");
      }
      options.onProgress?.("VALIDATING_DEPLOYMENT");
      await this.validateDeployment();
      const action = actionByName(this.deployment.abi, actionName);
      if (!isOwnershipAuth(action.auth)) throw new Error("prepareOwnershipAction requires an ownership-authenticated action");
      const payload = encodeActionPayload(this.deployment.abi, actionName, input);
      const selector = actionSelector(actionName);
      if (!sameHex(toHex(selector), action.selector)) throw new Error("deployment ABI selector does not match the canonical selector");
      options.onProgress?.("READING_FINALIZED_CONTEXT");
      const context = await this.rpc.finalizedContext();
      const effectiveOwner = options.actAs ?? controller;
      options.onProgress?.("READING_MANAGED_STATE");
      const root = await this.managedStateRoot(context);
      options.onProgress?.("READING_NONCE");
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
      const prepared = Object.freeze({ phase: "prepared" as const, actionName });
      const validUntil = Number(unsigned.validUntil);
      preparedOwnershipActions.set(prepared, {
        signer,
        signerLane,
        unsigned,
        message,
        requestOptions: { extrinsicsBase64: (options.extrinsics ?? []).map(toBase64) },
        submittedSlot: context.slot,
        validUntil,
      });
      this.ownershipReservations.set(signerLane, prepared);
      return prepared;
    } finally {
      release();
      if (this.ownershipTails.get(signerLane) === lane) this.ownershipTails.delete(signerLane);
    }
  }

  /**
   * Invoke the wallet signer immediately, with no network or state reads in
   * this phase. Call this directly from the user's signature button handler.
   */
  signPreparedOwnershipAction(prepared: PreparedOwnershipAction): Promise<SignedOwnershipAction> {
    const data = preparedOwnershipActions.get(prepared);
    if (!data || this.ownershipReservations.get(data.signerLane) !== prepared) {
      return Promise.reject(new Error("prepared Ownership action is unavailable or already consumed"));
    }

    let signature: Promise<Uint8Array>;
    try {
      signature = data.signer.signJamScriptAction({ ...data.unsigned, message: data.message });
    } catch (error) {
      preparedOwnershipActions.delete(prepared);
      this.ownershipReservations.delete(data.signerLane);
      return Promise.reject(error);
    }

    return signature.then((authorizationProof) => {
      if (this.ownershipReservations.get(data.signerLane) !== prepared) {
        throw new Error("prepared Ownership action was abandoned while the wallet request was open");
      }
      if (authorizationProof.length === 0 || authorizationProof.length > 65536) {
        throw new Error("invalid Ownership authorization proof");
      }
      const bytes = encodeSignedActionV2({ ...data.unsigned, authorizationProof });
      const actionHash = toHex(blake2(bytes));
      const signed = Object.freeze({
        phase: "signed" as const,
        actionName: prepared.actionName,
        actionHash,
        submittedSlot: data.submittedSlot,
        validUntil: data.validUntil,
      });
      const request: SubmitTransactionRequest = {
        serviceId: this.deployment.serviceId,
        serviceCodeHash: this.deployment.codeHash,
        payloadBase64: toBase64(bytes),
        extrinsicsBase64: data.requestOptions.extrinsicsBase64,
      };
      preparedOwnershipActions.delete(prepared);
      signedOwnershipActions.set(signed, {
        signerLane: data.signerLane,
        request,
        actionHash,
        submittedSlot: data.submittedSlot,
        validUntil: data.validUntil,
      });
      this.ownershipReservations.set(data.signerLane, signed);
      return signed;
    }).catch((error) => {
      preparedOwnershipActions.delete(prepared);
      if (this.ownershipReservations.get(data.signerLane) === prepared) this.ownershipReservations.delete(data.signerLane);
      throw error;
    });
  }

  /** Release an unsigned preparation after wallet rejection or timeout. */
  abandonPreparedOwnershipAction(prepared: PreparedOwnershipAction): void {
    const data = preparedOwnershipActions.get(prepared);
    if (!data) return;
    preparedOwnershipActions.delete(prepared);
    if (this.ownershipReservations.get(data.signerLane) === prepared) this.ownershipReservations.delete(data.signerLane);
  }

  /** Submit one signed payload. This method never retries a transport error. */
  async submitSignedOwnershipAction(signed: SignedOwnershipAction): Promise<SubmitActionResult> {
    const data = signedOwnershipActions.get(signed);
    if (!data || this.ownershipReservations.get(data.signerLane) !== signed) {
      throw new Error("signed Ownership action is unavailable or already submitted");
    }
    let submitted: SubmitTransactionResult;
    try {
      submitted = await this.rpc.submitTransaction(data.request);
    } catch (error) {
      // A JSON-RPC error is an explicit rejection. A transport failure is
      // ambiguous: the backend may have accepted the payload before the
      // response was lost, so keep the controller lane reserved and never
      // silently sign or submit a replacement.
      signedOwnershipActions.delete(signed);
      if (error instanceof RpcError) this.ownershipReservations.delete(data.signerLane);
      throw error;
    }
    this.actionHashes.set(submitted.transactionId.toLowerCase(), data.actionHash);
    signedOwnershipActions.delete(signed);
    this.ownershipReservations.delete(data.signerLane);
      return {
        ...submitted,
        actionHash: data.actionHash,
        submittedSlot: data.submittedSlot,
        validUntil: data.validUntil,
      };
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
  ): Promise<{
    request: Parameters<WorkRpc["submitTransaction"]>[0];
    actionHash: string;
    nonce: bigint;
    submittedSlot: number;
    validUntil: number;
  }> {
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

    return {
      request: requestBase,
      actionHash,
      nonce,
      submittedSlot: initialContext.slot,
      validUntil: Number(validUntil),
    };
  }

  finalizedContext(): Promise<FinalizedContext> {
    return this.rpc.finalizedContext();
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
    deployment: Pick<DeploymentDescriptor, "serviceId">,
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
    deployment: Pick<DeploymentDescriptor, "serviceId" | "serviceKey">,
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
    let lastStatus: TransactionStatusResult | undefined;
    for (;;) {
      try {
        const status = await this.transactionStatus(transactionId);
        lastStatus = status;
        if (status.status === "imported" || status.status === "failed") return status;
      } catch (error) {
        if (!(error instanceof RpcError) || error.code !== -32013) throw error;
      }
      if (Date.now() >= deadline) throw new TransactionWaitTimeoutError(transactionId, lastStatus);
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
