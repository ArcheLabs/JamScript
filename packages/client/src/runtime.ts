import { blake2AsU8a } from "@polkadot/util-crypto";

export const RUNTIME_REFINEMENT_VERSION = 2;
export const MAX_RUNTIME_ACTIONS = 1024;
export const MAX_RECOVERY_BYTES = 1024 * 1024;
export const MAX_RECOVERY_CHANGES = 4096;
export const MAX_STATE_KEY_BYTES = 4096;
export const MAX_STATE_VALUE_BYTES = 64 * 1024;

export type ActionReceiptV1 = {
  actionHash: Uint8Array;
  status: 0 | 1 | 2;
  errorCode: number | null;
};

export type RuntimeRefineOutputV2 = {
  version: 2;
  parentRoot: Uint8Array;
  newRoot: Uint8Array;
  transitionValidUntil: bigint | null;
  recoveryCommitment: Uint8Array;
  receipts: ActionReceiptV1[];
  recoveryPayload: Uint8Array;
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

function ensureBytes(value: Uint8Array, length: number, name: string): void {
  if (value.length !== length) throw new Error(`${name} must be ${length} bytes`);
}

function validateRecoveryPayload(bytes: Uint8Array): void {
  if (bytes.length > MAX_RECOVERY_BYTES) throw new Error("recovery payload is too large");
  let offset = 0;
  const take = (length: number): Uint8Array => {
    const end = offset + length;
    if (end > bytes.length) throw new Error("truncated recovery payload");
    const value = bytes.slice(offset, end);
    offset = end;
    return value;
  };
  const readU8 = (): number => take(1)[0];
  const readU32 = (): number => new DataView(take(4).buffer).getUint32(0, true);
  const diffVersion = readU8();
  if (diffVersion !== 1) throw new Error("unsupported recovery version");
  const diffLength = readU32();
  const diff = take(diffLength);
  let diffOffset = 0;
  const diffTake = (length: number): Uint8Array => {
    const end = diffOffset + length;
    if (end > diff.length) throw new Error("truncated state diff");
    const value = diff.slice(diffOffset, end);
    diffOffset = end;
    return value;
  };
  const diffU8 = (): number => diffTake(1)[0];
  const diffU32 = (): number => new DataView(diffTake(4).buffer).getUint32(0, true);
  if (diffU8() !== 1) throw new Error("unsupported state diff version");
  const count = diffU32();
  if (count > MAX_RECOVERY_CHANGES) throw new Error("too many state changes");
  let previousKey: Uint8Array | null = null;
  for (let index = 0; index < count; index += 1) {
    const keyLength = diffU32();
    if (keyLength > MAX_STATE_KEY_BYTES) throw new Error("state key is too large");
    const key = diffTake(keyLength);
    if (previousKey && compareBytes(previousKey, key) >= 0) {
      throw new Error("state diff keys are not strictly sorted");
    }
    previousKey = key;
    const valueTag = diffU8();
    if (valueTag === 1) {
      const valueLength = diffU32();
      if (valueLength > MAX_STATE_VALUE_BYTES) throw new Error("state value is too large");
      diffTake(valueLength);
    } else if (valueTag !== 0) {
      throw new Error("invalid state diff value tag");
    }
  }
  if (diffOffset !== diff.length || offset !== bytes.length) {
    throw new Error("trailing recovery payload bytes");
  }
}

function compareBytes(left: Uint8Array, right: Uint8Array): number {
  const length = Math.min(left.length, right.length);
  for (let index = 0; index < length; index += 1) {
    if (left[index] !== right[index]) return left[index] - right[index];
  }
  return left.length - right.length;
}

export function encodeRuntimeRefineOutputV2(output: RuntimeRefineOutputV2): Uint8Array {
  if (output.version !== RUNTIME_REFINEMENT_VERSION) throw new Error("unsupported runtime output version");
  ensureBytes(output.parentRoot, 32, "parentRoot");
  ensureBytes(output.newRoot, 32, "newRoot");
  ensureBytes(output.recoveryCommitment, 32, "recoveryCommitment");
  if (output.receipts.length > MAX_RUNTIME_ACTIONS) throw new Error("too many receipts");
  validateRecoveryPayload(output.recoveryPayload);
  const validity = output.transitionValidUntil === null
    ? Uint8Array.of(0)
    : concat(Uint8Array.of(1), u64(output.transitionValidUntil));
  const receipts = output.receipts.map((receipt) => {
    ensureBytes(receipt.actionHash, 32, "actionHash");
    if (![0, 1, 2].includes(receipt.status)) throw new Error("invalid receipt status");
    return concat(
      receipt.actionHash,
      Uint8Array.of(receipt.status),
      receipt.errorCode === null ? Uint8Array.of(0) : concat(Uint8Array.of(1), u32(receipt.errorCode)),
    );
  });
  return concat(
    Uint8Array.of(output.version),
    output.parentRoot,
    output.newRoot,
    validity,
    output.recoveryCommitment,
    u32(output.receipts.length),
    ...receipts,
    u32(output.recoveryPayload.length),
    output.recoveryPayload,
  );
}

export function decodeRuntimeRefineOutputV2(bytes: Uint8Array): RuntimeRefineOutputV2 {
  let offset = 0;
  const take = (length: number): Uint8Array => {
    const end = offset + length;
    if (end > bytes.length) throw new Error("truncated RuntimeRefineOutputV2");
    const value = bytes.slice(offset, end);
    offset = end;
    return value;
  };
  const readU8 = (): number => take(1)[0];
  const readU32 = (): number => new DataView(take(4).buffer).getUint32(0, true);
  const readU64 = (): bigint => new DataView(take(8).buffer).getBigUint64(0, true);
  const version = readU8();
  if (version !== RUNTIME_REFINEMENT_VERSION) throw new Error("unsupported runtime output version");
  const parentRoot = take(32);
  const newRoot = take(32);
  const validityTag = readU8();
  const transitionValidUntil = validityTag === 0 ? null : validityTag === 1 ? readU64() : (() => { throw new Error("invalid validity tag"); })();
  const recoveryCommitment = take(32);
  const count = readU32();
  if (count > MAX_RUNTIME_ACTIONS) throw new Error("too many receipts");
  const receipts: ActionReceiptV1[] = [];
  for (let index = 0; index < count; index += 1) {
    const actionHash = take(32);
    const status = readU8();
    if (status > 2) throw new Error("invalid receipt status");
    const errorTag = readU8();
    const errorCode = errorTag === 0 ? null : errorTag === 1 ? readU32() : (() => { throw new Error("invalid receipt error tag"); })();
    receipts.push({ actionHash, status: status as 0 | 1 | 2, errorCode });
  }
  const recoveryLength = readU32();
  if (recoveryLength > MAX_RECOVERY_BYTES) throw new Error("recovery payload is too large");
  const recoveryPayload = take(recoveryLength);
  if (offset !== bytes.length) throw new Error("trailing RuntimeRefineOutputV2 bytes");
  validateRecoveryPayload(recoveryPayload);
  const expectedCommitment = blake2AsU8a(recoveryPayload, 256);
  if (compareBytes(expectedCommitment, recoveryCommitment) !== 0) throw new Error("recovery commitment mismatch");
  return { version: 2, parentRoot, newRoot, transitionValidUntil, recoveryCommitment, receipts, recoveryPayload };
}
