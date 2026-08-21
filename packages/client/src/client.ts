import { actionByName, queryByName, stateByName, type DeploymentDescriptor } from "./abi.js";
import { decodeStateValue, decodeValue, encodeActionPayload, type CodecValue } from "./codec.js";
import {
  actionSelector,
  encodeSignedAction,
  NONCE_SCHEMA_V1,
  parseHex,
  signingDigest,
  stateKey,
  toHex,
  type SignedAction,
} from "./crypto.js";
import { asWorkRpc, RpcError, type FinalizedContext, type RpcTransport, type SubmitWorkResult, type WorkRpc, type WorkStatusResult } from "./rpc.js";
import type { JamSigner } from "./signer.js";
import { blake2AsU8a } from "@polkadot/util-crypto";

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
    const key = stateKey(this.deployment.serviceId, NONCE_SCHEMA_V1, publicKey);
    const encoded = await this.rpc.serviceStorageAt(
      finalized.blockHash,
      this.deployment.serviceId,
      toHex(key),
    );
    if (encoded === null) return 0n;
    const value = decodeValue("u64", decodeStateValue(parseHex(encoded)));
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
    const unsigned: Omit<SignedAction, "signature"> = {
      version: 1,
      genesisHash: parseHex(this.deployment.genesisHash, 32),
      serviceId: this.deployment.serviceId,
      actionSelector: selector,
      signerScheme: 0,
      publicKey: signer.publicKey,
      nonce,
      validUntil,
      payloadHash: blake2(payload),
      payload,
    };
    const signature = await signer.signRaw(signingDigest(unsigned));
    if (signature.length !== 64) throw new Error("sr25519 signRaw must return a 64-byte signature");
    const signed = encodeSignedAction({ ...unsigned, signature });
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
      throw new Error("M4 declarative queries require a 32-byte address key");
    }
    const context = await this.rpc.finalizedContext();
    const encoded = await this.rpc.serviceStorageAt(
      context.blockHash,
      this.deployment.serviceId,
      toHex(stateKey(this.deployment.serviceId, state.schema, key)),
    );
    return {
      value:
        encoded === null
          ? null
          : decodeValue(query.output.type, decodeStateValue(parseHex(encoded))),
      context,
    };
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

function sameHex(left: string, right: string): boolean {
  return left.toLowerCase().replace(/^0x/, "") === right.toLowerCase().replace(/^0x/, "");
}

function isStaleContext(error: unknown): boolean {
  return typeof error === "object" && error !== null && "code" in error && (error as { code: unknown }).code === -32010;
}
