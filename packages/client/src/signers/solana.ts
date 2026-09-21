import { SolanaSignMessage } from "@solana/wallet-standard-features";
import type { SolanaSignMessageFeature } from "@solana/wallet-standard-features";
import type { WalletAccount } from "@wallet-standard/base";
import { OWNERSHIP_KIND, type Ownership } from "../crypto.js";
import type { JamScriptOwnershipSignRequest, OwnershipSigner } from "../signer.js";

export class SolanaOwnershipSigner implements OwnershipSigner {
  private readonly ownership: Ownership;
  constructor(private readonly account: WalletAccount, private readonly signMessage: SolanaSignMessageFeature) {
    if (account.publicKey.length !== 32) throw new Error("SOLANA_PUBLIC_KEY_INVALID");
    this.ownership = { version: 1, kind: OWNERSHIP_KIND.ED25519_KEY, public: Uint8Array.from(account.publicKey) };
  }
  async getController(): Promise<Ownership> { return { ...this.ownership, public: this.ownership.public.slice() }; }
  async signJamScriptAction(request: JamScriptOwnershipSignRequest): Promise<Uint8Array> {
    const feature = this.signMessage?.[SolanaSignMessage];
    if (!feature || typeof feature.signMessage !== "function") throw new Error("SOLANA_SIGN_MESSAGE_UNSUPPORTED");
    const [output] = await feature.signMessage({ account: this.account, message: request.message });
    if (!output || !sameBytes(output.signedMessage, request.message) || output.signature.length !== 64 || (output.signatureType !== undefined && output.signatureType !== "ed25519")) throw new Error("SOLANA_SIGNATURE_INVALID");
    return Uint8Array.from(output.signature);
  }
}
function sameBytes(left: Uint8Array, right: Uint8Array): boolean { return left.length === right.length && left.every((value, index) => value === right[index]); }
