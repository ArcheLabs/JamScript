import { hexToU8a, u8aToHex } from "@polkadot/util";
import { blake2AsU8a } from "@polkadot/util-crypto";

export const ACTION_DOMAIN_V1 = new TextEncoder().encode("JAMSCRIPT_ACTION_V1");
export const APPLICATION_KEY_CLASS_V1 = 0x01;
export const RUNTIME_KEY_CLASS_V1 = 0x00;
export const WALLET_AUTH_MODULE_V1 = 0x01;
export const MANAGED_STATE_COMMITMENT_KEY_V1 = new TextEncoder().encode(
  ":jam-service-runtime:managed-state:v1",
);

export type SignedAction = {
  version: 1;
  genesisHash: Uint8Array;
  serviceId: number;
  actionSelector: Uint8Array;
  signerScheme: 0;
  publicKey: Uint8Array;
  nonce: bigint;
  validUntil: bigint;
  payloadHash: Uint8Array;
  signature: Uint8Array;
  payload: Uint8Array;
};

function concat(...parts: Uint8Array[]): Uint8Array {
  const output = new Uint8Array(parts.reduce((size, part) => size + part.length, 0));
  let offset = 0;
  for (const part of parts) {
    output.set(part, offset);
    offset += part.length;
  }
  return output;
}

function u32(value: number): Uint8Array {
  const output = new Uint8Array(4);
  new DataView(output.buffer).setUint32(0, value, true);
  return output;
}

function u64(value: bigint): Uint8Array {
  const output = new Uint8Array(8);
  new DataView(output.buffer).setBigUint64(0, value, true);
  return output;
}

function byte(value: number): Uint8Array {
  return Uint8Array.of(value);
}

export function parseHex(value: string, expectedBytes?: number): Uint8Array {
  const output = hexToU8a(value);
  if (expectedBytes !== undefined && output.length !== expectedBytes) {
    throw new Error("expected " + expectedBytes + " bytes, got " + output.length);
  }
  return output;
}

export function toHex(value: Uint8Array): string {
  return u8aToHex(value);
}

export function actionSelector(name: string): Uint8Array {
  return blake2AsU8a(concat(new TextEncoder().encode("jamscript/action/v1:"), new TextEncoder().encode(name)), 256).slice(0, 8);
}

export function stateKey(
  _serviceId: number,
  schema: Uint8Array | string,
  key: Uint8Array,
): Uint8Array {
  const schemaBytes = typeof schema === "string" ? new TextEncoder().encode(schema) : schema;
  if (schemaBytes.length > 0xffff) throw new Error("managed state namespace is too large");
  return concat(
    byte(APPLICATION_KEY_CLASS_V1),
    Uint8Array.of(schemaBytes.length & 0xff, schemaBytes.length >>> 8),
    schemaBytes,
    key,
  );
}

export function nonceKey(publicKey: Uint8Array): Uint8Array {
  if (publicKey.length !== 32) throw new Error("wallet account must be 32 bytes");
  return concat(byte(RUNTIME_KEY_CLASS_V1), byte(WALLET_AUTH_MODULE_V1), publicKey);
}

export function signingDigest(action: Omit<SignedAction, "signature">): Uint8Array {
  return blake2AsU8a(
    concat(
      ACTION_DOMAIN_V1,
      byte(action.version),
      action.genesisHash,
      u32(action.serviceId),
      action.actionSelector,
      byte(action.signerScheme),
      u64(action.nonce),
      u64(action.validUntil),
      action.payloadHash,
    ),
    256,
  );
}

export function encodeSignedAction(action: SignedAction): Uint8Array {
  if (action.genesisHash.length !== 32 || action.actionSelector.length !== 8) {
    throw new Error("invalid SignedAction fixed-width field");
  }
  if (action.publicKey.length > 255 || action.signature.length > 255) {
    throw new Error("SignedAction variable-width field is too large");
  }
  return concat(
    byte(action.version),
    action.genesisHash,
    u32(action.serviceId),
    action.actionSelector,
    byte(action.signerScheme),
    byte(action.publicKey.length),
    action.publicKey,
    u64(action.nonce),
    u64(action.validUntil),
    action.payloadHash,
    byte(action.signature.length),
    action.signature,
    u32(action.payload.length),
    action.payload,
  );
}
