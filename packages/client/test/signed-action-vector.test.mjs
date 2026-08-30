import assert from "node:assert/strict";
import fs from "node:fs";
import test from "node:test";
import {
  encodeValue,
  parseHex,
  toHex,
  decodeSignedActionV1,
  encodeSignedActionV1,
  signingDigestV1,
} from "../dist/index.js";
import { blake2AsU8a } from "@polkadot/util-crypto";

const vector = JSON.parse(
  fs.readFileSync(new URL("../../../test-vectors/signed-action-v1.json", import.meta.url)),
);

test("SignedActionV1 matches the Rust golden vector", () => {
  const unsigned = {
    version: 1,
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
  assert.equal(toHex(signingDigestV1(unsigned)), vector.signingDigest);
  assert.equal(toHex(encodeSignedActionV1(signed)), vector.encoded);
  assert.equal(toHex(blake2AsU8a(parseHex(vector.encoded), 256)), vector.actionHash);
  assert.deepEqual(decodeSignedActionV1(parseHex(vector.encoded)), signed);
  const tampered = parseHex(vector.encoded);
  tampered[tampered.length - 1] ^= 1;
  assert.throws(() => decodeSignedActionV1(tampered), /payload hash mismatch/);
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

test("JamScript Bytes ABI uses JAM general-natural length", () => {
  assert.deepEqual(
    [...encodeValue("Bytes<64>", Uint8Array.of(0xaa, 0xbb, 0xcc))],
    [3, 0xaa, 0xbb, 0xcc],
  );
  assert.deepEqual(
    [...encodeValue("Bytes<1024>", new Uint8Array(128)).slice(0, 4)],
    [128, 128, 0, 0],
  );
});
