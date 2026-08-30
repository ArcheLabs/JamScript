import { hexToU8a, u8aToHex } from "@polkadot/util";
import { blake2AsU8a } from "@polkadot/util-crypto";

export const ACTION_DOMAIN_V1 = new TextEncoder().encode("JAMSCRIPT_ACTION_V1");
export const APPLICATION_KEY_CLASS_V1 = 0x01;
export const RUNTIME_KEY_CLASS_V1 = 0x00;
export const WALLET_AUTH_MODULE_V1 = 0x01;
export const MANAGED_STATE_COMMITMENT_KEY_V1 = new TextEncoder().encode(
  ":jam-service-runtime:managed-state:v1",
);
export const MAX_ACTION_PAYLOAD_BYTES = 1_048_576;

export type ServiceKeyV1 = Uint8Array;

export type SignedActionV1 = {
  version: 1;
  networkDomain: Uint8Array;
  serviceKey: ServiceKeyV1;
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


export function signingDigestV1(action: Omit<SignedActionV1, "signature">): Uint8Array {
  if (action.networkDomain.length !== 32 || action.serviceKey.length !== 32 || action.actionSelector.length !== 8) {
    throw new Error("invalid SignedActionV1 fixed-width field");
  }
  return blake2AsU8a(
    concat(
      ACTION_DOMAIN_V1,
      byte(action.version),
      action.networkDomain,
      action.serviceKey,
      action.actionSelector,
      byte(action.signerScheme),
      u64(action.nonce),
      u64(action.validUntil),
      action.payloadHash,
    ),
    256,
  );
}


export function encodeSignedActionV1(action: SignedActionV1): Uint8Array {
  if (action.networkDomain.length !== 32 || action.serviceKey.length !== 32 || action.actionSelector.length !== 8) {
    throw new Error("invalid SignedActionV1 fixed-width field");
  }
  if (action.publicKey.length > 32 || action.signature.length > 64) {
    throw new Error("SignedActionV1 variable-width field is too large");
  }
  if (action.payload.length > MAX_ACTION_PAYLOAD_BYTES) throw new Error("SignedActionV1 payload is too large");
  return concat(
    byte(action.version),
    action.networkDomain,
    action.serviceKey,
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

export function decodeSignedActionV1(bytes: Uint8Array): SignedActionV1 {
  let offset = 0;
  const take = (length: number): Uint8Array => {
    const end = offset + length;
    if (end > bytes.length) throw new Error("truncated SignedActionV1");
    const value = bytes.slice(offset, end);
    offset = end;
    return value;
  };
  const readU8 = (): number => take(1)[0];
  const readU32 = (): number => new DataView(take(4).buffer).getUint32(0, true);
  const readU64 = (): bigint => new DataView(take(8).buffer).getBigUint64(0, true);
  const version = readU8();
  const networkDomain = take(32);
  const serviceKey = take(32);
  const actionSelector = take(8);
  const signerScheme = readU8();
  const publicKey = take(readU8());
  const nonce = readU64();
  const validUntil = readU64();
  const payloadHash = take(32);
  const signature = take(readU8());
  const payload = take(readU32());
  if (offset !== bytes.length) throw new Error("trailing SignedActionV1 bytes");
  if (version !== 1 || signerScheme !== 0 || publicKey.length !== 32 || signature.length !== 64 || payload.length > MAX_ACTION_PAYLOAD_BYTES) {
    throw new Error("unsupported SignedActionV1");
  }
  if (toHex(blake2AsU8a(payload, 256)) !== toHex(payloadHash)) {
    throw new Error("SignedActionV1 payload hash mismatch");
  }
  return { version: 1, networkDomain, serviceKey, actionSelector, signerScheme: 0, publicKey, nonce, validUntil, payloadHash, signature, payload };
}
