import { OWNERSHIP_KIND, type Ownership } from "./crypto.js";
import type { JamScriptOwnershipSignRequest, OwnershipSigner } from "./signer.js";

const MATRIX_PROOF_VERSION_V1 = 1;
const MAX_MATRIX_TEXT_BYTES = 255;
const MAX_MATRIX_ALGORITHMS = 8;
const MAX_MATRIX_ALGORITHM_BYTES = 128;

export type MatrixControlClaimProofV1 = {
  userId: string;
  selfSigningPublicKey: Uint8Array;
  masterSignature: Uint8Array;
  deviceId: string;
  algorithms: string[];
  deviceCurve25519Key: Uint8Array;
  deviceEd25519Key: Uint8Array;
  selfSigningSignature: Uint8Array;
};

function matrixText(value: string, limit: number, label: string): Uint8Array {
  if (value.length > limit || !/^[\x20-\x7e]*$/.test(value) || value.includes('"') || value.includes("\\")) {
    throw new Error(`invalid Matrix ${label}`);
  }
  return new TextEncoder().encode(value);
}

function matrixBytes(value: Uint8Array, length: number, label: string): Uint8Array {
  if (value.length !== length) throw new Error(`Matrix ${label} must be ${length} bytes`);
  return value.slice();
}

function u16(value: number): Uint8Array {
  return Uint8Array.of(value & 0xff, (value >>> 8) & 0xff);
}

function concat(...parts: Uint8Array[]): Uint8Array {
  const output = new Uint8Array(parts.reduce((size, part) => size + part.length, 0));
  let offset = 0;
  for (const part of parts) {
    output.set(part, offset);
    offset += part.length;
  }
  return output;
}

function proofText(value: string, limit: number, label: string): Uint8Array {
  const encoded = matrixText(value, limit, label);
  if (encoded.length > 0xffff) throw new Error(`Matrix ${label} is too long`);
  return concat(u16(encoded.length), encoded);
}

/** Encode the exact MatrixControlClaimProofV1 wire format used by Rust. */
export function encodeMatrixControlClaimProofV1(proof: MatrixControlClaimProofV1): Uint8Array {
  if (proof.algorithms.length > MAX_MATRIX_ALGORITHMS) throw new Error("too many Matrix algorithms");
  return concat(
    Uint8Array.of(MATRIX_PROOF_VERSION_V1),
    proofText(proof.userId, MAX_MATRIX_TEXT_BYTES, "user id"),
    matrixBytes(proof.selfSigningPublicKey, 32, "self-signing key"),
    matrixBytes(proof.masterSignature, 64, "master signature"),
    proofText(proof.deviceId, MAX_MATRIX_TEXT_BYTES, "device id"),
    Uint8Array.of(proof.algorithms.length),
    ...proof.algorithms.map((algorithm) => proofText(algorithm, MAX_MATRIX_ALGORITHM_BYTES, "algorithm")),
    matrixBytes(proof.deviceCurve25519Key, 32, "device curve25519 key"),
    matrixBytes(proof.deviceEd25519Key, 32, "device ed25519 key"),
    matrixBytes(proof.selfSigningSignature, 64, "self-signing signature"),
  );
}

class MatrixProofReader {
  private offset = 0;
  constructor(private readonly bytes: Uint8Array) {}

  take(length: number): Uint8Array {
    const end = this.offset + length;
    if (end > this.bytes.length) throw new Error("truncated Matrix proof");
    const result = this.bytes.slice(this.offset, end);
    this.offset = end;
    return result;
  }

  u8(): number { return this.take(1)[0]; }

  text(limit: number, label: string): string {
    const lengthPrefix = this.take(2);
    const length = lengthPrefix[0] | (lengthPrefix[1] << 8);
    if (length > limit) throw new Error(`invalid Matrix ${label}`);
    const value = new TextDecoder().decode(this.take(length));
    matrixText(value, limit, label);
    return value;
  }

  done(): boolean { return this.offset === this.bytes.length; }
}

/** Decode and validate the exact MatrixControlClaimProofV1 wire format used by Rust. */
export function decodeMatrixControlClaimProofV1(bytes: Uint8Array): MatrixControlClaimProofV1 {
  const reader = new MatrixProofReader(bytes);
  if (reader.u8() !== MATRIX_PROOF_VERSION_V1) throw new Error("unsupported Matrix proof version");
  const userId = reader.text(MAX_MATRIX_TEXT_BYTES, "user id");
  const selfSigningPublicKey = reader.take(32);
  const masterSignature = reader.take(64);
  const deviceId = reader.text(MAX_MATRIX_TEXT_BYTES, "device id");
  const algorithmCount = reader.u8();
  if (algorithmCount > MAX_MATRIX_ALGORITHMS) throw new Error("too many Matrix algorithms");
  const algorithms = Array.from({ length: algorithmCount }, () => reader.text(MAX_MATRIX_ALGORITHM_BYTES, "algorithm"));
  const deviceCurve25519Key = reader.take(32);
  const deviceEd25519Key = reader.take(32);
  const selfSigningSignature = reader.take(64);
  if (!reader.done()) throw new Error("trailing Matrix proof bytes");
  return {
    userId,
    selfSigningPublicKey,
    masterSignature,
    deviceId,
    algorithms,
    deviceCurve25519Key,
    deviceEd25519Key,
    selfSigningSignature,
  };
}

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
