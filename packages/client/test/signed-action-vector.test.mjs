import assert from "node:assert/strict";
import fs from "node:fs";
import test from "node:test";
import {
  actionSelector,
  encodeValue,
  encodeSignedAction,
  parseHex,
  signingDigest,
  toHex,
  decodeSignedActionV2,
  encodeSignedActionV2,
  signingDigestV2,
} from "../dist/index.js";

const vector = JSON.parse(
  fs.readFileSync(new URL("../../../test-vectors/signed-action-v1.json", import.meta.url)),
);

test("SignedActionV1 matches the Rust golden vector", () => {
  const unsigned = {
    version: 1,
    genesisHash: parseHex(vector.genesisHash, 32),
    serviceId: vector.serviceId,
    actionSelector: parseHex(vector.actionSelector, 8),
    signerScheme: 0,
    publicKey: parseHex(vector.publicKey, 32),
    nonce: BigInt(vector.nonce),
    validUntil: BigInt(vector.validUntil),
    payloadHash: parseHex(vector.payloadHash, 32),
    payload: parseHex(vector.payload),
  };
  const signed = { ...unsigned, signature: parseHex(vector.signature, 64) };
  assert.equal(toHex(actionSelector("increment")), vector.actionSelector);
  assert.equal(toHex(signingDigest(unsigned)), vector.signingDigest);
  assert.equal(toHex(encodeSignedAction(signed)), vector.encoded);
});

test("SignedActionV2 matches the Rust golden vector", () => {
  const vector = JSON.parse(
    fs.readFileSync(new URL("../../../test-vectors/signed-action-v2.json", import.meta.url)),
  );
  const unsigned = {
    version: 2,
    networkDomain: parseHex(vector.networkDomain, 32),
    serviceKey: parseHex(vector.serviceKey, 32),
    actionSelector: parseHex(vector.actionSelector, 8),
    signerScheme: 0,
    publicKey: parseHex(vector.publicKey, 32),
    nonce: BigInt(vector.nonce),
    validUntil: BigInt(vector.validUntil),
    payloadHash: parseHex(vector.payloadHash, 32),
    payload: parseHex(vector.payload),
  };
  const signed = { ...unsigned, signature: parseHex(vector.signature, 64) };
  assert.equal(toHex(signingDigestV2(unsigned)), vector.signingDigest);
  assert.equal(toHex(encodeSignedActionV2(signed)), vector.encoded);
  assert.deepEqual(decodeSignedActionV2(parseHex(vector.encoded)), signed);
  const tampered = parseHex(vector.encoded);
  tampered[tampered.length - 1] ^= 1;
  assert.throws(() => decodeSignedActionV2(tampered), /payload hash mismatch/);
});

test("ABI integer and boolean encoders reject silent coercion and wrap", () => {
  assert.throws(() => encodeValue("u32", -1), /u32 is out of range/);
  assert.throws(() => encodeValue("u32", 2 ** 32), /u32 is out of range/);
  assert.throws(() => encodeValue("u32", 1.5), /u32 must be/);
  assert.throws(() => encodeValue("u64", -1n), /u64 is out of range/);
  assert.throws(() => encodeValue("u64", 1n << 64n), /u64 is out of range/);
  assert.throws(() => encodeValue("u128", -1n), /u128 is out of range/);
  assert.throws(() => encodeValue("bool", 1), /bool must be a boolean/);
  assert.deepEqual([...encodeValue("bool", true)], [1]);
});
