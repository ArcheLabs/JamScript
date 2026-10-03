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
import { asWorkRpc, RpcError, type ActionReceipt, type BackendCapabilitiesV1, type BestContext, type FinalizedContext, type RpcTransport, type StructuredExecutionError, type SubmitActionResult, type SubmitTransactionRequest, type SubmitTransactionResult, type TransactionStatusResult, type WorkRpc, type WorkStatusResult } from "./rpc.js";
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

export type TransactionTrackingOptions = {
  intervalMs?: number;
  timeoutMs?: number;
  signal?: AbortSignal;
  actionHash?: string;
  onUpdate?: (update: TransactionLifecycleUpdate) => void;
};

export type TransactionLifecycleUpdate = {
  transactionId: string;
  status: TransactionStatusResult;
  confirmation: "unknown" | "best" | "finalized";
  actionResult?: ActionReceipt["status"];
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
  | "READING_BEST_CONTEXT"
  | "READING_MANAGED_STATE"
  | "READING_NONCE";

type PreparedOwnershipActionData = {
  signer: OwnershipSigner;
  nonceLane: string;
  nonce: bigint;
  unsigned: Omit<SignedActionV2, "authorizationProof">;
  message: Uint8Array;
  requestOptions: Pick<SubmitTransactionRequest, "extrinsicsBase64">;
  submittedSlot: number;
  validUntil: number;
};

type SignedOwnershipActionData = {
  nonceLane: string;
  nonce: bigint;
  request: SubmitTransactionRequest;
  actionHash: string;
  submittedSlot: number;
  validUntil: number;
};

export type SignedActionTrackingInfo = {
  actionHash: string;
  submittedSlot: number;
  validUntil: number;
};

export type PreparedActionTrackingInfo = Omit<SignedActionTrackingInfo, "actionHash"> & { actionName: string };

const preparedOwnershipActions = new WeakMap<PreparedOwnershipAction, PreparedOwnershipActionData>();
const signedOwnershipActions = new WeakMap<SignedOwnershipAction, SignedOwnershipActionData>();

export class TransactionTrackingError extends Error {
  constructor(
    message: string,
    readonly code: string,
    readonly transactionId: string,
    readonly actionHash?: string,
    readonly lastStatus?: TransactionStatusResult,
    readonly failureStage?: string,
    options?: ErrorOptions,
    readonly errorInfo?: StructuredExecutionError,
  ) {
    super(message, options);
    this.name = "TransactionTrackingError";
  }
}

export class TransactionWaitTimeoutError extends TransactionTrackingError {
  constructor(
    transactionId: string,
    lastStatus?: TransactionStatusResult,
    actionHash?: string,
    reason: "timeout" | "receipt_unavailable" | "status_unavailable" = "timeout",
    cause?: unknown,
  ) {
    super(
      reason === "receipt_unavailable"
        ? `transaction ${transactionId} is finalized but its matching action receipt is not available yet`
        : reason === "status_unavailable"
          ? `timed out waiting for transaction ${transactionId}; its status is still unavailable`
          : lastStatus
            ? `timed out waiting for transaction ${transactionId} (last status: ${lastStatus.status})`
            : `timed out waiting for transaction ${transactionId}`,
      reason.toUpperCase(),
      transactionId,
      actionHash,
      lastStatus,
      undefined,
      cause === undefined ? undefined : { cause },
    );
    this.name = "TransactionWaitTimeoutError";
  }
}

export class TransactionTrackingAbortedError extends TransactionTrackingError {
  constructor(transactionId: string, lastStatus?: TransactionStatusResult, actionHash?: string, cause?: unknown) {
    super("transaction tracking was stopped; the transaction remains unresolved", "ABORTED", transactionId, actionHash, lastStatus, undefined, cause === undefined ? undefined : { cause });
    this.name = "TransactionTrackingAbortedError";
  }
}

export class JamScriptClient {
  private readonly rpc: WorkRpc;
  private readonly actionHashes = new Map<string, string>();
  private readonly nextNonces = new Map<string, bigint>();
  private readonly nonceTails = new Map<string, Promise<void>>();
  private readonly ownershipTails = new Map<string, Promise<void>>();
  private readonly ownershipSubmitTails = new Map<string, Promise<void>>();
  private readonly ownershipReservations = new Map<string, Map<bigint, object>>();
  private readonly nextOwnershipNonces = new Map<string, bigint>();
  private readonly ownershipBlockedNonces = new Map<string, bigint>();
  private readonly ownershipTransactionNonces = new Map<string, { nonceLane: string; nonce: bigint }>();
  private readonly walletTransactionNonces = new Map<string, { signerKey: string; nonce: bigint }>();
  private readonly walletBlockedNonces = new Map<string, bigint>();
  private readonly statusReads = new Map<string, Promise<TransactionStatusResult>>();
  private lifecycleCapabilities?: Promise<BackendCapabilitiesV1>;

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

  transactionScope(): Pick<DeploymentDescriptor, "genesisHash" | "networkDomain" | "serviceId" | "serviceKey" | "codeHash"> {
    return {
      genesisHash: this.deployment.genesisHash,
      networkDomain: this.deployment.networkDomain,
      serviceId: this.deployment.serviceId,
      serviceKey: this.deployment.serviceKey,
      codeHash: this.deployment.codeHash,
    };
  }

  async readNonce(publicKey: Uint8Array, context?: FinalizedContext): Promise<bigint> {
    if (publicKey.length !== 32) throw new Error("sr25519 public key must be 32 bytes");
    const finalized = { ...(context ?? await this.rpc.finalizedContext()), contextType: "finalized" as const };
    const root = await this.managedStateRoot(finalized);
    const key = nonceKey(publicKey);
    const valueBytes = await this.readManagedValue(root, key, finalized);
    if (valueBytes === null) return 0n;
    const value = decodeValue("u64", valueBytes);
    if (typeof value !== "bigint") throw new Error("nonce storage is not u64");
    return value;
  }

  async submitAction(
    actionName: string,
    input: Record<string, CodecValue>,
    signer: JamSigner,
    options: { ttl?: bigint; extrinsics?: Uint8Array[]; staleRetries?: number; onSigned?: (info: SignedActionTrackingInfo) => void | Promise<void> } = {},
  ): Promise<SubmitActionResult> {
    await this.assertTransactionLifecycleSupport();
    if (signer.publicKey.length !== 32) throw new Error("sr25519 public key must be 32 bytes");
    const signerKey = toHex(signer.publicKey).toLowerCase();
    const prepared = await this.prepareAction(
      actionName,
      input,
      signer,
      options,
    );
    try {
      await options.onSigned?.({ actionHash: prepared.actionHash, submittedSlot: prepared.submittedSlot, validUntil: prepared.validUntil });
      } catch (error) {
        if (this.nextNonces.get(signerKey) === prepared.nonce + 1n) this.nextNonces.delete(signerKey);
        else {
          const blocked = this.walletBlockedNonces.get(signerKey);
          if (blocked === undefined || prepared.nonce < blocked) this.walletBlockedNonces.set(signerKey, prepared.nonce);
        }
        throw error;
    }
    let submitted: SubmitTransactionResult;
    try {
      submitted = await this.rpc.submitTransaction(prepared.request);
    } catch (error) {
      if (isDefiniteSubmissionRejection(error)) {
        if (this.nextNonces.get(signerKey) === prepared.nonce + 1n) this.nextNonces.delete(signerKey);
      } else {
        const blocked = this.walletBlockedNonces.get(signerKey);
        if (blocked === undefined || prepared.nonce < blocked) this.walletBlockedNonces.set(signerKey, prepared.nonce);
      }
      throw error;
    }
    this.actionHashes.set(submitted.transactionId.toLowerCase(), prepared.actionHash);
    this.walletTransactionNonces.set(submitted.transactionId.toLowerCase(), { signerKey, nonce: prepared.nonce });
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
    options: {
      actAs?: Ownership;
      ttl?: bigint;
      extrinsics?: Uint8Array[];
      onPrepared?: (info: PreparedActionTrackingInfo) => void | Promise<void>;
      onSigned?: (info: SignedActionTrackingInfo) => void | Promise<void>;
    } = {},
  ): Promise<SubmitActionResult> {
    // Fail before deriving nonces or prompting the Ownership signer when the
    // configured Backend cannot provide the lifecycle evidence Locus requires.
    await this.assertTransactionLifecycleSupport();
    const controller = await signer.getController();
    const nonceLane = toHex(ownershipNonceKey(options.actAs ?? controller)).toLowerCase();
    const previous = this.ownershipSubmitTails.get(nonceLane) ?? Promise.resolve();
    let release!: () => void;
    const lane = new Promise<void>((resolve) => { release = resolve; });
    this.ownershipSubmitTails.set(nonceLane, lane);
    await previous;
    try {
      const prepared = await this.prepareOwnershipAction(actionName, input, signer, options);
      const preparedData = preparedOwnershipActions.get(prepared);
      try {
        if (!preparedData) throw new Error("prepared Ownership action is unavailable");
        await options.onPrepared?.({
          actionName,
          submittedSlot: preparedData.submittedSlot,
          validUntil: preparedData.validUntil,
        });
      } catch (error) {
        this.abandonPreparedOwnershipAction(prepared);
        throw error;
      }
      const signed = await this.signPreparedOwnershipAction(prepared);
      return await this.submitSignedOwnershipAction(signed, { onSigned: options.onSigned });
    } finally {
      release();
      if (this.ownershipSubmitTails.get(nonceLane) === lane) this.ownershipSubmitTails.delete(nonceLane);
    }
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
    await this.assertTransactionLifecycleSupport();
    const controller = await signer.getController();
    const effectiveOwner = options.actAs ?? controller;
    const nonceLane = toHex(ownershipNonceKey(effectiveOwner)).toLowerCase();
    const previous = this.ownershipTails.get(nonceLane) ?? Promise.resolve();
    let release!: () => void;
    const lane = new Promise<void>((resolve) => { release = resolve; });
    this.ownershipTails.set(nonceLane, lane);
    await previous;
    try {
      options.onProgress?.("VALIDATING_DEPLOYMENT");
      await this.validateDeployment();
      const action = actionByName(this.deployment.abi, actionName);
      if (!isOwnershipAuth(action.auth)) throw new Error("prepareOwnershipAction requires an ownership-authenticated action");
      const payload = encodeActionPayload(this.deployment.abi, actionName, input);
      const selector = actionSelector(actionName);
      if (!sameHex(toHex(selector), action.selector)) throw new Error("deployment ABI selector does not match the canonical selector");
      options.onProgress?.("READING_BEST_CONTEXT");
      const context = await this.rpc.bestContext();
      options.onProgress?.("READING_MANAGED_STATE");
      const root = await this.managedStateRoot(context);
      options.onProgress?.("READING_NONCE");
      const nonceBytes = await this.readManagedValue(root, ownershipNonceKey(effectiveOwner), context);
      const chainNonce = nonceBytes === null ? 0n : decodeValue("u64", nonceBytes);
      if (typeof chainNonce !== "bigint") throw new Error("ownership nonce storage is not u64");
      const blockedNonce = this.ownershipBlockedNonces.get(nonceLane);
      if (blockedNonce !== undefined) {
        if (chainNonce <= blockedNonce) {
          throw new Error("OWNERSHIP_NONCE_RECONCILIATION_REQUIRED: an earlier Ownership submission or reservation has an unresolved outcome");
        }
        this.ownershipBlockedNonces.delete(nonceLane);
        const laneReservations = this.ownershipReservations.get(nonceLane);
        if (laneReservations) {
          for (const [reservedNonce, reservation] of laneReservations) {
            if (reservedNonce < chainNonce) {
              this.deleteOwnershipReservation(nonceLane, reservedNonce, reservation);
            }
          }
        }
        for (const [transactionId, pendingNonce] of this.ownershipTransactionNonces) {
          if (pendingNonce.nonceLane === nonceLane && pendingNonce.nonce < chainNonce) {
            this.ownershipTransactionNonces.delete(transactionId);
          }
        }
      }
      const localNonce = this.nextOwnershipNonces.get(nonceLane);
      const nonce = localNonce === undefined || localNonce < chainNonce ? chainNonce : localNonce;
      this.nextOwnershipNonces.set(nonceLane, nonce + 1n);
      const unsigned: Omit<SignedActionV2, "authorizationProof"> = {
        version: 2,
        networkDomain: parseHex(this.deployment.networkDomain, 32),
        serviceKey: parseHex(this.deployment.serviceKey, 32),
        actionSelector: selector,
        controller,
        actAs: options.actAs ?? null,
        nonce,
        validUntil: BigInt(context.slot) + (options.ttl ?? 64n),
        payloadHash: blake2(payload),
        payload,
      };
      const message = signingMessageV2(unsigned);
      const prepared = Object.freeze({ phase: "prepared" as const, actionName });
      const validUntil = Number(unsigned.validUntil);
      preparedOwnershipActions.set(prepared, {
        signer,
        nonceLane,
        nonce,
        unsigned,
        message,
        requestOptions: { extrinsicsBase64: (options.extrinsics ?? []).map(toBase64) },
        submittedSlot: context.slot,
        validUntil,
      });
      this.setOwnershipReservation(nonceLane, nonce, prepared);
      return prepared;
    } finally {
      release();
      if (this.ownershipTails.get(nonceLane) === lane) this.ownershipTails.delete(nonceLane);
    }
  }

  /**
   * Invoke the wallet signer immediately, with no network or state reads in
   * this phase. Call this directly from the user's signature button handler.
   */
  signPreparedOwnershipAction(prepared: PreparedOwnershipAction): Promise<SignedOwnershipAction> {
    const data = preparedOwnershipActions.get(prepared);
    if (!data || this.ownershipReservation(data.nonceLane, data.nonce) !== prepared) {
      return Promise.reject(new Error("prepared Ownership action is unavailable or already consumed"));
    }

    let signature: Promise<Uint8Array>;
    try {
      signature = data.signer.signJamScriptAction({ ...data.unsigned, message: data.message });
    } catch (error) {
      preparedOwnershipActions.delete(prepared);
      this.deleteOwnershipReservation(data.nonceLane, data.nonce, prepared);
      this.rewindOwnershipNonce(data.nonceLane, data.nonce);
      return Promise.reject(error);
    }

    return signature.then((authorizationProof) => {
      if (this.ownershipReservation(data.nonceLane, data.nonce) !== prepared) {
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
        nonceLane: data.nonceLane,
        nonce: data.nonce,
        request,
        actionHash,
        submittedSlot: data.submittedSlot,
        validUntil: data.validUntil,
      });
      this.setOwnershipReservation(data.nonceLane, data.nonce, signed);
      return signed;
    }).catch((error) => {
      preparedOwnershipActions.delete(prepared);
      if (this.ownershipReservation(data.nonceLane, data.nonce) === prepared) {
        this.deleteOwnershipReservation(data.nonceLane, data.nonce, prepared);
        this.rewindOwnershipNonce(data.nonceLane, data.nonce);
      }
      throw error;
    });
  }

  /** Release an unsigned preparation after wallet rejection or timeout. */
  abandonPreparedOwnershipAction(prepared: PreparedOwnershipAction): void {
    const data = preparedOwnershipActions.get(prepared);
    if (!data) return;
    preparedOwnershipActions.delete(prepared);
    if (this.ownershipReservation(data.nonceLane, data.nonce) === prepared) {
      this.deleteOwnershipReservation(data.nonceLane, data.nonce, prepared);
      this.rewindOwnershipNonce(data.nonceLane, data.nonce);
    }
  }

  private rewindOwnershipNonce(nonceLane: string, nonce: bigint): void {
    const hasHigherReservation = this.nextOwnershipNonces.get(nonceLane)! > nonce + 1n
      || [...(this.ownershipReservations.get(nonceLane)?.keys() ?? [])]
        .some((reservedNonce) => reservedNonce > nonce);
    if (hasHigherReservation) {
      const blocked = this.ownershipBlockedNonces.get(nonceLane);
      if (blocked === undefined || nonce < blocked) this.ownershipBlockedNonces.set(nonceLane, nonce);
      return;
    }
    if (this.nextOwnershipNonces.get(nonceLane) === nonce + 1n) this.nextOwnershipNonces.delete(nonceLane);
  }

  private ownershipReservation(nonceLane: string, nonce: bigint): object | undefined {
    return this.ownershipReservations.get(nonceLane)?.get(nonce);
  }

  private setOwnershipReservation(nonceLane: string, nonce: bigint, reservation: object): void {
    let lane = this.ownershipReservations.get(nonceLane);
    if (!lane) {
      lane = new Map();
      this.ownershipReservations.set(nonceLane, lane);
    }
    lane.set(nonce, reservation);
  }

  private deleteOwnershipReservation(nonceLane: string, nonce: bigint, expected: object): void {
    const lane = this.ownershipReservations.get(nonceLane);
    if (!lane || lane.get(nonce) !== expected) return;
    lane.delete(nonce);
    if (lane.size === 0) this.ownershipReservations.delete(nonceLane);
  }

  /** Submit one signed payload. This method never retries a transport error. */
  async submitSignedOwnershipAction(
    signed: SignedOwnershipAction,
    options: { onSigned?: (info: SignedActionTrackingInfo) => void | Promise<void> } = {},
  ): Promise<SubmitActionResult> {
    await this.assertTransactionLifecycleSupport();
    const data = signedOwnershipActions.get(signed);
    if (!data || this.ownershipReservation(data.nonceLane, data.nonce) !== signed) {
      throw new Error("signed Ownership action is unavailable or already submitted");
    }
    const blockedNonce = this.ownershipBlockedNonces.get(data.nonceLane);
    if (blockedNonce !== undefined && data.nonce >= blockedNonce) {
      this.abandonSignedOwnershipAction(signed);
      throw new Error("OWNERSHIP_NONCE_RECONCILIATION_REQUIRED: resolve the earlier Ownership nonce before submitting this action");
    }
    try {
      await options.onSigned?.({ actionHash: data.actionHash, submittedSlot: data.submittedSlot, validUntil: data.validUntil });
    } catch (error) {
      this.abandonSignedOwnershipAction(signed);
      throw error;
    }
    let submitted: SubmitTransactionResult;
    try {
      submitted = await this.rpc.submitTransaction(data.request);
    } catch (error) {
      // Only explicit server-side rejection classes are safe to rewind.
      // HTTP 5xx, timeouts, and network errors may follow an accepted request.
      signedOwnershipActions.delete(signed);
      if (isDefiniteSubmissionRejection(error) && this.ownershipReservation(data.nonceLane, data.nonce) === signed) {
        this.deleteOwnershipReservation(data.nonceLane, data.nonce, signed);
        this.rewindOwnershipNonce(data.nonceLane, data.nonce);
      } else if (!(error instanceof RpcError)) {
        const blocked = this.ownershipBlockedNonces.get(data.nonceLane);
        if (blocked === undefined || data.nonce < blocked) this.ownershipBlockedNonces.set(data.nonceLane, data.nonce);
      }
      throw error;
    }
    this.actionHashes.set(submitted.transactionId.toLowerCase(), data.actionHash);
    this.ownershipTransactionNonces.set(submitted.transactionId.toLowerCase(), {
      nonceLane: data.nonceLane,
      nonce: data.nonce,
    });
    signedOwnershipActions.delete(signed);
    if (this.ownershipReservation(data.nonceLane, data.nonce) === signed) {
      this.deleteOwnershipReservation(data.nonceLane, data.nonce, signed);
    }
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
      let localNonce = this.nextNonces.get(signerKey);
      let chainNonce = localNonce === undefined || this.walletBlockedNonces.has(signerKey)
        ? await this.readNonce(publicKey, context)
        : localNonce;
      const blockedNonce = this.walletBlockedNonces.get(signerKey);
      if (blockedNonce !== undefined) {
        if (chainNonce <= blockedNonce) {
          throw new Error("WALLET_NONCE_RECONCILIATION_REQUIRED: an earlier transaction has an unresolved submission or reorganization outcome");
        }
        this.walletBlockedNonces.delete(signerKey);
        this.walletTransactionNonces.forEach((pending, transactionId) => {
          if (pending.signerKey === signerKey && pending.nonce < chainNonce) this.walletTransactionNonces.delete(transactionId);
        });
        this.nextNonces.delete(signerKey);
        localNonce = undefined;
      }
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

  /** Release a signature that has not yet been sent to the Backend. */
  abandonSignedOwnershipAction(signed: SignedOwnershipAction): void {
    const data = signedOwnershipActions.get(signed);
    if (!data) return;
    signedOwnershipActions.delete(signed);
    if (this.ownershipReservation(data.nonceLane, data.nonce) === signed) {
      this.deleteOwnershipReservation(data.nonceLane, data.nonce, signed);
      this.rewindOwnershipNonce(data.nonceLane, data.nonce);
    }
  }

  finalizedContext(): Promise<FinalizedContext> {
    return this.rpc.finalizedContext();
  }

  bestContext(): Promise<BestContext> {
    return this.rpc.bestContext();
  }

  async queryFinalized(queryName: string, key?: CodecValue): Promise<QueryResult> {
    const context = { ...await this.rpc.finalizedContext(), contextType: "finalized" as const };
    return this.queryAtContext(queryName, key, context);
  }

  async queryBest(queryName: string, key?: CodecValue): Promise<QueryResult> {
    const context = await this.rpc.bestContext();
    return this.queryAtContext(queryName, key, context);
  }

  /** @deprecated Select queryBest or queryFinalized explicitly. */
  queryLatest(queryName: string, key?: CodecValue): Promise<QueryResult> {
    return this.queryFinalized(queryName, key);
  }

  private async queryAtContext(
    queryName: string,
    key: CodecValue | undefined,
    context: FinalizedContext,
  ): Promise<QueryResult> {
    const query = queryByName(this.deployment.abi, queryName);
    const state = stateByName(this.deployment.abi, query.state);
    const keyBytes = isUnitType(state.keyType)
      ? new Uint8Array()
      : encodeValue(state.keyType, key === undefined ? null : key);
    if (JSON.stringify(query.keyType) !== JSON.stringify(state.keyType)) throw new Error("query key type does not match state key type");
    const root = await this.managedStateRoot(context);
    const valueBytes = await this.readManagedValue(
      root,
      stateKey(state.schema, keyBytes),
      context,
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
    return this.queryBest(queryName, key);
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
    context?: FinalizedContext,
  ): Promise<Uint8Array | null> {
    return this.readManagedValueFor(root, this.deployment, key, context);
  }

  private async readManagedValueFor(
    root: Uint8Array,
    deployment: Pick<DeploymentDescriptor, "serviceId" | "serviceKey">,
    key: Uint8Array,
    context?: FinalizedContext,
  ): Promise<Uint8Array | null> {
    const response = await this.stateProvider.get({ serviceId: deployment.serviceId, serviceKey: deployment.serviceKey, stateRoot: toHex(root), key, context });
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

  async transactionStatus(transactionId: string): Promise<TransactionStatusResult> {
    const status = await this.readTransactionStatus(transactionId);
    const transactionKey = transactionId.toLowerCase();
    const walletNonce = this.walletTransactionNonces.get(transactionKey);
    if (walletNonce) {
      const actionHash = this.actionHashes.get(transactionKey)?.toLowerCase();
      const receipt = actionHash
        ? status.actionReceipts?.find((candidate) => candidate.actionHash.toLowerCase() === actionHash)
        : undefined;
      if (status.status === "reorged" || status.status === "submission_unknown" || status.status === "failed"
        || (status.bestChainStatus === "included" && status.finalized !== true)
        || (receipt && receipt.status !== "applied")) {
        const blocked = this.walletBlockedNonces.get(walletNonce.signerKey);
        if (blocked === undefined || walletNonce.nonce < blocked) this.walletBlockedNonces.set(walletNonce.signerKey, walletNonce.nonce);
      } else if (status.finalized === true && receipt?.status === "applied") {
        this.walletTransactionNonces.delete(transactionKey);
      }
    }
    const ownershipNonce = this.ownershipTransactionNonces.get(transactionKey);
    if (ownershipNonce) {
      const actionHash = this.actionHashes.get(transactionKey)?.toLowerCase();
      const actionReceipt = actionHash
        ? status.actionReceipts?.find((receipt) => receipt.actionHash.toLowerCase() === actionHash)
        : undefined;
      if (status.status === "reorged" || status.status === "failed" || status.status === "submission_unknown"
        || (status.bestChainStatus === "included" && status.finalized !== true)
        || (actionReceipt && actionReceipt.status !== "applied")) {
        const blocked = this.ownershipBlockedNonces.get(ownershipNonce.nonceLane);
        if (blocked === undefined || ownershipNonce.nonce < blocked) {
          this.ownershipBlockedNonces.set(ownershipNonce.nonceLane, ownershipNonce.nonce);
        }
      } else if (status.finalized === true) {
        this.ownershipTransactionNonces.delete(transactionKey);
      }
    }
    return status;
  }

  private readTransactionStatus(transactionId: string): Promise<TransactionStatusResult> {
    const key = transactionId.toLowerCase();
    const current = this.statusReads.get(key);
    if (current) return current;
    const request = this.rpc.transactionStatus(transactionId);
    this.statusReads.set(key, request);
    void request.finally(() => {
      if (this.statusReads.get(key) === request) this.statusReads.delete(key);
    }).catch(() => undefined);
    return request;
  }

  private async requireLifecycleCapabilities(transactionId: string, actionHash?: string): Promise<BackendCapabilitiesV1> {
    this.lifecycleCapabilities ??= this.rpc.capabilities();
    let capabilities: BackendCapabilitiesV1;
    try {
      capabilities = await this.lifecycleCapabilities;
    } catch (cause) {
      this.lifecycleCapabilities = undefined;
      throw new TransactionTrackingError(
        "the Backend does not expose verifiable transaction lifecycle capabilities",
        "LIFECYCLE_UNSUPPORTED",
        transactionId,
        actionHash,
        undefined,
        "capability_check",
        { cause },
      );
    }
    if (capabilities.transactionLifecycleVersion !== 1
      || capabilities.bestChainTracking !== true
      || capabilities.strictFinalizedReceipts !== true) {
      throw new TransactionTrackingError(
        "the configured Backend does not support strict transaction lifecycle tracking; upgrade the Backend before submitting or confirming actions",
        "LIFECYCLE_UNSUPPORTED",
        transactionId,
        actionHash,
        undefined,
        "capability_check",
      );
    }
    return capabilities;
  }

  async assertTransactionLifecycleSupport(): Promise<BackendCapabilitiesV1> {
    const capabilities = await this.requireLifecycleCapabilities("preflight");
    return capabilities;
  }

  private async pollTransaction(
    transactionId: string,
    options: TransactionTrackingOptions,
    accept: (status: TransactionStatusResult) => boolean,
  ): Promise<TransactionStatusResult> {
    const intervalMs = Math.max(100, options.intervalMs ?? 1_000);
    const deadline = Date.now() + Math.max(1, options.timeoutMs ?? 120_000);
    let lastStatus: TransactionStatusResult | undefined;
    let previousUpdateKey = "";
    let transientFailures = 0;
    let lastCause: unknown;
    const emit = (status: TransactionStatusResult) => {
      const confirmation = status.finalized === true
        ? "finalized"
        : status.bestChainStatus === "included" && status.bestContext?.contextType === "best"
          ? "best"
          : "unknown";
      const targetReceipt = this.resolveActionReceipt(status, options.actionHash, transactionId, false);
      const update: TransactionLifecycleUpdate = {
        transactionId,
        status,
        confirmation,
        ...(targetReceipt ? { actionResult: targetReceipt.status } : {}),
      };
      const key = JSON.stringify(update);
      if (key !== previousUpdateKey) {
        previousUpdateKey = key;
        options.onUpdate?.(update);
      }
    };
    for (;;) {
      if (options.signal?.aborted) {
        throw new TransactionTrackingAbortedError(transactionId, lastStatus, options.actionHash, options.signal.reason);
      }
      try {
        const status = await this.transactionStatus(transactionId);
        lastStatus = status;
        transientFailures = 0;
        lastCause = undefined;
        emit(status);
        if (accept(status)) return status;
      } catch (cause) {
        if (cause instanceof TransactionTrackingError) throw cause;
        if (!isRetryableTrackingError(cause)) throw cause;
        transientFailures += 1;
        lastCause = cause;
      }
      if (Date.now() >= deadline) {
        const reason = lastStatus?.finalized === true
          ? "receipt_unavailable"
          : lastStatus === undefined
            ? "status_unavailable"
            : "timeout";
        throw new TransactionWaitTimeoutError(transactionId, lastStatus, options.actionHash, reason, lastCause);
      }
      const backoff = Math.min(intervalMs * 2 ** Math.min(transientFailures, 5), 10_000);
      await waitForDelay(Math.min(backoff, Math.max(1, deadline - Date.now())), options.signal, transactionId, lastStatus, options.actionHash);
    }
  }

  private resolveActionReceipt(
    status: TransactionStatusResult,
    actionHash: string | undefined,
    transactionId: string,
    requireReceipt = true,
  ): ActionReceipt | undefined {
    if (!actionHash) {
      if (!requireReceipt) return undefined;
      throw new TransactionTrackingError(
        "action hash is unavailable; restore tracking with the submitted actionHash",
        "ACTION_IDENTITY_UNAVAILABLE",
        transactionId,
        undefined,
        status,
        "receipt_mapping",
      );
    }
    const receipts = status.actionReceipts ?? [];
    if (status.actionIndex !== null) {
      const receipt = receipts[status.actionIndex];
      if (!receipt) return undefined;
      if (!sameHex(receipt.actionHash, actionHash)) {
        throw new TransactionTrackingError(
          "Backend actionIndex points to a receipt with a different actionHash",
          "ACTION_IDENTITY_MISMATCH",
          transactionId,
          actionHash,
          status,
          "receipt_mapping",
        );
      }
      return receipt;
    }
    const matches = receipts.filter((receipt) => sameHex(receipt.actionHash, actionHash));
    if (matches.length > 1) {
      throw new TransactionTrackingError(
        "multiple action receipts match the saved actionHash",
        "ACTION_IDENTITY_AMBIGUOUS",
        transactionId,
        actionHash,
        status,
        "receipt_mapping",
      );
    }
    return matches[0];
  }

  async waitForTransaction(
    transactionId: string,
    options: TransactionTrackingOptions & { confirmation?: "best" | "finalized" } = {},
  ): Promise<TransactionStatusResult> {
    await this.requireLifecycleCapabilities(transactionId, options.actionHash);
    const confirmation = options.confirmation ?? "finalized";
    return this.pollTransaction(transactionId, options, (status) => status.status === "failed"
      || status.status === "submission_unknown"
      || (confirmation === "best"
        ? status.finalized === true || (status.bestChainStatus === "included" && status.bestContext?.contextType === "best")
        : status.finalized === true));
  }

  async waitForBest(
    transactionId: string,
    options: TransactionTrackingOptions = {},
  ): Promise<TransactionStatusResult> {
    await this.requireLifecycleCapabilities(transactionId, options.actionHash);
    return this.pollTransaction(transactionId, options, (status) => status.status === "failed"
      || status.status === "reorged"
      || status.status === "submission_unknown"
      || status.finalized === true
      || (status.bestChainStatus === "included" && status.bestContext?.contextType === "best"));
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
    optionsOrActionHash: TransactionTrackingOptions | string = {},
    legacyOptions: TransactionTrackingOptions = {},
  ): Promise<WaitForActionResult> {
    const options = { ...(typeof optionsOrActionHash === "string" ? legacyOptions : optionsOrActionHash) };
    const legacyHash = typeof optionsOrActionHash === "string" ? optionsOrActionHash : undefined;
    if (legacyHash && options.actionHash && !sameHex(legacyHash, options.actionHash)) {
      throw new TransactionTrackingError("conflicting actionHash arguments were supplied", "ACTION_IDENTITY_MISMATCH", transactionId, options.actionHash);
    }
    const expected = options.actionHash ?? legacyHash ?? this.actionHashes.get(transactionId.toLowerCase());
    if (!expected) {
      throw new TransactionTrackingError("action hash is unavailable; pass actionHash to restore tracking", "ACTION_IDENTITY_UNAVAILABLE", transactionId);
    }
    options.actionHash = expected;
    await this.requireLifecycleCapabilities(transactionId, expected);
    const transaction = await this.pollTransaction(transactionId, options, (status) => status.finalized === true
      && this.resolveActionReceipt(status, expected, transactionId) !== undefined
      || status.status === "submission_unknown"
      || status.status === "failed");
    if (transaction.status === "submission_unknown") {
      throw new TransactionTrackingError("the Backend cannot determine whether this transaction was submitted; keep it pending and do not resubmit automatically", "SUBMISSION_UNKNOWN", transactionId, expected, transaction, "submission");
    }
    if (transaction.status === "failed") {
      throw new TransactionTrackingError(
        `transaction failed before a finalized action receipt was produced: ${transaction.error ?? "unknown Work failure"}`,
        transaction.errorInfo?.code ?? "WORK_FAILED",
        transactionId,
        expected,
        transaction,
        "work",
        undefined,
        transaction.errorInfo ?? undefined,
      );
    }
    const receipt = this.resolveActionReceipt(transaction, expected, transactionId);
    if (!receipt) throw new TransactionWaitTimeoutError(transactionId, transaction, expected, "receipt_unavailable");
    return {
      ...transaction,
      status: receipt.status,
      transactionStatus: transaction.status,
      actionHash: receipt.actionHash,
      errorCode: receipt.errorCode,
      actionReceipt: receipt,
    };
  }

  waitForFinalized(
    transactionId: string,
    options: TransactionTrackingOptions = {},
  ): Promise<WaitForActionResult> {
    return this.waitForAction(transactionId, options);
  }

  watchTransaction(transactionId: string, options: TransactionTrackingOptions = {}): Promise<WaitForActionResult> {
    return this.waitForFinalized(transactionId, options);
  }
}

function isRetryableTrackingError(error: unknown): boolean {
  if (error instanceof RpcError) {
    return error.code === -32013 || error.code === 408 || error.code === 429 || error.code >= 500;
  }
  return error instanceof TypeError
    || (error instanceof Error && (error.name === "FetchError" || error.name === "NetworkError"));
}

function isDefiniteSubmissionRejection(error: unknown): boolean {
  if (!(error instanceof RpcError)) return false;
  if ([
    -32600, -32601, -32602,
    -32003, -32010, -32011, -32031, -32032, -32033, -32043, -32044, -32045,
  ].includes(error.code)) return true;
  return [400, 401, 403, 404, 409, 422].includes(error.code);
}

function waitForDelay(
  milliseconds: number,
  signal: AbortSignal | undefined,
  transactionId: string,
  lastStatus?: TransactionStatusResult,
  actionHash?: string,
): Promise<void> {
  if (signal?.aborted) {
    return Promise.reject(new TransactionTrackingAbortedError(transactionId, lastStatus, actionHash, signal.reason));
  }
  return new Promise((resolve, reject) => {
    const timer = setTimeout(() => {
      signal?.removeEventListener("abort", abort);
      resolve();
    }, milliseconds);
    const abort = () => {
      clearTimeout(timer);
      signal?.removeEventListener("abort", abort);
      reject(new TransactionTrackingAbortedError(transactionId, lastStatus, actionHash, signal?.reason));
    };
    signal?.addEventListener("abort", abort, { once: true });
  });
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
