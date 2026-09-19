import assert from "node:assert/strict";
import fs from "node:fs";
import test from "node:test";
import { decodeValue, encodeValue } from "../dist/codec.js";
import { decodeOwnership, encodeOwnership, ownershipKey } from "../dist/crypto.js";

const vectors = JSON.parse(
  fs.readFileSync(new URL("../../../test-vectors/ownership-v1.json", import.meta.url)),
).vectors;

function bytes(hex) {
  return Uint8Array.from(hex.slice(2).match(/../g) ?? [], (pair) => Number.parseInt(pair, 16));
}

function hex(value) {
  return `0x${Buffer.from(value).toString("hex")}`;
}

test("Ownership canonical encoding and key match the shared vectors", () => {
  for (const vector of vectors) {
    const value = { version: 1, kind: vector.kind, public: bytes(vector.public) };
    assert.equal(hex(encodeOwnership(value)), vector.canonical, vector.name);
    assert.deepEqual(decodeOwnership(bytes(vector.canonical)), value, vector.name);
    assert.equal(hex(ownershipKey(value)), vector.key, vector.name);
    assert.equal(hex(encodeValue("ownership", value)), vector.canonical, vector.name);
    assert.deepEqual(decodeValue("ownership", bytes(vector.canonical)), value, vector.name);
  }
});

test("Ownership rejects non-canonical values", () => {
  assert.throws(() => decodeOwnership(Uint8Array.of(1, 0, 31, 0)), /invalid Ownership length/);
  assert.throws(() => encodeOwnership({ version: 1, kind: 2, public: new Uint8Array(33).fill(4) }), /compressed secp256k1/);
});
