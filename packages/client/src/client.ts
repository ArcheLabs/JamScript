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
import { asWorkRpc, RpcError, type ActionReceipt, type FinalizedContext, type RpcTransport, type SubmitWorkResult, type WorkRpc, type WorkStatusResult } from "./rpc.js";
import type { JamSigner } from "./signer.js";
import { blake2AsU8a } from "@polkadot/util-crypto";
import { verifyManagedStateProof } from "./proof.js";
import {
  RpcStateProvider,
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
};

export class JamScriptClient {
  private readonly rpc: WorkRpc;

  constructor(
    private readonly deployment: DeploymentDescriptor,
    transport: RpcTransport,
    private readonly options: JamScriptClientOptions = {},
  ) {
    if (deployment.abiVersion !== 1 || deployment.abi.abiVersion !== 1) {
      throw new Error("unsupported JamScript ABI version");
    }
    this.rpc = asWorkRpc(transport);
    this.stateProvider = options.stateProvider ?? new RpcStateProvider(transport);
  }

  private readonly stateProvider: StateProvider;

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
  ): Promise<SubmitWorkResult> {
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
    const nonce = await this.readNonce(signer.publicKey, initialContext);
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

    let context = initialContext;
    const retries = options.staleRetries ?? 1;
    for (let attempt = 0; ; attempt += 1) {
      try {
        const submitted = await this.rpc.submitWork({
          ...requestBase,
          context: {
            blockHash: context.blockHash,
            stateRoot: context.stateRoot,
            slot: context.slot,
          },
        });
        return { ...submitted, actionHash };
      } catch (error) {
        if (attempt >= retries || !isStaleContext(error)) throw error;
        context = await this.rpc.finalizedContext();
      }
    }
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
    return verifyManagedStateProof(root, key, response.value, response.proof);
  }

  workStatus(packageHash: string): Promise<WorkStatusResult> {
    return this.rpc.workStatus(packageHash);
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
        if (!(error instanceof RpcError) || error.code !== -32013) throw error;
      }
      if (Date.now() >= deadline) throw new Error("timed out waiting for finalized Work");
      await new Promise((resolve) => setTimeout(resolve, intervalMs));
    }
  }

  async waitForAction(
    packageHash: string,
    actionHash?: string,
    options: { intervalMs?: number; timeoutMs?: number } = {},
  ): Promise<WorkStatusResult & { actionReceipt: ActionReceipt }> {
    const work = await this.waitForWork(packageHash, options);
    if (work.status === "failed") {
      throw new RpcError("Work failed before an action receipt was produced", -32040, work);
    }
    const expected = actionHash?.toLowerCase();
    const receipts = work.actionReceipts ?? [];
    const receipt = expected === undefined && receipts.length === 1
      ? receipts[0]
      : receipts.find((item) => item.actionHash.toLowerCase() === expected);
    if (!receipt) {
      throw new Error("canonical action receipt is missing from the imported Work result");
    }
    return { ...work, actionReceipt: receipt };
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

function isStaleContext(error: unknown): boolean {
  return typeof error === "object" && error !== null && "code" in error && (error as { code: unknown }).code === -32010;
}
