import type { AbiTypeDescriptor, AbiTypeRef, JamScriptAbi } from "./abi.js";

export type CodecValue = null | undefined | bigint | number | boolean | string | Uint8Array | CodecValue[] | { [key: string]: CodecValue };

class Writer {
  private readonly chunks: Uint8Array[] = [];
  push(bytes: Uint8Array): void { this.chunks.push(bytes); }
  finish(): Uint8Array { const size = this.chunks.reduce((sum, chunk) => sum + chunk.length, 0); const output = new Uint8Array(size); let offset = 0; for (const chunk of this.chunks) { output.set(chunk, offset); offset += chunk.length; } return output; }
}
class Reader {
  private offset = 0;
  constructor(private readonly bytes: Uint8Array) {}
  take(length: number): Uint8Array { const end = this.offset + length; if (!Number.isSafeInteger(end) || end > this.bytes.length) throw new Error("truncated JAM value"); const result = this.bytes.slice(this.offset, end); this.offset = end; return result; }
  u8(): number { return this.take(1)[0]; }
  natural(): bigint { const first = this.u8(); if (first < 0x80) return BigInt(first); const extra = first === 0xff ? 8 : Math.clz32((~first) & 0xff) - 24; if (extra === 8) return new DataView(this.take(8).buffer).getBigUint64(0, true); let result = 0n; for (let i = 0; i < extra; i += 1) result |= BigInt(this.u8()) << BigInt(i * 8); return result | BigInt(first & (0x7f >> extra)) << BigInt(extra * 8); }
  remaining(): number { return this.bytes.length - this.offset; }
}
function compact(value: bigint): Uint8Array { if (value < 0n) throw new Error("length cannot be negative"); if (value < 128n) return Uint8Array.of(Number(value)); if (value < (1n << 56n)) { let extra = 0; let threshold = 128n; while (value >= threshold) { extra += 1; threshold <<= 7n; } const output = new Uint8Array(extra + 1); output[0] = (256 - (1 << (8 - extra))) | Number(value >> BigInt(extra * 8)); let left = value; for (let i = 0; i < extra; i += 1) { output[i + 1] = Number(left & 0xffn); left >>= 8n; } return output; } const output = new Uint8Array(9); output[0] = 0xff; let left = value; for (let i = 0; i < 8; i += 1) { output[i + 1] = Number(left & 0xffn); left >>= 8n; } if (left !== 0n) throw new Error("natural is out of range"); return output; }
function asBytes(value: CodecValue): Uint8Array { if (!(value instanceof Uint8Array)) throw new Error("expected Uint8Array"); return value; }
function integer(value: CodecValue, min: bigint, max: bigint, type: string): bigint { const result = typeof value === "bigint" ? value : typeof value === "number" && Number.isSafeInteger(value) ? BigInt(value) : null; if (result === null) throw new Error(type + " must be a bigint or safe integer"); if (result < min || result > max) throw new Error(type + " is out of range"); return result; }
function le(value: bigint, width: number): Uint8Array { const result = new Uint8Array(width); let left = BigInt.asUintN(width * 8, value); for (let i = 0; i < width; i += 1) { result[i] = Number(left & 0xffn); left >>= 8n; } return result; }
function readLe(reader: Reader, width: number, signed: boolean): bigint { const data = reader.take(width); let result = 0n; for (let i = width - 1; i >= 0; i -= 1) result = (result << 8n) | BigInt(data[i]); return signed ? BigInt.asIntN(width * 8, result) : result; }

function descriptor(type: AbiTypeRef): AbiTypeDescriptor {
  if (typeof type !== "string") return type;
  const bounded = /^(Bytes|string)<([0-9]+)>$/.exec(type); if (bounded) return { kind: bounded[1] === "Bytes" ? "bytes" : "string", max: Number(bounded[2]) };
  const fixed = /^FixedBytes<([0-9]+)>$/.exec(type); if (fixed) return { kind: "fixedBytes", len: Number(fixed[1]) };
  if (["unit", "bool", "u8", "u16", "u32", "u64", "u128", "i8", "i16", "i32", "i64", "i128", "address"].includes(type)) return { kind: type as AbiTypeDescriptor["kind"] } as AbiTypeDescriptor;
  throw new Error("unsupported ABI type: " + type);
}

function encode(type: AbiTypeRef, value: CodecValue, writer: Writer): void {
  const ty = descriptor(type);
  switch (ty.kind) {
    case "unit": return;
    case "bool": if (typeof value !== "boolean") throw new Error("bool must be a boolean"); writer.push(Uint8Array.of(value ? 1 : 0)); return;
    case "u8": writer.push(Uint8Array.of(Number(integer(value, 0n, 0xffn, "u8")))); return;
    case "u16": writer.push(le(integer(value, 0n, 0xffffn, "u16"), 2)); return;
    case "u32": writer.push(le(integer(value, 0n, 0xffffffffn, "u32"), 4)); return;
    case "u64": writer.push(le(integer(value, 0n, 0xffffffffffffffffn, "u64"), 8)); return;
    case "u128": writer.push(le(integer(value, 0n, (1n << 128n) - 1n, "u128"), 16)); return;
    case "i8": writer.push(le(integer(value, -128n, 127n, "i8"), 1)); return;
    case "i16": writer.push(le(integer(value, -32768n, 32767n, "i16"), 2)); return;
    case "i32": writer.push(le(integer(value, -2147483648n, 2147483647n, "i32"), 4)); return;
    case "i64": writer.push(le(integer(value, -(1n << 63n), (1n << 63n) - 1n, "i64"), 8)); return;
    case "i128": writer.push(le(integer(value, -(1n << 127n), (1n << 127n) - 1n, "i128"), 16)); return;
    case "address": { const data = asBytes(value); if (data.length !== 32) throw new Error("address must be 32 bytes"); writer.push(data); return; }
    case "fixedBytes": { const data = asBytes(value); if (data.length !== ty.len) throw new Error(`fixedBytes length must be ${ty.len}`); writer.push(data); return; }
    case "bytes": { const data = asBytes(value); if (data.length > ty.max) throw new Error("bytes value exceeds its bound"); writer.push(compact(BigInt(data.length))); writer.push(data); return; }
    case "string": { if (typeof value !== "string") throw new Error("string must be a string"); const data = new TextEncoder().encode(value); if (data.length > ty.max) throw new Error("string value exceeds its UTF-8 byte bound"); writer.push(compact(BigInt(data.length))); writer.push(data); return; }
    case "fixedArray": if (!Array.isArray(value) || value.length !== ty.len) throw new Error("fixedArray length mismatch"); value.forEach(item => encode(ty.item, item, writer)); return;
    case "array": if (!Array.isArray(value) || value.length > ty.max) throw new Error("array value exceeds its bound"); writer.push(compact(BigInt(value.length))); value.forEach(item => encode(ty.item, item, writer)); return;
    case "option": if (value === null || value === undefined) writer.push(Uint8Array.of(0)); else { writer.push(Uint8Array.of(1)); encode(ty.item, value, writer); } return;
    case "tuple": if (!Array.isArray(value) || value.length !== ty.items.length) throw new Error("tuple length mismatch"); ty.items.forEach((item, i) => encode(item, value[i], writer)); return;
    case "record": if (value === null || typeof value !== "object" || Array.isArray(value) || value instanceof Uint8Array) throw new Error("record must be an object"); ty.fields.forEach(field => { if (!(field.name in value)) throw new Error("missing record field: " + field.name); encode(field.type, value[field.name], writer); }); return;
    case "enum": { if (value === null || typeof value !== "object" || Array.isArray(value) || value instanceof Uint8Array) throw new Error("enum must contain index and value"); const enumeration = value as { index?: number; value?: CodecValue }; if (typeof enumeration.index !== "number" || enumeration.value === undefined) throw new Error("enum must contain index and value"); const variant = ty.variants.find(candidate => candidate.index === enumeration.index); if (!variant) throw new Error("invalid enum variant"); writer.push(Uint8Array.of(enumeration.index)); encode(variant.type, enumeration.value, writer); return; }
    case "result": { if (value === null || typeof value !== "object" || Array.isArray(value) || value instanceof Uint8Array) throw new Error("result must be an object"); const result = value as { ok?: CodecValue; err?: CodecValue }; const ok = "ok" in result; writer.push(Uint8Array.of(ok ? 0 : 1)); encode(ok ? ty.ok : ty.err, ok ? result.ok : result.err, writer); return; }
    default: throw new Error("unsupported ABI type");
  }
}
function decode(reader: Reader, type: AbiTypeRef): CodecValue { const ty = descriptor(type); switch (ty.kind) {
  case "unit": return undefined; case "bool": { const value = reader.u8(); if (value > 1) throw new Error("invalid bool value"); return value === 1; }
  case "u8": return Number(readLe(reader, 1, false)); case "u16": return Number(readLe(reader, 2, false)); case "u32": return Number(readLe(reader, 4, false)); case "u64": return readLe(reader, 8, false); case "u128": return readLe(reader, 16, false);
  case "i8": return Number(readLe(reader, 1, true)); case "i16": return Number(readLe(reader, 2, true)); case "i32": return Number(readLe(reader, 4, true)); case "i64": return readLe(reader, 8, true); case "i128": return readLe(reader, 16, true);
  case "address": return reader.take(32); case "fixedBytes": return reader.take(ty.len);
  case "bytes": { const length = reader.natural(); if (length > BigInt(ty.max)) throw new Error("bytes value exceeds its bound"); return reader.take(Number(length)); }
  case "string": { const length = reader.natural(); if (length > BigInt(ty.max)) throw new Error("string value exceeds its UTF-8 byte bound"); return new TextDecoder("utf-8", { fatal: true }).decode(reader.take(Number(length))); }
  case "fixedArray": return Array.from({ length: ty.len }, () => decode(reader, ty.item)); case "array": { const length = reader.natural(); if (length > BigInt(ty.max)) throw new Error("array value exceeds its bound"); return Array.from({ length: Number(length) }, () => decode(reader, ty.item)); }
  case "option": { const tag = reader.u8(); if (tag === 0) return null; if (tag !== 1) throw new Error("invalid option tag"); return decode(reader, ty.item); }
  case "tuple": return ty.items.map(item => decode(reader, item)); case "record": return Object.fromEntries(ty.fields.map(field => [field.name, decode(reader, field.type)]));
  case "enum": { const index = reader.u8(); const variant = ty.variants.find(candidate => candidate.index === index); if (!variant) throw new Error("invalid enum variant"); return { index, value: decode(reader, variant.type) }; }
  case "result": { const tag = reader.u8(); if (tag === 0) return { ok: decode(reader, ty.ok) }; if (tag === 1) return { err: decode(reader, ty.err) }; throw new Error("invalid result tag"); }
  default: throw new Error("unsupported ABI type");
} }

export function encodeValue(type: AbiTypeRef, value: CodecValue): Uint8Array { const writer = new Writer(); encode(type, value, writer); return writer.finish(); }
export function decodeValue(type: AbiTypeRef, bytes: Uint8Array): CodecValue { const reader = new Reader(bytes); const result = decode(reader, type); if (reader.remaining() !== 0) throw new Error("trailing bytes in ABI value"); return result; }
export function encodeActionPayload(abi: JamScriptAbi, actionName: string, values: Record<string, CodecValue>): Uint8Array { const action = abi.actions.find(candidate => candidate.name === actionName); if (!action) throw new Error("unknown JamScript action: " + actionName); const writer = new Writer(); for (const field of action.input) { if (!(field.name in values)) throw new Error("missing action field: " + field.name); encode(field.type, values[field.name], writer); } return writer.finish(); }
export function decodeStateValue(encoded: Uint8Array): Uint8Array {
  if (encoded.length === 0) throw new Error("empty StateValue");
  const first = encoded[0]; let length: number; let offset: number;
  switch (first & 3) {
    case 0: length = first >>> 2; offset = 1; break;
    case 1: if (encoded.length < 2) throw new Error("truncated StateValue length"); length = ((encoded[0] | (encoded[1] << 8)) >>> 2); offset = 2; break;
    case 2: if (encoded.length < 4) throw new Error("truncated StateValue length"); length = ((encoded[0] | (encoded[1] << 8) | (encoded[2] << 16) | (encoded[3] << 24)) >>> 2); offset = 4; break;
    default: throw new Error("StateValue is too large for browser client");
  }
  if (offset + length !== encoded.length) throw new Error("invalid StateValue length");
  return encoded.slice(offset);
}
