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
export const FATAL_NUMERIC_OVERFLOW = 0x80000003;
export const FATAL_NUMERIC_UNDERFLOW = 0x80000004;
export const FATAL_NUMERIC_DIV_ZERO = 0x80000005;
export const FATAL_NUMERIC_CAST = 0x80000006;

export type JamOwnership = {
  version: number;
  kind: number;
  public: Uint8Array;
};

const OWNERSHIP_KEY_DOMAIN = new TextEncoder().encode("OWNERSHIP_ABSTRACTION_KEY_V1");
const BLAKE2B_IV_LO = [0xF3BCC908, 0x84CAA73B, 0xFE94F82B, 0x5F1D36F1, 0xADE682D1, 0x2B3E6C1F, 0xFB41BD6B, 0x137E2179];
const BLAKE2B_IV_HI = [0x6A09E667, 0xBB67AE85, 0x3C6EF372, 0xA54FF53A, 0x510E527F, 0x9B05688C, 0x1F83D9AB, 0x5BE0CD19];
const BLAKE2B_SIGMA = [
  [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15],
  [14, 10, 4, 8, 9, 15, 13, 6, 1, 12, 0, 2, 11, 7, 5, 3],
  [11, 8, 12, 0, 5, 2, 15, 13, 10, 14, 3, 6, 7, 1, 9, 4],
  [7, 9, 3, 1, 13, 12, 11, 14, 2, 6, 5, 10, 4, 0, 15, 8],
  [9, 0, 5, 7, 2, 4, 10, 15, 14, 1, 11, 12, 6, 8, 3, 13],
  [2, 12, 6, 10, 0, 11, 8, 3, 4, 13, 7, 5, 15, 14, 1, 9],
  [12, 5, 1, 15, 14, 13, 4, 10, 0, 7, 6, 3, 9, 2, 8, 11],
  [13, 11, 7, 14, 12, 1, 3, 9, 5, 0, 15, 4, 8, 6, 2, 10],
  [6, 15, 14, 9, 11, 3, 0, 8, 12, 2, 13, 7, 1, 4, 10, 5],
  [10, 2, 8, 4, 7, 6, 1, 5, 15, 11, 9, 14, 3, 12, 13, 0],
  [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15],
  [14, 10, 4, 8, 9, 15, 13, 6, 1, 12, 0, 2, 11, 7, 5, 3],
];

function blakeWord(value: number): number { return value >>> 0; }
function blakeXor(left: number, right: number): number { return (left ^ right) >>> 0; }
function blakeAdd(left: number, right: number): number { return (left + right) >>> 0; }
function blakeAdd64(left: { lo: number; hi: number }, right: { lo: number; hi: number }): { lo: number; hi: number } {
  const lo = blakeAdd(left.lo, right.lo);
  const carry = lo < (left.lo >>> 0) ? 1 : 0;
  return { lo, hi: blakeAdd(blakeAdd(left.hi, right.hi), carry) };
}
function blakeXor64(left: { lo: number; hi: number }, right: { lo: number; hi: number }): { lo: number; hi: number } {
  return { lo: blakeXor(left.lo, right.lo), hi: blakeXor(left.hi, right.hi) };
}
function blakeRotr(value: { lo: number; hi: number }, count: number): { lo: number; hi: number } {
  if (count === 32) return { lo: value.hi >>> 0, hi: value.lo >>> 0 };
  if (count < 32) return {
    lo: ((value.lo >>> count) | (value.hi << (32 - count))) >>> 0,
    hi: ((value.hi >>> count) | (value.lo << (32 - count))) >>> 0,
  };
  const shift = count - 32;
  return {
    lo: ((value.hi >>> shift) | (value.lo << (32 - shift))) >>> 0,
    hi: ((value.lo >>> shift) | (value.hi << (32 - shift))) >>> 0,
  };
}
function blakeLoad(bytes: Uint8Array, offset: number): { lo: number; hi: number } {
  const lo = bytes[offset] + bytes[offset + 1] * 256 + bytes[offset + 2] * 65536 + bytes[offset + 3] * 16777216;
  const hi = bytes[offset + 4] + bytes[offset + 5] * 256 + bytes[offset + 6] * 65536 + bytes[offset + 7] * 16777216;
  return { lo: blakeWord(lo), hi: blakeWord(hi) };
}
function blakeStore(bytes: Uint8Array, offset: number, value: { lo: number; hi: number }): void {
  bytes[offset] = value.lo & 255; bytes[offset + 1] = (value.lo >>> 8) & 255; bytes[offset + 2] = (value.lo >>> 16) & 255; bytes[offset + 3] = (value.lo >>> 24) & 255;
  bytes[offset + 4] = value.hi & 255; bytes[offset + 5] = (value.hi >>> 8) & 255; bytes[offset + 6] = (value.hi >>> 16) & 255; bytes[offset + 7] = (value.hi >>> 24) & 255;
}
function blakeG(v: { lo: number; hi: number }[], a: number, b: number, c: number, d: number, x: { lo: number; hi: number }, y: { lo: number; hi: number }): void {
  v[a] = blakeAdd64(blakeAdd64(v[a], v[b]), x);
  v[d] = blakeRotr(blakeXor64(v[d], v[a]), 32);
  v[c] = blakeAdd64(v[c], v[d]);
  v[b] = blakeRotr(blakeXor64(v[b], v[c]), 24);
  v[a] = blakeAdd64(blakeAdd64(v[a], v[b]), y);
  v[d] = blakeRotr(blakeXor64(v[d], v[a]), 16);
  v[c] = blakeAdd64(v[c], v[d]);
  v[b] = blakeRotr(blakeXor64(v[b], v[c]), 63);
}
function blakeCompress(h: { lo: number; hi: number }[], block: Uint8Array, count: number, last: boolean): void {
  const message = Array.from({ length: 16 }, (_, index) => blakeLoad(block, index * 8));
  const v = h.map(value => ({ lo: value.lo, hi: value.hi }));
  for (let index = 0; index < 8; index += 1) v.push({ lo: BLAKE2B_IV_LO[index], hi: BLAKE2B_IV_HI[index] });
  v[12].lo = blakeXor(v[12].lo, count >>> 0);
  v[13].lo = blakeXor(v[13].lo, Math.floor(count / 0x100000000) >>> 0);
  if (last) { v[14].lo = blakeXor(v[14].lo, 0xffffffff); v[14].hi = blakeXor(v[14].hi, 0xffffffff); }
  for (let round = 0; round < 12; round += 1) {
    const sigma = BLAKE2B_SIGMA[round];
    blakeG(v, 0, 4, 8, 12, message[sigma[0]], message[sigma[1]]); blakeG(v, 1, 5, 9, 13, message[sigma[2]], message[sigma[3]]);
    blakeG(v, 2, 6, 10, 14, message[sigma[4]], message[sigma[5]]); blakeG(v, 3, 7, 11, 15, message[sigma[6]], message[sigma[7]]);
    blakeG(v, 0, 5, 10, 15, message[sigma[8]], message[sigma[9]]); blakeG(v, 1, 6, 11, 12, message[sigma[10]], message[sigma[11]]);
    blakeG(v, 2, 7, 8, 13, message[sigma[12]], message[sigma[13]]); blakeG(v, 3, 4, 9, 14, message[sigma[14]], message[sigma[15]]);
  }
  for (let index = 0; index < 8; index += 1) h[index] = blakeXor64(h[index], blakeXor64(v[index], v[index + 8]));
}
function blake2b256(input: Uint8Array): Uint8Array {
  const h = Array.from({ length: 8 }, (_, index) => ({ lo: BLAKE2B_IV_LO[index], hi: BLAKE2B_IV_HI[index] }));
  h[0].lo = blakeXor(h[0].lo, 0x01010020);
  let offset = 0;
  let count = 0;
  while (offset + 128 < input.length) { blakeCompress(h, input.slice(offset, offset + 128), count + 128, false); offset += 128; count += 128; }
  const block = new Uint8Array(128); block.set(input.slice(offset));
  blakeCompress(h, block, count + input.length - offset, true);
  const output = new Uint8Array(32); for (let index = 0; index < 4; index += 1) blakeStore(output, index * 8, h[index]); return output;
}

export function encodeOwnership(value: JamOwnership): Uint8Array {
  const lengths = [32, 32, 33, 20, 32];
  if (value.version !== 1 || value.kind < 0 || value.kind > 4 || value.public.length !== lengths[value.kind] || (value.kind === 2 && value.public[0] !== 2 && value.public[0] !== 3)) throw new Error("invalid Ownership");
  const output = new Uint8Array(4 + value.public.length); output[0] = 1; output[1] = value.kind; output[2] = value.public.length & 255; output[3] = value.public.length >>> 8; output.set(value.public, 4); return output;
}

export function ownershipKey(value: JamOwnership): Uint8Array {
  const canonical = encodeOwnership(value); const preimage = new Uint8Array(OWNERSHIP_KEY_DOMAIN.length + canonical.length); preimage.set(OWNERSHIP_KEY_DOMAIN); preimage.set(canonical, OWNERSHIP_KEY_DOMAIN.length); return blake2b256(preimage);
}

export function decodeOwnershipAt(cursor: { input: Uint8Array; offset: number }): JamOwnership {
  if (cursor.offset < 0 || cursor.offset + 4 > cursor.input.length) throw new Error("truncated Ownership");
  const start = cursor.offset;
  const version = cursor.input[cursor.offset];
  const kind = cursor.input[cursor.offset + 1];
  const length = cursor.input[cursor.offset + 2] + cursor.input[cursor.offset + 3] * 256;
  const end = cursor.offset + 4 + length;
  if (end < cursor.offset || end > cursor.input.length) throw new Error("truncated Ownership");
  cursor.offset = end;
  const value = { version, kind, public: cursor.input.slice(start + 4, end) };
  encodeOwnership(value);
  return value;
}

export function decodeOwnershipAuthContext(raw: Uint8Array): { owner: JamOwnership; controller: JamOwnership } {
  let offset = 0;
  if (raw.length < 3 || raw[offset++] !== 1) throw new Error("invalid Ownership auth context");
  const read = (): JamOwnership => {
    if (offset + 2 > raw.length) throw new Error("truncated Ownership auth context");
    const length = raw[offset] + raw[offset + 1] * 256; offset += 2;
    if (offset + length > raw.length) throw new Error("truncated Ownership auth context");
    const cursor = { input: raw.slice(offset, offset + length), offset: 0 };
    const value = decodeOwnershipAt(cursor);
    if (cursor.offset !== length) throw new Error("invalid Ownership auth context value");
    offset += length;
    return value;
  };
  const owner = read();
  const controller = read();
  if (offset !== raw.length) throw new Error("trailing Ownership auth context");
  return { owner, controller };
}

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

export function failNumeric(code: number): never {
  pendingKind = KIND_FATAL;
  pendingCode = code;
  throw new Error("jamscript numeric failure");
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
