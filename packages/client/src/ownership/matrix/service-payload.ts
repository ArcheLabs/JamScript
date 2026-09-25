// @ts-nocheck
import { decodeMatrixControlClaimProofV1Bytes } from "./proof-runtime.js";

/**
 * Build the two canonical signed objects used by the Matrix Ownership adapter.
 * The packed return value keeps this boundary usable by ScriptC without passing
 * a verifier callback (function values are not part of JamScript's crypto ABI).
 *
 * Layout: u16 masterMessageLength, u16 deviceMessageLength, masterMessage,
 * masterSignature[64], selfSigningPublicKey[32], deviceMessage,
 * selfSigningSignature[64]. An empty result means malformed or mismatched input.
 */
export function matrixOwnershipVerificationPayload(
  subject: JamOwnership,
  controller: JamOwnership,
  encodedProof: Uint8Array,
): Uint8Array {
  if (!isEd25519Ownership(subject) || !isEd25519Ownership(controller)) return new Uint8Array(0);

  try {
    const proof = decodeMatrixControlClaimProofV1Bytes(encodedProof);
    const userLength = matrixPayloadReadU16(proof, 0);
    const deviceLength = matrixPayloadReadU16(proof, 2);
    const algorithmsLength = matrixPayloadReadU16(proof, 4);
    let offset = 6;
    const userId = proof.slice(offset, offset + userLength); offset += userLength;
    const selfSigningKey = proof.slice(offset, offset + 32); offset += 32;
    const masterSignature = proof.slice(offset, offset + 64); offset += 64;
    const deviceId = proof.slice(offset, offset + deviceLength); offset += deviceLength;
    const algorithms = proof.slice(offset, offset + algorithmsLength); offset += algorithmsLength;
    const deviceCurve25519Key = proof.slice(offset, offset + 32); offset += 32;
    const deviceEd25519Key = proof.slice(offset, offset + 32); offset += 32;
    const selfSigningSignature = proof.slice(offset, offset + 64);
    if (!matrixSameBytes(deviceEd25519Key, controller.public)) return new Uint8Array(0);

    const encodedSelfSigningKey = base64Bytes(selfSigningKey);
    const masterMessage = concatBytes(
      staticAscii([123, 34, 107, 101, 121, 115, 34, 58, 123, 34, 101, 100, 50, 53, 53, 49, 57, 58]),
      encodedSelfSigningKey,
      staticAscii([34, 58, 34]),
      encodedSelfSigningKey,
      staticAscii([34, 125, 44, 34, 117, 115, 97, 103, 101, 34, 58, 91, 34, 115, 101, 108, 102, 95, 115, 105, 103, 110, 105, 110, 103, 34, 93, 44, 34, 117, 115, 101, 114, 95, 105, 100, 34, 58, 34]),
      userId,
      staticAscii([34, 125]),
    );
    const deviceMessage = concatBytes(
      staticAscii([123, 34, 97, 108, 103, 111, 114, 105, 116, 104, 109, 115, 34, 58, 91]),
      algorithms,
      staticAscii([93, 44, 34, 100, 101, 118, 105, 99, 101, 95, 105, 100, 34, 58, 34]),
      deviceId,
      staticAscii([34, 44, 34, 107, 101, 121, 115, 34, 58, 123, 34, 99, 117, 114, 118, 101, 50, 53, 53, 49, 57, 58]),
      deviceId,
      staticAscii([34, 58, 34]),
      base64Bytes(deviceCurve25519Key),
      staticAscii([34, 44, 34, 101, 100, 50, 53, 53, 49, 57, 58]),
      deviceId,
      staticAscii([34, 58, 34]),
      base64Bytes(deviceEd25519Key),
      staticAscii([34, 125, 44, 34, 117, 115, 101, 114, 95, 105, 100, 34, 58, 34]),
      userId,
      staticAscii([34, 125]),
    );
    return packVerificationPayload(masterMessage, masterSignature, selfSigningKey, deviceMessage, selfSigningSignature);
  } catch {
    return new Uint8Array(0);
  }
}

function packVerificationPayload(
  masterMessage: Uint8Array,
  masterSignature: Uint8Array,
  selfSigningKey: Uint8Array,
  deviceMessage: Uint8Array,
  deviceSignature: Uint8Array,
): Uint8Array {
  const output = new Uint8Array(4 + masterMessage.length + 64 + 32 + deviceMessage.length + 64);
  writeU16(output, 0, masterMessage.length);
  writeU16(output, 2, deviceMessage.length);
  let offset = 4;
  output.set(masterMessage, offset); offset += masterMessage.length;
  output.set(masterSignature, offset); offset += 64;
  output.set(selfSigningKey, offset); offset += 32;
  output.set(deviceMessage, offset); offset += deviceMessage.length;
  output.set(deviceSignature, offset);
  return output;
}

function writeU16(target: Uint8Array, offset: number, value: number): void {
  target[offset] = value & 0xff;
  target[offset + 1] = value >>> 8;
}

function matrixPayloadReadU16(value: Uint8Array, offset: number): number {
  return value[offset] | (value[offset + 1] << 8);
}

function isEd25519Ownership(value: JamOwnership): boolean {
  return value.version === 1 && value.kind === 0 && value.public.length === 32;
}

function matrixSameBytes(left: Uint8Array, right: Uint8Array): boolean {
  if (left.length !== right.length) return false;
  for (let index = 0; index < left.length; index += 1) if (left[index] !== right[index]) return false;
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
