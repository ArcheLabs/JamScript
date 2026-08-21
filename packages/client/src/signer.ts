export interface JamSigner {
  readonly publicKey: Uint8Array;
  signRaw(message: Uint8Array): Promise<Uint8Array>;
}

export type InjectedSigner = {
  signRaw(input: {
    address: string;
    data: string;
    type: "bytes";
  }): Promise<{ signature: string }>;
};

export class PolkadotExtensionSigner implements JamSigner {
  constructor(
    public readonly publicKey: Uint8Array,
    private readonly address: string,
    private readonly injector: InjectedSigner,
  ) {
    if (publicKey.length !== 32) throw new Error("sr25519 public key must be 32 bytes");
  }

  async signRaw(message: Uint8Array): Promise<Uint8Array> {
    const result = await this.injector.signRaw({
      address: this.address,
      data: bytesToHex(message),
      type: "bytes",
    });
    return hexToBytes(result.signature);
  }
}

function bytesToHex(bytes: Uint8Array): string {
  return "0x" + Array.from(bytes, (value) => value.toString(16).padStart(2, "0")).join("");
}

function hexToBytes(value: string): Uint8Array {
  const hex = value.startsWith("0x") ? value.slice(2) : value;
  if (hex.length % 2 !== 0) throw new Error("signature hex has odd length");
  const output = new Uint8Array(hex.length / 2);
  for (let index = 0; index < output.length; index += 1) {
    output[index] = Number.parseInt(hex.slice(index * 2, index * 2 + 2), 16);
  }
  return output;
}
