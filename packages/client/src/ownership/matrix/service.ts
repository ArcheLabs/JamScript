import type { Ownership } from "../../crypto.js";
import { matrixOwnershipVerificationPayload } from "./service-payload.js";
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
  const payload = matrixOwnershipVerificationPayload(subject, controller, encodedProof);
  if (payload.length < 4 + 64 + 32 + 64) return false;
  const masterMessageLength = matrixNodeReadU16(payload, 0);
  const deviceMessageLength = matrixNodeReadU16(payload, 2);
  if (payload.length !== 4 + masterMessageLength + 64 + 32 + deviceMessageLength + 64) return false;
  let offset = 4;
  const masterMessage = payload.slice(offset, offset + masterMessageLength); offset += masterMessageLength;
  const masterSignature = payload.slice(offset, offset + 64); offset += 64;
  const selfSigningKey = payload.slice(offset, offset + 32); offset += 32;
  const deviceMessage = payload.slice(offset, offset + deviceMessageLength); offset += deviceMessageLength;
  const deviceSignature = payload.slice(offset, offset + 64);
  return verifyEd25519(subject.public, masterMessage, masterSignature)
    && verifyEd25519(selfSigningKey, deviceMessage, deviceSignature);
}

/** Factory form for Ownership SDK clients and adapter registries. */
export function createMatrixOwnershipAdapter(verifyEd25519: Ed25519Verifier): OwnershipAdapter {
  return {
    id: "matrix-cross-signing-v1",
    verify: (subject, controller, proof) =>
      verifyMatrixOwnershipAuthorization(subject, controller, proof, verifyEd25519),
  };
}

function matrixNodeReadU16(value: Uint8Array, offset: number): number {
  return value[offset] | (value[offset + 1] << 8);
}
