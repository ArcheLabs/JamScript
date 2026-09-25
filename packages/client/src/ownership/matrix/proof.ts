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

export type MatrixControlClaimProofInputV1 = MatrixControlClaimProofV1;

/** Build a validated, wire-compatible Matrix M→S→D proof from discovered keys. */
export function createMatrixControlClaimProofV1(input: MatrixControlClaimProofInputV1): MatrixControlClaimProofV1 {
  const proof: MatrixControlClaimProofV1 = {
    userId: input.userId,
    selfSigningPublicKey: input.selfSigningPublicKey.slice(),
    masterSignature: input.masterSignature.slice(),
    deviceId: input.deviceId,
    algorithms: [...input.algorithms],
    deviceCurve25519Key: input.deviceCurve25519Key.slice(),
    deviceEd25519Key: input.deviceEd25519Key.slice(),
    selfSigningSignature: input.selfSigningSignature.slice(),
  };
  encodeMatrixControlClaimProofV1(proof);
  return proof;
}

function matrixText(value: string, limit: number, label: string): Uint8Array {
  if (value.length > limit || !/^[\x20-\x7e]*$/.test(value) || value.includes('"') || value.includes("\\")) {
    throw new Error("invalid Matrix " + label);
  }
  const output = new Uint8Array(value.length);
  for (let index = 0; index < value.length; index += 1) output[index] = value.charCodeAt(index);
  return output;
}

function matrixBytes(value: Uint8Array, length: number, label: string): Uint8Array {
  if (value.length !== length) throw new Error("Matrix " + label + " must be " + length + " bytes");
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
  if (proof.algorithms.length > 8) throw new Error("too many Matrix algorithms");
  return concat(
    Uint8Array.of(1),
    proofText(proof.userId, 255, "user id"),
    matrixBytes(proof.selfSigningPublicKey, 32, "self-signing key"),
    matrixBytes(proof.masterSignature, 64, "master signature"),
    proofText(proof.deviceId, 255, "device id"),
    Uint8Array.of(proof.algorithms.length),
    ...proof.algorithms.map((algorithm) => proofText(algorithm, 128, "algorithm")),
    matrixBytes(proof.deviceCurve25519Key, 32, "device curve25519 key"),
    matrixBytes(proof.deviceEd25519Key, 32, "device ed25519 key"),
    matrixBytes(proof.selfSigningSignature, 64, "self-signing signature"),
  );
}

/** Decode and validate the exact MatrixControlClaimProofV1 wire format used by Rust. */
export function decodeMatrixControlClaimProofV1(bytes: Uint8Array): MatrixControlClaimProofV1 {
  const decoded = decodeMatrixControlClaimProofV1Bytes(bytes);
  const userLength = decoded[0] | (decoded[1] << 8);
  const deviceLength = decoded[2] | (decoded[3] << 8);
  const algorithmsLength = decoded[4] | (decoded[5] << 8);
  let offset = 6;
  const userId = decoded.slice(offset, offset + userLength); offset += userLength;
  const selfSigningPublicKey = decoded.slice(offset, offset + 32); offset += 32;
  const masterSignature = decoded.slice(offset, offset + 64); offset += 64;
  const deviceId = decoded.slice(offset, offset + deviceLength); offset += deviceLength;
  const algorithms = decoded.slice(offset, offset + algorithmsLength); offset += algorithmsLength;
  const deviceCurve25519Key = decoded.slice(offset, offset + 32); offset += 32;
  const deviceEd25519Key = decoded.slice(offset, offset + 32); offset += 32;
  const selfSigningSignature = decoded.slice(offset, offset + 64);
  return {
    userId: asciiString(userId),
    selfSigningPublicKey,
    masterSignature,
    deviceId: asciiString(deviceId),
    algorithms: decodeAlgorithms(algorithms, algorithms.length),
    deviceCurve25519Key,
    deviceEd25519Key,
    selfSigningSignature,
  };
}

function asciiString(bytes: Uint8Array): string {
  let value = "";
  for (let index = 0; index < bytes.length; index += 1) value += String.fromCharCode(bytes[index]);
  return value;
}

function decodeAlgorithms(bytes: Uint8Array, length: number): string[] {
  const result: string[] = [];
  let inString = false;
  let start = 0;
  for (let index = 0; index < length; index += 1) {
    if (bytes[index] === 34) {
      if (inString) result.push(asciiString(bytes.slice(start, index)));
      else start = index + 1;
      inString = !inString;
    }
  }
  return result;
}
import { decodeMatrixControlClaimProofV1Bytes } from "./proof-runtime.js";
