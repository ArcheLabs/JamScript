import { decodeAddress } from "@polkadot/util-crypto";
import { OWNERSHIP_KIND, type Ownership } from "../crypto.js";
import type { JamScriptOwnershipSignRequest, OwnershipSigner } from "../signer.js";

export type PolkadotSignatureScheme = "ed25519" | "sr25519" | "ecdsa";
export type PolkadotInjectedSigner = { signRaw(input: { address: string; data: string; type: "bytes" }): Promise<{ signature: string }> };

export function decodePolkadotAccountId(address: string): Uint8Array {
  try {
    const accountId = decodeAddress(address);
    if (accountId.length !== 32) throw new Error("decoded account is not AccountId32");
    return Uint8Array.from(accountId);
  } catch (cause) { throw new Error("POLKADOT_ACCOUNT_INVALID", { cause }); }
}

const SCHEME_BYTES: Record<PolkadotSignatureScheme, number> = { ed25519: 0, sr25519: 1, ecdsa: 2 };
const SIGNATURE_LENGTHS: Record<PolkadotSignatureScheme, number> = { ed25519: 64, sr25519: 64, ecdsa: 65 };

export class PolkadotOwnershipSigner implements OwnershipSigner {
  private readonly ownership: Ownership;
  constructor(private readonly options: { accountId: Uint8Array; address: string; scheme: PolkadotSignatureScheme; signer: PolkadotInjectedSigner }) {
    if (options.accountId.length !== 32) throw new Error("POLKADOT_ACCOUNT_INVALID");
    if (!Object.hasOwn(SCHEME_BYTES, options.scheme)) throw new Error("POLKADOT_SCHEME_UNSUPPORTED");
    if (options.address.length === 0) throw new Error("POLKADOT_ACCOUNT_INVALID");
    this.ownership = { version: 1, kind: OWNERSHIP_KIND.MULTICRYPTO_ACCOUNT32, public: options.accountId.slice() };
  }
  async getController(): Promise<Ownership> { return { ...this.ownership, public: this.ownership.public.slice() }; }
  async signJamScriptAction(request: JamScriptOwnershipSignRequest): Promise<Uint8Array> {
    const result = await this.options.signer.signRaw({ address: this.options.address, data: bytesToHex(request.message), type: "bytes" });
    let signature: Uint8Array;
    try { signature = hexToBytes(result.signature); }
    catch (cause) { throw new Error("POLKADOT_SIGNATURE_INVALID", { cause }); }
    if (signature.length !== SIGNATURE_LENGTHS[this.options.scheme]) throw new Error("POLKADOT_SIGNATURE_INVALID");
    return Uint8Array.of(SCHEME_BYTES[this.options.scheme], ...signature);
  }
}

function bytesToHex(bytes: Uint8Array): string { return "0x" + Array.from(bytes, (value) => value.toString(16).padStart(2, "0")).join(""); }
function hexToBytes(value: string): Uint8Array {
  const hex = value.startsWith("0x") ? value.slice(2) : value;
  if (hex.length % 2 !== 0 || !/^[0-9a-f]+$/i.test(hex)) throw new Error("invalid signature hex");
  return Uint8Array.from(hex.match(/../g) ?? [], (pair) => Number.parseInt(pair, 16));
}
