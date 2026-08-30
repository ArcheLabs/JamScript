import { verifyManagedStateProof } from "./proof.js";
import { parseHex, toHex } from "./crypto.js";
import type { RpcTransport } from "./rpc.js";

export type StateProviderRequest = {
  serviceId: number;
  serviceKey: string;
  stateRoot: string;
  key: Uint8Array;
};

export type StateProviderResponse = {
  serviceId: number;
  stateRoot: string;
  key: Uint8Array;
  value: Uint8Array | null;
  proof: Uint8Array[];
};

export type StateProviderFailureKind =
  | "Unavailable"
  | "RootUnavailable"
  | "MalformedResponse"
  | "InvalidProof"
  | "InconsistentResponse";

export class StateProviderError extends Error {
  constructor(
    readonly kind: StateProviderFailureKind,
    message: string,
    readonly cause?: unknown,
  ) {
    super(message);
    this.name = "StateProviderError";
  }
}

export interface StateProvider {
  get(request: StateProviderRequest): Promise<StateProviderResponse>;
}

/** The default MiniJAM-compatible provider. It returns untrusted bytes. */
export class RpcStateProvider implements StateProvider {
  constructor(private readonly transport: RpcTransport) {}

  async get(request: StateProviderRequest): Promise<StateProviderResponse> {
    let response: {
      serviceId: number;
      stateRoot: string;
      keyBase64: string;
      valueBase64: string | null;
      proofBase64: string[];
    };
    try {
      response = await this.transport.call("minijam_getManagedStateV1", {
        serviceId: request.serviceId,
        stateRoot: request.stateRoot,
        keyBase64: toBase64(request.key),
      });
    } catch (error) {
      const detail = error instanceof Error ? ": " + error.message : "";
      throw new StateProviderError(
        classifyRpcFailure(error),
        "managed-state provider request failed" + detail,
        error,
      );
    }
    try {
      return {
        serviceId: response.serviceId,
        stateRoot: response.stateRoot,
        key: fromBase64(response.keyBase64),
        value: response.valueBase64 === null ? null : fromBase64(response.valueBase64),
        proof: response.proofBase64.map(fromBase64),
      };
    } catch (error) {
      throw new StateProviderError("MalformedResponse", "managed-state provider response is malformed", error);
    }
  }
}

/** Tries providers in order and only accepts a response with a valid proof. */
export class FallbackStateProvider implements StateProvider {
  constructor(private readonly providers: readonly StateProvider[]) {
    if (providers.length === 0) throw new Error("at least one state provider is required");
  }

  async get(request: StateProviderRequest): Promise<StateProviderResponse> {
    let lastError: unknown;
    for (const provider of this.providers) {
      try {
        const response = await provider.get(request);
        assertResponseIdentity(request, response);
        verifyManagedStateProof(
          parseHex(request.stateRoot, 32),
          request.key,
          response.value,
          response.proof,
        );
        return response;
      } catch (error) {
        lastError = error instanceof StateProviderError
          ? error
          : new StateProviderError("InvalidProof", "state provider proof verification failed", error);
      }
    }
    throw lastError ?? new StateProviderError("Unavailable", "no state provider is available");
  }
}

function assertResponseIdentity(
  request: StateProviderRequest,
  response: StateProviderResponse,
): void {
  if (
    response.serviceId !== request.serviceId
    || !sameHex(response.stateRoot, request.stateRoot)
    || !sameBytes(response.key, request.key)
  ) {
    throw new StateProviderError(
      "InconsistentResponse",
      "managed-state provider response does not match the request",
    );
  }
}

function classifyRpcFailure(error: unknown): StateProviderFailureKind {
  const code = typeof error === "object" && error !== null && "code" in error
    ? (error as { code?: unknown }).code
    : undefined;
  if (code === -32030 || code === -32601) return "RootUnavailable";
  return "Unavailable";
}

function sameHex(left: string, right: string): boolean {
  return left.toLowerCase().replace(/^0x/, "") === right.toLowerCase().replace(/^0x/, "");
}

function sameBytes(left: Uint8Array, right: Uint8Array): boolean {
  return left.length === right.length && left.every((byte, index) => byte === right[index]);
}

function toBase64(bytes: Uint8Array): string {
  let binary = "";
  for (const byte of bytes) binary += String.fromCharCode(byte);
  return btoa(binary);
}

function fromBase64(value: string): Uint8Array {
  const binary = atob(value);
  const output = new Uint8Array(binary.length);
  for (let index = 0; index < binary.length; index += 1) output[index] = binary.charCodeAt(index);
  return output;
}
