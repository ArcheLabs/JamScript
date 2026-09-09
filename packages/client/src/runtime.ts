import { blake2AsU8a } from "@polkadot/util-crypto";

export const RUNTIME_REFINEMENT_VERSION = 1;
export const MAX_RUNTIME_ACTIONS = 1024;
export const MAX_RUNTIME_ACTION_BYTES = 1024 * 1024;
export const MAX_RUNTIME_ACTION_TOTAL_BYTES = 4 * 1024 * 1024;
export const MAX_RECOVERY_BYTES = 1024 * 1024;
export const MAX_RECOVERY_CHANGES = 4096;
export const MAX_STATE_KEY_BYTES = 4096;
export const MAX_STATE_VALUE_BYTES = 64 * 1024;
export const MAX_EXTERNAL_STATE_WITNESSES = 64;
export const MAX_EXTERNAL_WITNESS_TOTAL_BYTES = 2 * 1024 * 1024;
const MAX_WITNESS_NODES = 4096;
const MAX_WITNESS_NODE_BYTES = 64 * 1024;
const MAX_WITNESS_BYTES = 1024 * 1024;
const MAX_STATE_VIEW_BYTES = 1024 * 1024;

export type ActionReceiptV1 = {
  actionHash: Uint8Array;
  status: 0 | 1 | 2;
  errorCode: number | null;
};

export type RuntimeRefineOutputV1 = {
  version: 1;
  parentRoot: Uint8Array;
  newRoot: Uint8Array;
  externalDependencies: ExternalStateDependencyV1[];
  transitionValidUntil: bigint | null;
  recoveryCommitment: Uint8Array;
  receipts: ActionReceiptV1[];
  recoveryPayload: Uint8Array;
};

export type ExternalStateDependencyV1 = {
  serviceId: number;
  stateRoot: Uint8Array;
};

export type StateAccessPlanV1 = {
  version: 1;
  keys: Uint8Array[];
};

export type ManagedStateWitnessV1 = {
  version: 1;
  parentRoot: Uint8Array;
  accessPlan: StateAccessPlanV1;
  storageProof: Uint8Array[];
};

export type ExternalStateWitnessV1 = {
  serviceId: number;
  managedState: ManagedStateWitnessV1;
};

export type RuntimeRefineInputV1 = {
  version: 1;
  managedState: ManagedStateWitnessV1;
  externalState: ExternalStateWitnessV1[];
  actions: Uint8Array[];
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

export function encodeRuntimeRefineOutputV1(output: RuntimeRefineOutputV1): Uint8Array {
  if (output.version !== RUNTIME_REFINEMENT_VERSION) throw new Error("unsupported runtime output version");
  ensureBytes(output.parentRoot, 32, "parentRoot");
  ensureBytes(output.newRoot, 32, "newRoot");
  ensureBytes(output.recoveryCommitment, 32, "recoveryCommitment");
  const dependencies = canonicalDependencies(output.externalDependencies);
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
    u32(dependencies.length),
    ...dependencies.map((dependency) => concat(u32(dependency.serviceId), dependency.stateRoot)),
    validity,
    output.recoveryCommitment,
    u32(output.receipts.length),
    ...receipts,
    u32(output.recoveryPayload.length),
    output.recoveryPayload,
  );
}

export function decodeRuntimeRefineOutputV1(bytes: Uint8Array): RuntimeRefineOutputV1 {
  let offset = 0;
  const take = (length: number): Uint8Array => {
    const end = offset + length;
    if (end > bytes.length) throw new Error("truncated RuntimeRefineOutputV1");
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
  const dependencyCount = readU32();
  if (dependencyCount > MAX_EXTERNAL_STATE_WITNESSES) throw new Error("too many external dependencies");
  const externalDependencies: ExternalStateDependencyV1[] = [];
  let previousServiceId: number | null = null;
  for (let index = 0; index < dependencyCount; index += 1) {
    const serviceId = readU32();
    const stateRoot = take(32);
    if (previousServiceId !== null && serviceId < previousServiceId) {
      throw new Error("external dependencies are not sorted");
    }
    if (externalDependencies.at(-1)?.serviceId === serviceId) {
      const previousRoot = externalDependencies.at(-1)?.stateRoot;
      if (!previousRoot || compareBytes(previousRoot, stateRoot) !== 0) {
        throw new Error("duplicate external dependency service ID");
      }
      continue;
    }
    previousServiceId = serviceId;
    externalDependencies.push({ serviceId, stateRoot });
  }
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
  if (offset !== bytes.length) throw new Error("trailing RuntimeRefineOutputV1 bytes");
  validateRecoveryPayload(recoveryPayload);
  const expectedCommitment = blake2AsU8a(recoveryPayload, 256);
  if (compareBytes(expectedCommitment, recoveryCommitment) !== 0) throw new Error("recovery commitment mismatch");
  return { version: 1, parentRoot, newRoot, externalDependencies, transitionValidUntil, recoveryCommitment, receipts, recoveryPayload };
}

export function encodeRuntimeRefineInputV1(input: RuntimeRefineInputV1): Uint8Array {
  if (input.version !== RUNTIME_REFINEMENT_VERSION) throw new Error("unsupported runtime input version");
  const managed = encodeManagedStateWitnessV1(input.managedState);
  const external = encodeExternalStateWitnessesV1(input.externalState);
  if (input.actions.length > MAX_RUNTIME_ACTIONS) throw new Error("too many actions");
  const actionBytes = input.actions.reduce((total, action) => total + action.length, 0);
  if (actionBytes > MAX_RUNTIME_ACTION_TOTAL_BYTES) throw new Error("actions are too large");
  return concat(
    Uint8Array.of(input.version),
    encodeBytes(managed, MAX_WITNESS_ENCODED_BYTES),
    encodeBytes(external, MAX_EXTERNAL_WITNESS_TOTAL_BYTES),
    u32(input.actions.length),
    ...input.actions.map((action) => encodeBytes(action, MAX_RUNTIME_ACTION_BYTES)),
  );
}

export function decodeRuntimeRefineInputV1(bytes: Uint8Array): RuntimeRefineInputV1 {
  const reader = new RuntimeReader(bytes);
  const version = reader.u8();
  if (version !== RUNTIME_REFINEMENT_VERSION) throw new Error("unsupported runtime input version");
  const managedState = decodeManagedStateWitnessV1(reader.bytes(MAX_WITNESS_ENCODED_BYTES));
  const externalState = decodeExternalStateWitnessesV1(
    reader.bytes(MAX_EXTERNAL_WITNESS_TOTAL_BYTES),
  );
  const count = reader.u32();
  if (count > MAX_RUNTIME_ACTIONS) throw new Error("too many actions");
  const actions: Uint8Array[] = [];
  let actionBytes = 0;
  for (let index = 0; index < count; index += 1) {
    const action = reader.bytes(MAX_RUNTIME_ACTION_BYTES);
    actionBytes += action.length;
    if (actionBytes > MAX_RUNTIME_ACTION_TOTAL_BYTES) throw new Error("actions are too large");
    actions.push(action);
  }
  if (reader.remaining() !== 0) throw new Error("trailing RuntimeRefineInputV1 bytes");
  return { version: 1, managedState, externalState, actions };
}

const MAX_WITNESS_ENCODED_BYTES = 1 + 32 + 4 + MAX_STATE_VIEW_BYTES + 4 + (MAX_WITNESS_NODES * 4) + MAX_WITNESS_BYTES;

function encodeBytes(value: Uint8Array, maximum: number): Uint8Array {
  if (value.length > maximum) throw new Error("runtime value is too large");
  return concat(u32(value.length), value);
}

function encodeAccessPlanV1(plan: StateAccessPlanV1): Uint8Array {
  if (plan.version !== 1 || plan.keys.length > MAX_RECOVERY_CHANGES) throw new Error("invalid state access plan");
  for (let index = 1; index < plan.keys.length; index += 1) {
    if (compareBytes(plan.keys[index - 1], plan.keys[index]) >= 0) throw new Error("state access plan is not canonical");
  }
  return concat(
    Uint8Array.of(plan.version),
    u32(plan.keys.length),
    ...plan.keys.map((key) => encodeBytes(key, MAX_STATE_KEY_BYTES)),
  );
}

function encodeManagedStateWitnessV1(witness: ManagedStateWitnessV1): Uint8Array {
  if (witness.version !== 1) throw new Error("invalid managed state witness version");
  ensureBytes(witness.parentRoot, 32, "parentRoot");
  if (witness.storageProof.length > MAX_WITNESS_NODES) throw new Error("too many witness nodes");
  let total = 0;
  const nodes = witness.storageProof.map((node) => {
    total += node.length;
    if (node.length > MAX_WITNESS_NODE_BYTES || total > MAX_WITNESS_BYTES) throw new Error("witness is too large");
    return encodeBytes(node, MAX_WITNESS_NODE_BYTES);
  });
  return concat(
    Uint8Array.of(witness.version),
    witness.parentRoot,
    encodeBytes(encodeAccessPlanV1(witness.accessPlan), MAX_ACCESS_PLAN_ENCODED_BYTES),
    u32(nodes.length),
    ...nodes,
  );
}

function encodeExternalStateWitnessesV1(witnesses: readonly ExternalStateWitnessV1[]): Uint8Array {
  if (witnesses.length > MAX_EXTERNAL_STATE_WITNESSES) throw new Error("too many external witnesses");
  const ordered = [...witnesses].sort((left, right) => left.serviceId - right.serviceId);
  const result: Uint8Array[] = [u32(ordered.length)];
  let previous: number | null = null;
  let total = 0;
  for (const witness of ordered) {
    if (previous === witness.serviceId) throw new Error("duplicate external witness service ID");
    if (!Number.isSafeInteger(witness.serviceId) || witness.serviceId < 0 || witness.serviceId > 0xffffffff) throw new Error("invalid external witness service ID");
    previous = witness.serviceId;
    const managed = encodeManagedStateWitnessV1(witness.managedState);
    total += managed.length;
    if (total > MAX_EXTERNAL_WITNESS_TOTAL_BYTES) throw new Error("external witnesses are too large");
    result.push(u32(witness.serviceId), encodeBytes(managed, MAX_WITNESS_ENCODED_BYTES));
  }
  const encoded = concat(...result);
  if (encoded.length > MAX_EXTERNAL_WITNESS_TOTAL_BYTES) throw new Error("external witnesses are too large");
  return encoded;
}

function decodeAccessPlanV1(bytes: Uint8Array): StateAccessPlanV1 {
  const reader = new RuntimeReader(bytes);
  const version = reader.u8();
  if (version !== 1) throw new Error("invalid state access plan version");
  const count = reader.u32();
  if (count > MAX_RECOVERY_CHANGES) throw new Error("too many state access keys");
  const keys: Uint8Array[] = [];
  for (let index = 0; index < count; index += 1) {
    const key = reader.bytes(MAX_STATE_KEY_BYTES);
    if (keys.length > 0 && compareBytes(keys[keys.length - 1], key) >= 0) throw new Error("state access plan is not canonical");
    keys.push(key);
  }
  if (reader.remaining() !== 0) throw new Error("trailing state access plan bytes");
  return { version: 1, keys };
}

function decodeManagedStateWitnessV1(bytes: Uint8Array): ManagedStateWitnessV1 {
  const reader = new RuntimeReader(bytes);
  const version = reader.u8();
  if (version !== 1) throw new Error("invalid managed state witness version");
  const parentRoot = reader.take(32);
  const accessPlan = decodeAccessPlanV1(reader.bytes(MAX_ACCESS_PLAN_ENCODED_BYTES));
  const count = reader.u32();
  if (count > MAX_WITNESS_NODES) throw new Error("too many witness nodes");
  const storageProof: Uint8Array[] = [];
  let total = 0;
  for (let index = 0; index < count; index += 1) {
    const node = reader.bytes(MAX_WITNESS_NODE_BYTES);
    total += node.length;
    if (total > MAX_WITNESS_BYTES) throw new Error("witness is too large");
    storageProof.push(node);
  }
  if (reader.remaining() !== 0) throw new Error("trailing managed state witness bytes");
  return { version: 1, parentRoot, accessPlan, storageProof };
}

function decodeExternalStateWitnessesV1(bytes: Uint8Array): ExternalStateWitnessV1[] {
  const reader = new RuntimeReader(bytes);
  const count = reader.u32();
  if (count > MAX_EXTERNAL_STATE_WITNESSES) throw new Error("too many external witnesses");
  const witnesses: ExternalStateWitnessV1[] = [];
  let previous: number | null = null;
  let total = 0;
  for (let index = 0; index < count; index += 1) {
    const serviceId = reader.u32();
    if (previous !== null && serviceId <= previous) throw new Error("external witnesses are not sorted");
    previous = serviceId;
    const managedBytes = reader.bytes(MAX_WITNESS_ENCODED_BYTES);
    total += managedBytes.length;
    if (total > MAX_EXTERNAL_WITNESS_TOTAL_BYTES) throw new Error("external witnesses are too large");
    witnesses.push({ serviceId, managedState: decodeManagedStateWitnessV1(managedBytes) });
  }
  if (reader.remaining() !== 0) throw new Error("trailing external witness bytes");
  return witnesses;
}

const MAX_ACCESS_PLAN_ENCODED_BYTES = MAX_STATE_VIEW_BYTES;

class RuntimeReader {
  private offset = 0;

  constructor(private readonly bytesValue: Uint8Array) {}

  take(length: number): Uint8Array {
    const end = this.offset + length;
    if (end > this.bytesValue.length) throw new Error("truncated runtime wire value");
    const value = this.bytesValue.slice(this.offset, end);
    this.offset = end;
    return value;
  }

  u8(): number {
    return this.take(1)[0];
  }

  u32(): number {
    return new DataView(this.take(4).buffer).getUint32(0, true);
  }

  bytes(maximum: number): Uint8Array {
    const length = this.u32();
    if (length > maximum) throw new Error("runtime wire value is too large");
    return this.take(length);
  }

  remaining(): number {
    return this.bytesValue.length - this.offset;
  }
}

function canonicalDependencies(
  dependencies: readonly ExternalStateDependencyV1[],
): ExternalStateDependencyV1[] {
  if (dependencies.length > MAX_EXTERNAL_STATE_WITNESSES) {
    throw new Error("too many external dependencies");
  }
  const ordered = [...dependencies]
    .map((dependency) => {
      ensureBytes(dependency.stateRoot, 32, "external dependency stateRoot");
      if (!Number.isSafeInteger(dependency.serviceId) || dependency.serviceId < 0 || dependency.serviceId > 0xffffffff) {
        throw new Error("invalid external dependency serviceId");
      }
      return dependency;
    })
    .sort((left, right) => left.serviceId - right.serviceId);
  const result: ExternalStateDependencyV1[] = [];
  for (const dependency of ordered) {
    const previous = result.at(-1);
    if (previous?.serviceId === dependency.serviceId) {
      if (compareBytes(previous.stateRoot, dependency.stateRoot) !== 0) {
        throw new Error("duplicate external dependency service ID");
      }
      continue;
    }
    result.push(dependency);
  }
  return result;
}
