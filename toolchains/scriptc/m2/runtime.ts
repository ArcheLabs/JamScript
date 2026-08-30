const STATE_VIEW_VERSION = 1;
const SCRIPT_RESULT_VERSION = 1;
const STATE_DIFF_VERSION = 1;
const KIND_APPLIED = 0;
const KIND_ABORT = 1;
const KIND_NEED_STATE = 2;
const KIND_FATAL = 3;
const MAX_KEY_BYTES = 4096;
const MAX_VALUE_BYTES = 65536;
const MAX_ENTRIES = 4096;
const MAX_VIEW_BYTES = 1048576;
const MAX_ABORT_CODE = 0x00ffffff;
const FATAL_UNCAUGHT = 0x80000001;
const FATAL_INVALID_VIEW = 0x80000002;

type StateEntry = {
  key: Uint8Array;
  value: Uint8Array | null;
};

let baseEntries: StateEntry[] = [];
let overlayEntries: StateEntry[] = [];
let pendingKind = 0;
let pendingCode = 0;
let pendingKey: Uint8Array = new Uint8Array(0);

function failStateView(): never {
  pendingKind = KIND_FATAL;
  pendingCode = FATAL_INVALID_VIEW;
  throw new Error("jamscript state view failure");
}

function needState(key: Uint8Array): never {
  pendingKind = KIND_NEED_STATE;
  pendingKey = copyBytes(key);
  throw new Error("jamscript state dependency");
}

function copyBytes(value: Uint8Array): Uint8Array {
  const result = new Uint8Array(value.length);
  result.set(value, 0);
  return result;
}

function compareBytes(left: Uint8Array, right: Uint8Array): number {
  const length = left.length < right.length ? left.length : right.length;
  for (let index = 0; index < length; index += 1) {
    if (left[index] < right[index]) return -1;
    if (left[index] > right[index]) return 1;
  }
  if (left.length < right.length) return -1;
  if (left.length > right.length) return 1;
  return 0;
}

function readU32(input: Uint8Array, offset: number): number {
  if (offset < 0 || offset + 4 > input.length) failStateView();
  return input[offset]
    + input[offset + 1] * 256
    + input[offset + 2] * 65536
    + input[offset + 3] * 16777216;
}

function writeU32(output: Uint8Array, offset: number, value: number): number {
  output[offset] = value & 255;
  output[offset + 1] = (value >>> 8) & 255;
  output[offset + 2] = (value >>> 16) & 255;
  output[offset + 3] = (value >>> 24) & 255;
  return offset + 4;
}

function readBytes(input: Uint8Array, offset: number, max: number): { value: Uint8Array; next: number } {
  const length = readU32(input, offset);
  const start = offset + 4;
  const end = start + length;
  if (length > max || end < start || end > input.length) failStateView();
  return { value: input.slice(start, end), next: end };
}

export function initializeStateView(input: Uint8Array): void {
  pendingKind = 0;
  pendingCode = 0;
  pendingKey = new Uint8Array(0);
  if (input.length > MAX_VIEW_BYTES || input.length < 5 || input[0] !== STATE_VIEW_VERSION) {
    failStateView();
  }
  const count = readU32(input, 1);
  if (count > MAX_ENTRIES) failStateView();
  const entries: StateEntry[] = [];
  let offset = 5;
  let previous: Uint8Array | null = null;
  for (let index = 0; index < count; index += 1) {
    const decodedKey = readBytes(input, offset, MAX_KEY_BYTES);
    offset = decodedKey.next;
    if (previous !== null && compareBytes(previous, decodedKey.value) >= 0) {
      failStateView();
    }
    if (offset >= input.length) failStateView();
    const presence = input[offset];
    offset += 1;
    let value: Uint8Array | null = null;
    if (presence === 1) {
      const decodedValue = readBytes(input, offset, MAX_VALUE_BYTES);
      value = decodedValue.value;
      offset = decodedValue.next;
    } else if (presence !== 0) {
      failStateView();
    }
    entries.push({ key: decodedKey.value, value });
    previous = decodedKey.value;
  }
  if (offset !== input.length) failStateView();
  baseEntries = entries;
  overlayEntries = [];
}

function find(entries: StateEntry[], key: Uint8Array): number {
  let low = 0;
  let high = entries.length;
  while (low < high) {
    const middle = low + ((high - low) >>> 1);
    const ordering = compareBytes(entries[middle].key, key);
    if (ordering < 0) low = middle + 1;
    else high = middle;
  }
  return low;
}

function knownBase(key: Uint8Array): boolean {
  const index = find(baseEntries, key);
  return index < baseEntries.length && compareBytes(baseEntries[index].key, key) === 0;
}

export function stateGetRaw(key: Uint8Array): Uint8Array | null {
  const overlayIndex = find(overlayEntries, key);
  if (overlayIndex < overlayEntries.length && compareBytes(overlayEntries[overlayIndex].key, key) === 0) {
    const value = overlayEntries[overlayIndex].value;
    return value === null ? null : copyBytes(value);
  }
  const baseIndex = find(baseEntries, key);
  if (baseIndex >= baseEntries.length || compareBytes(baseEntries[baseIndex].key, key) !== 0) {
    needState(key);
  }
  const value = baseEntries[baseIndex].value;
  return value === null ? null : copyBytes(value);
}

function writeOverlay(key: Uint8Array, value: Uint8Array | null): void {
  if (key.length > MAX_KEY_BYTES || (value !== null && value.length > MAX_VALUE_BYTES)) {
    failStateView();
  }
  const index = find(overlayEntries, key);
  const entry = { key: copyBytes(key), value: value === null ? null : copyBytes(value) };
  if (index < overlayEntries.length && compareBytes(overlayEntries[index].key, key) === 0) {
    overlayEntries[index] = entry;
  } else {
    const next: StateEntry[] = [];
    for (let current = 0; current < index; current += 1) next.push(overlayEntries[current]);
    next.push(entry);
    for (let current = index; current < overlayEntries.length; current += 1) next.push(overlayEntries[current]);
    overlayEntries = next;
  }
}

export function stateSetRaw(key: Uint8Array, value: Uint8Array): void {
  const overlayIndex = find(overlayEntries, key);
  const knownOverlay = overlayIndex < overlayEntries.length
    && compareBytes(overlayEntries[overlayIndex].key, key) === 0;
  if (!knownOverlay && !knownBase(key)) needState(key);
  writeOverlay(key, value);
}

export function stateDeleteRaw(key: Uint8Array): void {
  const overlayIndex = find(overlayEntries, key);
  const knownOverlay = overlayIndex < overlayEntries.length
    && compareBytes(overlayEntries[overlayIndex].key, key) === 0;
  if (!knownOverlay && !knownBase(key)) needState(key);
  writeOverlay(key, null);
}

export function stateHasRaw(key: Uint8Array): boolean {
  return stateGetRaw(key) !== null;
}

export function applicationKeyV1(namespace: Uint8Array, canonicalKey: Uint8Array): Uint8Array {
  if (namespace.length > 65535) failStateView();
  const output = new Uint8Array(3 + namespace.length + canonicalKey.length);
  output[0] = 1;
  output[1] = namespace.length & 255;
  output[2] = (namespace.length >>> 8) & 255;
  output.set(namespace, 3);
  output.set(canonicalKey, 3 + namespace.length);
  return output;
}

export function abort(code: number): never {
  if (code < 1 || code > MAX_ABORT_CODE || Math.floor(code) !== code) {
    failStateView();
  }
  pendingKind = KIND_ABORT;
  pendingCode = code;
  throw new Error("jamscript application abort");
}

function encodeCode(kind: number, code: number): Uint8Array {
  const output = new Uint8Array(6);
  output[0] = SCRIPT_RESULT_VERSION;
  output[1] = kind;
  writeU32(output, 2, code);
  return output;
}

function encodeNeedState(key: Uint8Array): Uint8Array {
  const output = new Uint8Array(6 + key.length);
  output[0] = SCRIPT_RESULT_VERSION;
  output[1] = KIND_NEED_STATE;
  writeU32(output, 2, key.length);
  output.set(key, 6);
  return output;
}

function encodeApplied(): Uint8Array {
  let diffLength = 5;
  for (const entry of overlayEntries) {
    diffLength += 4 + entry.key.length + 1;
    if (entry.value !== null) diffLength += 4 + entry.value.length;
  }
  const output = new Uint8Array(6 + diffLength);
  output[0] = SCRIPT_RESULT_VERSION;
  output[1] = KIND_APPLIED;
  writeU32(output, 2, diffLength);
  output[6] = STATE_DIFF_VERSION;
  let offset = writeU32(output, 7, overlayEntries.length);
  for (const entry of overlayEntries) {
    offset = writeU32(output, offset, entry.key.length);
    output.set(entry.key, offset);
    offset += entry.key.length;
    if (entry.value === null) {
      output[offset] = 0;
      offset += 1;
    } else {
      output[offset] = 1;
      offset += 1;
      offset = writeU32(output, offset, entry.value.length);
      output.set(entry.value, offset);
      offset += entry.value.length;
    }
  }
  return output;
}

export function appliedResult(): Uint8Array {
  return encodeApplied();
}

export function caughtResult(_error: unknown): Uint8Array {
  if (pendingKind === KIND_NEED_STATE) return encodeNeedState(pendingKey);
  if (pendingKind === KIND_ABORT) return encodeCode(KIND_ABORT, pendingCode);
  if (pendingKind === KIND_FATAL) return encodeCode(KIND_FATAL, pendingCode);
  return encodeCode(KIND_FATAL, FATAL_UNCAUGHT);
}
