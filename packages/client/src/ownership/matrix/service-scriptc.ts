// ScriptC-only entry. The import is removed by Locus's service bundler, leaving
// direct calls to the generic JamScript runtime primitive in the combined unit.
// @ts-nocheck
import { verifyEd25519 } from "jam";
import { matrixOwnershipVerificationPayload } from "./service-payload.js";

export function verifyMatrixOwnershipAuthorizationScriptc(
  subject: JamOwnership,
  controller: JamOwnership,
  proof: Uint8Array,
): boolean {
  const payload = matrixOwnershipVerificationPayload(subject, controller, proof);
  if (payload.length < 4 + 64 + 32 + 64) return false;
  const masterMessageLength = matrixScriptcReadU16(payload, 0);
  const deviceMessageLength = matrixScriptcReadU16(payload, 2);
  const expectedLength = 4 + masterMessageLength + 64 + 32 + deviceMessageLength + 64;
  if (payload.length !== expectedLength) return false;

  let offset = 4;
  const masterMessage = payload.slice(offset, offset + masterMessageLength); offset += masterMessageLength;
  const masterSignature = payload.slice(offset, offset + 64); offset += 64;
  const selfSigningKey = payload.slice(offset, offset + 32); offset += 32;
  const deviceMessage = payload.slice(offset, offset + deviceMessageLength); offset += deviceMessageLength;
  const deviceSignature = payload.slice(offset, offset + 64);
  if (!verifyEd25519(subject.public, masterMessage, masterSignature)) return false;
  return verifyEd25519(selfSigningKey, deviceMessage, deviceSignature);
}

function matrixScriptcReadU16(value: Uint8Array, offset: number): number {
  return value[offset] | (value[offset + 1] << 8);
}
