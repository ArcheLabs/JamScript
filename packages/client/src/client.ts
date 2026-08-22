import { actionByName, queryByName, stateByName, type DeploymentDescriptor } from "./abi.js";
import { decodeStateValue, decodeValue, encodeActionPayload, type CodecValue } from "./codec.js";
import {
  actionSelector,
  encodeSignedActionV2,
  MANAGED_STATE_COMMITMENT_KEY_V1,
  nonceKey,
  parseHex,
  signingDigestV2,
  stateKey,
  toHex,
  type SignedActionV2,
} from "./crypto.js";
import { asWorkRpc, RpcError, type FinalizedContext, type RpcTransport, type SubmitWorkResult, type WorkRpc, type WorkStatusResult } from "./rpc.js";
import type { JamSigner } from "./signer.js";
import { blake2AsU8a } from "@polkadot/util-crypto";

const EMPTY_STATE_ROOT_V1 = "0x03170a2e7597b7b7e3d84c05391d139a62b157e78786d8c082f29dcf4c111314";

export type QueryResult = {
  value: CodecValue | null;
  context: FinalizedContext;
};

export class JamScriptClient {
  private readonly rpc: WorkRpc;

  constructor(
    private readonly deployment: DeploymentDescriptor,
    transport: RpcTransport,
  ) {
    if (deployment.abiVersion !== 1 || deployment.abi.abiVersion !== 1) {
      throw new Error("unsupported JamScript ABI version");
    }
    this.rpc = asWorkRpc(transport);
  }

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
    const valueBytes = await this.readManagedValue(finalized, root, key);
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
    const unsigned: Omit<SignedActionV2, "signature"> = {
      version: 2,
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
    const signature = await signer.signRaw(signingDigestV2(unsigned));
    if (signature.length !== 64) throw new Error("sr25519 signRaw must return a 64-byte signature");
    const signed = encodeSignedActionV2({ ...unsigned, signature });
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
        return await this.rpc.submitWork({
          ...requestBase,
          context: {
            blockHash: context.blockHash,
            stateRoot: context.stateRoot,
            slot: context.slot,
          },
        });
      } catch (error) {
        if (attempt >= retries || !isStaleContext(error)) throw error;
        context = await this.rpc.finalizedContext();
      }
    }
  }

  async query(queryName: string, key: Uint8Array): Promise<QueryResult> {
    const query = queryByName(this.deployment.abi, queryName);
    const state = stateByName(this.deployment.abi, query.state);
    if (query.keyType !== "address" || state.keyType !== "address" || key.length !== 32) {
      throw new Error("managed-state queries require a 32-byte address key");
    }
    const context = await this.rpc.finalizedContext();
    const root = await this.managedStateRoot(context);
    const valueBytes = await this.readManagedValue(
      context,
      root,
      stateKey(state.schema, key),
    );
    return {
      value: valueBytes === null
        ? null
        : decodeValue(query.output.type, valueBytes),
      context,
    };
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
    context: FinalizedContext,
    root: Uint8Array,
    key: Uint8Array,
  ): Promise<Uint8Array | null> {
    try {
      const response = await this.rpc.managedStateAt(
        this.deployment.serviceId,
        toHex(root),
        toBase64(key),
      );
      if (
        response.serviceId !== this.deployment.serviceId
        || response.stateRoot.toLowerCase() !== toHex(root).toLowerCase()
        || response.keyBase64 !== toBase64(key)
      ) {
        throw new Error("managed-state provider response does not match the requested query");
      }
      return response.valueBase64 === null ? null : fromBase64(response.valueBase64);
    } catch (error) {
      if (!isManagedStateUnavailable(error)) throw error;
      const encoded = await this.rpc.serviceStorageAt(
        context.blockHash,
        this.deployment.serviceId,
        toHex(key),
      );
      return encoded === null ? null : decodeStateValue(parseHex(encoded));
    }
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

function sameHex(left: string, right: string): boolean {
  return left.toLowerCase().replace(/^0x/, "") === right.toLowerCase().replace(/^0x/, "");
}

function isStaleContext(error: unknown): boolean {
  return typeof error === "object" && error !== null && "code" in error && (error as { code: unknown }).code === -32010;
}

function isManagedStateUnavailable(error: unknown): boolean {
  return error instanceof RpcError && (error.code === -32601 || error.code === -32030);
}
