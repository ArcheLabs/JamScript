import { actionCommitmentV2, OWNERSHIP_KIND, parseHex, toHex, type Ownership } from "../crypto.js";
import type { JamScriptOwnershipSignRequest, OwnershipSigner } from "../signer.js";

export type Eip1193Provider = {
  request(args: { method: string; params?: readonly unknown[] | object }): Promise<unknown>;
};

export type EvmTypedData = {
  types: {
    EIP712Domain: readonly { name: string; type: string }[];
    JamScriptAction: readonly { name: string; type: string }[];
  };
  primaryType: "JamScriptAction";
  domain: { name: "JamScript"; version: "1"; salt: string };
  message: { commitment: string };
};

const EIP712_DOMAIN_TYPES = [
  { name: "name", type: "string" },
  { name: "version", type: "string" },
  { name: "salt", type: "bytes32" },
] as const;
const JAMSCRIPT_ACTION_TYPES = [{ name: "commitment", type: "bytes32" }] as const;

export function createEvmTypedData(request: JamScriptOwnershipSignRequest): EvmTypedData {
  return {
    types: { EIP712Domain: EIP712_DOMAIN_TYPES, JamScriptAction: JAMSCRIPT_ACTION_TYPES },
    primaryType: "JamScriptAction",
    domain: { name: "JamScript", version: "1", salt: toHex(request.networkDomain) },
    message: { commitment: toHex(actionCommitmentV2(request)) },
  };
}

export class EvmOwnershipSigner implements OwnershipSigner {
  private readonly ownership: Ownership;

  constructor(private readonly provider: Eip1193Provider, private readonly address: string) {
    let publicKey: Uint8Array;
    try { publicKey = parseHex(address, 20); }
    catch (cause) { throw new Error("EVM_ACCOUNT_INVALID", { cause }); }
    this.ownership = { version: 1, kind: OWNERSHIP_KIND.SECP256K1_KECCAK20, public: publicKey };
  }

  async getController(): Promise<Ownership> {
    return { ...this.ownership, public: this.ownership.public.slice() };
  }

  async signJamScriptAction(request: JamScriptOwnershipSignRequest): Promise<Uint8Array> {
    const typedData = createEvmTypedData(request);
    let result: unknown;
    try {
      result = await this.provider.request({ method: "eth_signTypedData_v4", params: [this.address, JSON.stringify(typedData)] });
    } catch (cause) {
      if (isUnsupportedTypedDataError(cause)) throw new Error("EVM_TYPED_DATA_UNSUPPORTED", { cause });
      throw cause;
    }
    if (typeof result !== "string") throw new Error("EVM_SIGNATURE_INVALID");
    const signature = parseSignature(result);
    if (![0, 1, 27, 28].includes(signature[64])) throw new Error("EVM_SIGNATURE_INVALID");
    return signature;
  }
}

function parseSignature(value: string): Uint8Array {
  try { return parseHex(value, 65); }
  catch (cause) { throw new Error("EVM_SIGNATURE_INVALID", { cause }); }
}

function isUnsupportedTypedDataError(error: unknown): boolean {
  if (!error || typeof error !== "object") return false;
  const value = error as { code?: unknown; message?: unknown };
  return value.code === -32601 || value.code === 4200 || (typeof value.message === "string" && /not supported|unsupported/i.test(value.message));
}
