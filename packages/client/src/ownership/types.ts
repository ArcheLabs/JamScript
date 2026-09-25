import type { Ownership } from "../crypto.js";
import type { OwnershipSigner } from "../signer.js";

/** Provider-neutral proof envelope used by Ownership adapters. */
export type OwnershipAuthorization = {
  adapterId: string;
  proof: Uint8Array;
};

/** Stable identity, its active signing controller, and optional adapter proof. */
export type OwnershipSession = {
  subject: Ownership;
  signer: OwnershipSigner;
  authorization?: OwnershipAuthorization;
};

/** Provider-specific proof semantics stay outside JamScript Core. */
export interface OwnershipAdapter {
  readonly id: string;
  verify(subject: Ownership, controller: Ownership, proof: Uint8Array): boolean;
}
