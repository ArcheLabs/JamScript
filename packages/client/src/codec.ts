import type { AbiField, JamScriptAbi } from "./abi.js";

export type CodecValue = bigint | number | boolean | string | Uint8Array;

class Writer {
  private readonly chunks: Uint8Array[] = [];

  push(bytes: Uint8Array): void {
    this.chunks.push(bytes);
  }

  u8(value: number): void {
    this.push(Uint8Array.of(value & 0xff));
  }

  u32(value: number): void {
    const bytes = new Uint8Array(4);
    new DataView(bytes.buffer).setUint32(0, value, true);
    this.push(bytes);
  }

  u64(value: bigint): void {
    const bytes = new Uint8Array(8);
    new DataView(bytes.buffer).setBigUint64(0, value, true);
    this.push(bytes);
  }

  u128(value: bigint): void {
    const bytes = new Uint8Array(16);
    let remaining = value;
    for (let index = 0; index < 16; index += 1) {
      bytes[index] = Number(remaining & 0xffn);
      remaining >>= 8n;
    }
    if (remaining !== 0n) throw new Error("u128 value is out of range");
    this.push(bytes);
  }

  finish(): Uint8Array {
    const size = this.chunks.reduce((sum, chunk) => sum + chunk.length, 0);
    const output = new Uint8Array(size);
    let offset = 0;
    for (const chunk of this.chunks) {
      output.set(chunk, offset);
      offset += chunk.length;
    }
    return output;
  }
}

class Reader {
  private offset = 0;
  constructor(private readonly bytes: Uint8Array) {}

  take(length: number): Uint8Array {
    const end = this.offset + length;
    if (end > this.bytes.length) throw new Error("truncated SCALE value");
    const output = this.bytes.slice(this.offset, end);
    this.offset = end;
    return output;
  }

  u32(): number {
    return new DataView(this.take(4).buffer).getUint32(0, true);
  }

  u64(): bigint {
    return new DataView(this.take(8).buffer).getBigUint64(0, true);
  }

  u128(): bigint {
    const bytes = this.take(16);
    let output = 0n;
    for (let index = 15; index >= 0; index -= 1) {
      output = (output << 8n) | BigInt(bytes[index]);
    }
    return output;
  }

  remaining(): number {
    return this.bytes.length - this.offset;
  }
}

function asBytes(value: CodecValue): Uint8Array {
  if (!(value instanceof Uint8Array)) throw new Error("expected Uint8Array");
  return value;
}

function asBigInt(value: CodecValue): bigint {
  if (typeof value === "bigint" || typeof value === "number" || typeof value === "string") {
    return BigInt(value);
  }
  throw new Error("expected integer value");
}

function parseBounded(type: string): { kind: "bytes" | "string"; max: number } | undefined {
  const match = /^(Bytes|string)<([0-9]+)>$/.exec(type);
  if (!match) return undefined;
  return { kind: match[1] === "Bytes" ? "bytes" : "string", max: Number(match[2]) };
}

export function encodeValue(type: string, value: CodecValue): Uint8Array {
  const writer = new Writer();
  if (type === "u64") writer.u64(asBigInt(value));
  else if (type === "u32") writer.u32(Number(value));
  else if (type === "u128") writer.u128(asBigInt(value));
  else if (type === "bool") writer.u8(value === true ? 1 : 0);
  else if (type === "address") {
    const bytes = asBytes(value);
    if (bytes.length !== 32) throw new Error("address must be 32 bytes");
    writer.push(bytes);
  } else {
    const bounded = parseBounded(type);
    if (!bounded) throw new Error("unsupported ABI type: " + type);
    const bytes =
      bounded.kind === "bytes"
        ? asBytes(value)
        : new TextEncoder().encode(String(value));
    if (bytes.length > bounded.max) throw new Error(type + " value exceeds its bound");
    writer.u32(bytes.length);
    writer.push(bytes);
  }
  return writer.finish();
}

export function encodeActionPayload(
  abi: JamScriptAbi,
  actionName: string,
  values: Record<string, CodecValue>,
): Uint8Array {
  const action = abi.actions.find((candidate) => candidate.name === actionName);
  if (!action) throw new Error("unknown JamScript action: " + actionName);
  const chunks = action.input.map((field: AbiField) => {
    const value = values[field.name];
    if (value === undefined) throw new Error("missing action field: " + field.name);
    return encodeValue(field.type, value);
  });
  const size = chunks.reduce((sum, chunk) => sum + chunk.length, 0);
  const output = new Uint8Array(size);
  let offset = 0;
  for (const chunk of chunks) {
    output.set(chunk, offset);
    offset += chunk.length;
  }
  return output;
}

export function decodeValue(type: string, bytes: Uint8Array): CodecValue {
  const reader = new Reader(bytes);
  let result: CodecValue;
  if (type === "u64") result = reader.u64();
  else if (type === "u32") result = reader.u32();
  else if (type === "u128") result = reader.u128();
  else if (type === "bool") result = reader.take(1)[0] !== 0;
  else if (type === "address") result = reader.take(32);
  else {
    const bounded = parseBounded(type);
    if (!bounded) throw new Error("unsupported ABI type: " + type);
    const value = reader.take(reader.u32());
    if (value.length > bounded.max) throw new Error(type + " value exceeds its bound");
    result = bounded.kind === "bytes" ? value : new TextDecoder().decode(value);
  }
  if (reader.remaining() !== 0) throw new Error("trailing bytes in ABI value");
  return result;
}

export function decodeStateValue(scaleEncoded: Uint8Array): Uint8Array {
  if (scaleEncoded.length === 0) throw new Error("empty StateValue");
  const first = scaleEncoded[0];
  let length: number;
  let offset: number;
  switch (first & 3) {
    case 0:
      length = first >>> 2;
      offset = 1;
      break;
    case 1:
      length = ((scaleEncoded[0] | (scaleEncoded[1] << 8)) >>> 2);
      offset = 2;
      break;
    case 2:
      length =
        (scaleEncoded[0] |
          (scaleEncoded[1] << 8) |
          (scaleEncoded[2] << 16) |
          (scaleEncoded[3] << 24)) >>>
        2;
      offset = 4;
      break;
    default:
      throw new Error("StateValue is too large for browser client");
  }
  if (offset + length !== scaleEncoded.length) throw new Error("invalid StateValue length");
  return scaleEncoded.slice(offset);
}
