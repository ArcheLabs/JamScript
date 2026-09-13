import { caughtResult } from "../runtime.js";
import {
  jamU8AddChecked, jamU8SubChecked, jamU8MulChecked, jamU8DivChecked, jamU8ModChecked, jamU8Compare,
  jamU16AddChecked, jamU16SubChecked, jamU16MulChecked, jamU16DivChecked, jamU16ModChecked, jamU16Compare,
  jamU32AddChecked, jamU32SubChecked, jamU32MulChecked, jamU32DivChecked, jamU32ModChecked, jamU32Compare,
} from "./runtime.js";

function one(value: number): Uint8Array { return new Uint8Array([value]); }
function encode16(value: number): Uint8Array { return new Uint8Array([value % 256, Math.floor(value / 256) % 256]); }
function encode32(value: number): Uint8Array {
  return new Uint8Array([
    value % 256,
    Math.floor(value / 256) % 256,
    Math.floor(value / 65536) % 256,
    Math.floor(value / 16777216) % 256,
  ]);
}
function read16(payload: Uint8Array, offset: number): number { return payload[offset] + payload[offset + 1] * 256; }
function read32(payload: Uint8Array, offset: number): number {
  return payload[offset] + payload[offset + 1] * 256 + payload[offset + 2] * 65536 + payload[offset + 3] * 16777216;
}

export function executeU8(payload: Uint8Array, _sender: Uint8Array, _state: Uint8Array): Uint8Array {
  try {
    if (payload.length !== 3) throw new Error("u8 conformance payload length");
    const left = payload[0]; const right = payload[1]; const operation = payload[2];
    if (operation === 0) return one(jamU8AddChecked(left, right));
    if (operation === 1) return one(jamU8SubChecked(left, right));
    if (operation === 2) return one(jamU8MulChecked(left, right));
    if (operation === 3) return one(jamU8DivChecked(left, right));
    if (operation === 4) return one(jamU8ModChecked(left, right));
    return one(jamU8Compare(left, right) < 0 ? 255 : jamU8Compare(left, right) > 0 ? 1 : 0);
  } catch (error) { return caughtResult(error); }
}

export function executeU16(payload: Uint8Array, _sender: Uint8Array, _state: Uint8Array): Uint8Array {
  try {
    if (payload.length !== 5) throw new Error("u16 conformance payload length");
    const left = read16(payload, 0); const right = read16(payload, 2); const operation = payload[4];
    if (operation === 0) return encode16(jamU16AddChecked(left, right));
    if (operation === 1) return encode16(jamU16SubChecked(left, right));
    if (operation === 2) return encode16(jamU16MulChecked(left, right));
    if (operation === 3) return encode16(jamU16DivChecked(left, right));
    if (operation === 4) return encode16(jamU16ModChecked(left, right));
    return one(jamU16Compare(left, right) < 0 ? 255 : jamU16Compare(left, right) > 0 ? 1 : 0);
  } catch (error) { return caughtResult(error); }
}

export function executeU32(payload: Uint8Array, _sender: Uint8Array, _state: Uint8Array): Uint8Array {
  try {
    if (payload.length !== 9) throw new Error("u32 conformance payload length");
    const left = read32(payload, 0); const right = read32(payload, 4); const operation = payload[8];
    if (operation === 0) return encode32(jamU32AddChecked(left, right));
    if (operation === 1) return encode32(jamU32SubChecked(left, right));
    if (operation === 2) return encode32(jamU32MulChecked(left, right));
    if (operation === 3) return encode32(jamU32DivChecked(left, right));
    if (operation === 4) return encode32(jamU32ModChecked(left, right));
    return one(jamU32Compare(left, right) < 0 ? 255 : jamU32Compare(left, right) > 0 ? 1 : 0);
  } catch (error) { return caughtResult(error); }
}
