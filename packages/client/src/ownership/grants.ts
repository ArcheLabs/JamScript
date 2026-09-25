import { ownershipKey, type Ownership } from "../crypto.js";

const GRANT_PREFIX = "__ownership/grants/v1/";

/**
 * Deterministic service-local grant key. The returned key is a convention;
 * applications store it in their own service state and do not share grants
 * across services.
 */
export function ownershipGrantStorageKey(subject: Ownership, controller: Ownership): Uint8Array {
  return new TextEncoder().encode(
    `${GRANT_PREFIX}${toHex(ownershipKey(subject))}/${toHex(ownershipKey(controller))}`,
  );
}

function toHex(bytes: Uint8Array): string {
  return Array.from(bytes, (byte) => byte.toString(16).padStart(2, "0")).join("");
}
