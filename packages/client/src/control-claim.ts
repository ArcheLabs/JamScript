import { encodeOwnership, type Ownership } from "./crypto.js";
import {
  encodeMatrixControlBootstrapV1,
  type MatrixControlBootstrapV1,
  type MatrixControlClaimProofV1,
} from "./matrix.js";
import type { SubmitActionResult } from "./rpc.js";
import type { OwnershipSigner } from "./signer.js";

/** Deployment identity for the dedicated network-scoped Ownership Control service. */
export type ControlClaimDeployment = {
  genesisHash: string;
  networkDomain: string;
  serviceKey: string;
  serviceId: number;
  codeHash: string;
};

export type MatrixControlBootstrapSigner = OwnershipSigner & {
  signBootstrapMessage(message: Uint8Array): Promise<Uint8Array>;
};

export type MatrixControlBootstrapInput = {
  subject: Ownership;
  controllerSigner: MatrixControlBootstrapSigner;
  proof: MatrixControlClaimProofV1 | Uint8Array;
};

export const CONTROL_CLAIM_ACTIONS = {
  bootstrapMatrixController: "bootstrapMatrixController",
  addController: "addController",
  revokeController: "revokeController",
} as const;

export function encodeControlClaimActionV1(
  operation: "add" | "revoke",
  subject: Ownership,
  controller: Ownership,
): Uint8Array {
  const subjectBytes = encodeOwnership(subject);
  const controllerBytes = encodeOwnership(controller);
  const output = new Uint8Array(2 + 2 + subjectBytes.length + 2 + controllerBytes.length);
  let offset = 0;
  output[offset++] = 1;
  output[offset++] = operation === "add" ? 0 : 1;
  output.set(Uint8Array.of(subjectBytes.length & 0xff, subjectBytes.length >>> 8), offset);
  offset += 2;
  output.set(subjectBytes, offset);
  offset += subjectBytes.length;
  output.set(Uint8Array.of(controllerBytes.length & 0xff, controllerBytes.length >>> 8), offset);
  offset += 2;
  output.set(controllerBytes, offset);
  return output;
}

export type ControlClaimClientSurface = {
  bootstrapMatrixControlClaim(input: MatrixControlBootstrapInput & { deployment: ControlClaimDeployment }): Promise<SubmitActionResult>;
  addController(input: { deployment: ControlClaimDeployment; subject: Ownership; controller: Ownership; signer: OwnershipSigner }): Promise<SubmitActionResult>;
  revokeController(input: { deployment: ControlClaimDeployment; subject: Ownership; controller: Ownership; signer: OwnershipSigner }): Promise<SubmitActionResult>;
  isControllerActive(deployment: ControlClaimDeployment, subject: Ownership, controller: Ownership): Promise<boolean>;
  hasBootstrapCompleted(deployment: ControlClaimDeployment, subject: Ownership): Promise<boolean>;
};

/** A typed facade for the platform ControlClaim ingress exposed by JamScriptClient. */
export class ControlClaimClient {
  constructor(private readonly client: ControlClaimClientSurface) {}

  bootstrapMatrixControlClaim(input: MatrixControlBootstrapInput & { deployment: ControlClaimDeployment }) {
    return this.client.bootstrapMatrixControlClaim(input);
  }

  addController(input: { deployment: ControlClaimDeployment; subject: Ownership; controller: Ownership; signer: OwnershipSigner }) {
    return this.client.addController(input);
  }

  revokeController(input: { deployment: ControlClaimDeployment; subject: Ownership; controller: Ownership; signer: OwnershipSigner }) {
    return this.client.revokeController(input);
  }

  isControllerActive(deployment: ControlClaimDeployment, subject: Ownership, controller: Ownership) {
    return this.client.isControllerActive(deployment, subject, controller);
  }

  hasBootstrapCompleted(deployment: ControlClaimDeployment, subject: Ownership) {
    return this.client.hasBootstrapCompleted(deployment, subject);
  }
}
