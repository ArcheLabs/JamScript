import { actionByName, queryByName, stateByName, type DeploymentDescriptor } from "./abi.js";
import { decodeStateValue, decodeValue, encodeActionPayload, encodeValue, type CodecValue } from "./codec.js";
import {
  actionSelector,
  encodeSignedActionV1,
  MANAGED_STATE_COMMITMENT_KEY_V1,
  nonceKey,
  parseHex,
  signingDigestV1,
  stateKey,
  toHex,
  type SignedActionV1,
} from "./crypto.js";
import { asWorkRpc, RpcError, type ActionReceipt, type FinalizedContext, type RpcTransport, type SubmitActionResult, type SubmitTransactionResult, type TransactionStatusResult, type WorkRpc, type WorkStatusResult } from "./rpc.js";
import type { JamSigner } from "./signer.js";
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
  private readonly signerTails = new Map<string, Promise<void>>();

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
    const previous = this.signerTails.get(signerKey) ?? Promise.resolve();
    let release!: () => void;
    const lane = new Promise<void>((resolve) => {
      release = resolve;
    });
    this.signerTails.set(signerKey, lane);
    await previous;
    try {
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
        // Admission failure means the local reservation may have created a
        // nonce gap.  Force the next action in this signer lane to resync.
        this.nextNonces.delete(signerKey);
        throw error;
      }
      this.actionHashes.set(submitted.transactionId.toLowerCase(), prepared.actionHash);
      return { ...submitted, actionHash: prepared.actionHash };
    } finally {
      release();
      if (this.signerTails.get(signerKey) === lane) this.signerTails.delete(signerKey);
    }
  }

  private async prepareAction(
    actionName: string,
    input: Record<string, CodecValue>,
    signer: JamSigner,
    options: { ttl?: bigint; extrinsics?: Uint8Array[]; staleRetries?: number },
  ): Promise<{ request: Parameters<WorkRpc["submitTransaction"]>[0]; actionHash: string }> {
    await this.validateDeployment();
    const action = actionByName(this.deployment.abi, actionName);
    if (action.auth !== "wallet") throw new Error("submitAction requires a wallet-authenticated action");
    if (signer.publicKey.length !== 32) throw new Error("sr25519 public key must be 32 bytes");
    const payload = encodeActionPayload(this.deployment.abi, actionName, input);
    const selector = actionSelector(actionName);
    if (!sameHex(toHex(selector), action.selector)) {
      throw new Error("deployment ABI selector does not match the canonical selector");
    }
    const initialContext = await this.rpc.finalizedContext();
    const signerKey = toHex(signer.publicKey).toLowerCase();
    const chainNonce = await this.readNonce(signer.publicKey, initialContext);
    const localNonce = this.nextNonces.get(signerKey) ?? 0n;
    const nonce = localNonce < chainNonce ? chainNonce : localNonce;
    this.nextNonces.set(signerKey, nonce + 1n);
    const ttl = options.ttl ?? 64n;
    const validUntil = BigInt(initialContext.slot) + ttl;
    const unsigned: Omit<SignedActionV1, "signature"> = {
      version: 1,
      networkDomain: parseHex(this.deployment.genesisHash, 32),
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

    return { request: requestBase, actionHash };
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
    const encoded = await this.rpc.serviceStorageAt(
      context.blockHash,
      this.deployment.serviceId,
      toHex(MANAGED_STATE_COMMITMENT_KEY_V1),
    );
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
    const response = await this.stateProvider.get({
      serviceId: this.deployment.serviceId,
      serviceKey: this.deployment.serviceKey,
      stateRoot: toHex(root),
      key,
    });
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
