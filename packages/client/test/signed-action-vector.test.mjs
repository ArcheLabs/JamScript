import assert from "node:assert/strict";
import fs from "node:fs";
import test from "node:test";
import {
  actionSelector,
  encodeSignedAction,
  parseHex,
  signingDigest,
  toHex,
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
