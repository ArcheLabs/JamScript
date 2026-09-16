import { OWNERSHIP_KIND, type Ownership } from "./crypto.js";
import type { JamScriptOwnershipSignRequest, OwnershipSigner } from "./signer.js";

export type MatrixKeyDiscovery = {
  queryMasterPublicKey(userId: string): Promise<Uint8Array>;
};

export class MatrixOwnershipResolver {
  constructor(private readonly discovery: MatrixKeyDiscovery) {}

  async resolve(userId: string): Promise<Ownership> {
    if (!/^@[^\s:]+:[^\s:]+$/.test(userId)) throw new Error("invalid Matrix User ID");
    const publicKey = await this.discovery.queryMasterPublicKey(userId);
    if (publicKey.length !== 32) throw new Error("Matrix master key must be 32 bytes");
    return { version: 1, kind: OWNERSHIP_KIND.ED25519_KEY, public: publicKey };
  }
}

export type MatrixDeviceSigning = {
  sign(message: Uint8Array): Promise<Uint8Array>;
};

export class MatrixDeviceController implements OwnershipSigner {
  private readonly ownership: Ownership;

  constructor(deviceEd25519PublicKey: Uint8Array, private readonly signing: MatrixDeviceSigning) {
    if (deviceEd25519PublicKey.length !== 32) throw new Error("Matrix device key must be 32 bytes");
    this.ownership = { version: 1, kind: OWNERSHIP_KIND.ED25519_KEY, public: deviceEd25519PublicKey };
  }

  async getController(): Promise<Ownership> {
    return this.ownership;
  }

  signJamScriptAction(request: JamScriptOwnershipSignRequest): Promise<Uint8Array> {
    return this.signing.sign(request.message);
  }
}

export type MatrixControlClaimBootstrapper = {
  bootstrap(subject: Ownership, controller: Ownership, proof: Uint8Array): Promise<void>;
};
