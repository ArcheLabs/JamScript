import type { Ownership } from "../../crypto.js";
import { decodeMatrixControlClaimProofV1Bytes } from "./proof-runtime.js";
import type { MatrixControlClaimProofBytesV1 } from "./proof-runtime.js";
import type { OwnershipAdapter } from "../types.js";

export type Ed25519Verifier = (
  publicKey: Uint8Array,
  message: Uint8Array,
  signature: Uint8Array,
) => boolean;

/** Deterministic Matrix M→S→D authorization built on generic Ed25519 crypto. */
export function verifyMatrixOwnershipAuthorization(
  subject: Ownership,
  controller: Ownership,
  encodedProof: Uint8Array,
  verifyEd25519: Ed25519Verifier,
): boolean {
  if (!isEd25519Ownership(subject) || !isEd25519Ownership(controller)) return false;

  try {
    const proof = decodeMatrixControlClaimProofV1Bytes(encodedProof);
    if (!sameAdapterBytes(proof.deviceEd25519Key, controller.public)) return false;

    const selfSigningKey = base64Bytes(proof.selfSigningPublicKey);
    const selfSigningObject = concatBytes(
      staticAscii([123, 34, 107, 101, 121, 115, 34, 58, 123, 34, 101, 100, 50, 53, 53, 49, 57, 58]),
      selfSigningKey,
      staticAscii([34, 58, 34]),
      selfSigningKey,
      staticAscii([34, 125, 44, 34, 117, 115, 97, 103, 101, 34, 58, 91, 34, 115, 101, 108, 102, 95, 115, 105, 103, 110, 105, 110, 103, 34, 93, 44, 34, 117, 115, 101, 114, 95, 105, 100, 34, 58, 34]),
      proof.userId,
      staticAscii([34, 125]),
    );
    if (!verifyEd25519(subject.public, selfSigningObject, proof.masterSignature)) return false;

    const deviceObject = canonicalDeviceObject(proof);
    return verifyEd25519(proof.selfSigningPublicKey, deviceObject, proof.selfSigningSignature);
  } catch {
    return false;
  }
}

/** Factory form for Ownership SDK clients and adapter registries. */
export function createMatrixOwnershipAdapter(verifyEd25519: Ed25519Verifier): OwnershipAdapter {
  return {
    id: "matrix-cross-signing-v1",
    verify: (subject, controller, proof) =>
      verifyMatrixOwnershipAuthorization(subject, controller, proof, verifyEd25519),
  };
}

function canonicalDeviceObject(proof: MatrixControlClaimProofBytesV1): Uint8Array {
  const algorithms = proof.algorithms.slice(0, proof.algorithmsLength);
  const deviceId = proof.deviceId;
  return concatBytes(
    staticAscii([123, 34, 97, 108, 103, 111, 114, 105, 116, 104, 109, 115, 34, 58, 91]),
    algorithms,
    staticAscii([93, 44, 34, 100, 101, 118, 105, 99, 101, 95, 105, 100, 34, 58, 34]),
    deviceId,
    staticAscii([34, 44, 34, 107, 101, 121, 115, 34, 58, 123, 34, 99, 117, 114, 118, 101, 50, 53, 53, 49, 57, 58]),
    deviceId,
    staticAscii([34, 58, 34]),
    base64Bytes(proof.deviceCurve25519Key),
    staticAscii([34, 44, 34, 101, 100, 50, 53, 53, 49, 57, 58]),
    deviceId,
    staticAscii([34, 58, 34]),
    base64Bytes(proof.deviceEd25519Key),
    staticAscii([34, 125, 44, 34, 117, 115, 101, 114, 95, 105, 100, 34, 58, 34]),
    proof.userId,
    staticAscii([34, 125]),
  );
}

function isEd25519Ownership(value: Ownership): boolean {
  return value.version === 1 && value.kind === 0 && value.public.length === 32;
}

function sameAdapterBytes(left: Uint8Array, right: Uint8Array): boolean {
  if (left.length !== right.length) return false;
  for (let index = 0; index < left.length; index += 1) {
    if (left[index] !== right[index]) return false;
  }
  return true;
}

function staticAscii(values: number[]): Uint8Array {
  const output = new Uint8Array(values.length);
  for (let index = 0; index < values.length; index += 1) output[index] = values[index];
  return output;
}

function concatBytes(...parts: Uint8Array[]): Uint8Array {
  let total = 0;
  for (let index = 0; index < parts.length; index += 1) total += parts[index].length;
  const output = new Uint8Array(total);
  let offset = 0;
  for (let index = 0; index < parts.length; index += 1) {
    output.set(parts[index], offset);
    offset += parts[index].length;
  }
  return output;
}

function base64Bytes(bytes: Uint8Array): Uint8Array {
  const output = new Uint8Array(Math.floor((bytes.length * 4 + 2) / 3));
  let outputIndex = 0;
  for (let offset = 0; offset < bytes.length; offset += 3) {
    const hasSecond = offset + 1 < bytes.length;
    const hasThird = offset + 2 < bytes.length;
    const first = bytes[offset];
    const second = hasSecond ? bytes[offset + 1] : 0;
    const third = hasThird ? bytes[offset + 2] : 0;
    output[outputIndex++] = base64Ascii(first >>> 2);
    output[outputIndex++] = base64Ascii(((first & 3) << 4) | (second >>> 4));
    if (hasSecond) output[outputIndex++] = base64Ascii(((second & 15) << 2) | (third >>> 6));
    if (hasThird) output[outputIndex++] = base64Ascii(third & 63);
  }
  return output;
}

function base64Ascii(index: number): number {
  if (index < 26) return index + 65;
  if (index < 52) return index - 26 + 97;
  if (index < 62) return index - 52 + 48;
  if (index === 62) return 43;
  return 47;
}
