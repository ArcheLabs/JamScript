import assert from "node:assert/strict";
import fs from "node:fs";
import test from "node:test";
import { decodeValue, encodeValue } from "../dist/codec.js";

const vectorFiles = ["primitives.json", "composites.json"];
const vectors = vectorFiles.flatMap((file) => JSON.parse(
  fs.readFileSync(new URL(`../../../test-vectors/abi-codec/${file}`, import.meta.url)),
));

function hexToBytes(hex) {
  assert.equal(hex.length % 2, 0);
  return Uint8Array.from(hex.match(/../g) ?? [], (pair) => Number.parseInt(pair, 16));
}

function normalize(type, value) {
  switch (type.kind) {
    case "unit": return undefined;
    case "u64": case "u128": case "i64": case "i128": return BigInt(value);
    case "address": case "fixedBytes": case "bytes": return hexToBytes(value);
    case "fixedArray": case "array": return value.map((item) => normalize(type.item, item));
    case "option": return value === null ? null : normalize(type.item, value);
    case "tuple": return value.map((item, index) => normalize(type.items[index], item));
    case "record": return Object.fromEntries(type.fields.map((field) => [field.name, normalize(field.type, value[field.name])]));
    case "enum": { const variant = type.variants.find((candidate) => candidate.index === value.index); return { index: value.index, value: normalize(variant.type, value.value) }; }
    case "result": return "ok" in value ? { ok: normalize(type.ok, value.ok) } : { err: normalize(type.err, value.err) };
    default: return value;
  }
}

test("shared ABI vectors are consumed by the TypeScript codec", () => {
  for (const vector of vectors) {
    const expected = hexToBytes(vector.encodedHex);
    assert.deepEqual(encodeValue(vector.type, normalize(vector.type, vector.value)), expected);
    assert.deepEqual(decodeValue(vector.type, expected), normalize(vector.type, vector.value));
  }
});

test("sequence boundaries use JAM natural encoding", () => {
  const cases = new Map([
    [0, [0x00]], [1, [0x01]], [127, [0x7f]], [128, [0x80, 0x80]],
    [255, [0x80, 0xff]], [256, [0x81, 0x00]],
    [16383, [0xbf, 0xff]], [16384, [0xc0, 0x00, 0x40]],
  ]);
  for (const [length, prefix] of cases) {
    const bytesType = { kind: "bytes", max: 16384 };
    const bytesValue = new Uint8Array(length).fill(0xaa);
    const bytesEncoded = encodeValue(bytesType, bytesValue);
    assert.deepEqual(bytesEncoded.slice(0, prefix.length), Uint8Array.from(prefix));
    assert.deepEqual(decodeValue(bytesType, bytesEncoded), bytesValue);

    const stringType = { kind: "string", max: 16384 };
    const stringValue = "a".repeat(length);
    const stringEncoded = encodeValue(stringType, stringValue);
    assert.deepEqual(stringEncoded.slice(0, prefix.length), Uint8Array.from(prefix));
    assert.equal(decodeValue(stringType, stringEncoded), stringValue);

    const arrayType = { kind: "array", item: { kind: "u8" }, max: 16384 };
    const arrayValue = Array(length).fill(0);
    const arrayEncoded = encodeValue(arrayType, arrayValue);
    assert.deepEqual(arrayEncoded.slice(0, prefix.length), Uint8Array.from(prefix));
    assert.deepEqual(decodeValue(arrayType, arrayEncoded), arrayValue);
  }
});

test("legacy bounded primitive spellings remain compatible", () => {
  for (const type of ["String<4>", "string<4>", "Bytes<4>", "bytes<4>"]) {
    assert.deepEqual(encodeValue(type, type.toLowerCase().includes("string") ? "猫" : Uint8Array.of(1)), type.toLowerCase().includes("string") ? Uint8Array.of(3, 0xe7, 0x8c, 0xab) : Uint8Array.of(1, 1));
  }
  for (const type of ["FixedBytes<2>", "fixedBytes<2>"]) assert.deepEqual(encodeValue(type, Uint8Array.of(1, 2)), Uint8Array.of(1, 2));
});

test("malformed ABI values are rejected", () => {
  assert.throws(() => decodeValue("bool", Uint8Array.of(2)), /invalid bool/);
  assert.throws(() => decodeValue({ kind: "option", item: { kind: "u8" } }, Uint8Array.of(2)), /invalid option/);
  assert.throws(() => decodeValue({ kind: "enum", variants: [{ name: "only", index: 0, type: { kind: "unit" } }] }, Uint8Array.of(1)), /invalid enum/);
  assert.throws(() => decodeValue({ kind: "result", ok: { kind: "unit" }, err: { kind: "unit" } }, Uint8Array.of(2)), /invalid result/);
  assert.throws(() => decodeValue("u32", Uint8Array.of(0)), /truncated/);
  assert.throws(() => decodeValue({ kind: "bytes", max: 4 }, Uint8Array.of(1)), /truncated/);
  assert.throws(() => decodeValue({ kind: "string", max: 4 }, Uint8Array.of(1, 0xff)), /utf-8/i);
  assert.throws(() => decodeValue({ kind: "array", item: { kind: "u8" }, max: 1 }, Uint8Array.of(2)), /exceeds/);
  assert.throws(() => decodeValue({ kind: "bytes", max: 1 }, Uint8Array.of(0, 1)), /trailing/);
  assert.throws(() => encodeValue({ kind: "bytes", max: 1 }, Uint8Array.of(1, 2)), /exceeds/);
  assert.throws(() => encodeValue({ kind: "fixedBytes", len: 2 }, Uint8Array.of(1)), /length/);
  assert.throws(() => encodeValue("address", Uint8Array.of(1)), /32 bytes/);
  assert.throws(() => encodeValue("u8", 256), /out of range/);
  assert.throws(() => encodeValue("i8", -129), /out of range/);
});
