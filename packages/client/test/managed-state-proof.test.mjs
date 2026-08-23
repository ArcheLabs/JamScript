import assert from "node:assert/strict";
import test from "node:test";
import { verifyManagedStateProof } from "../dist/index.js";

const root = hex("802e9d7821c66ad0f0747bf287bf46410819ca9f2aa787411522f26577e766bd");
const key = hex("01090073636f7265732f76310101010101010101010101010101010101010101010101010101010101010101");
const value = hex("2a00000000000000");
const proof = [hex("7f1901090073636f7265732f76310101010101010101010101010101010101010101010101010101010101010101202a00000000000000")];
const emptyRoot = hex("03170a2e7597b7b7e3d84c05391d139a62b157e78786d8c082f29dcf4c111314");
const emptyProof = [hex("00")];

test("verifies LayoutV1 inclusion and non-inclusion", () => {
  assert.deepEqual(verifyManagedStateProof(root, key, value, proof), value);
  assert.equal(verifyManagedStateProof(emptyRoot, key, null, emptyProof), null);
});

test("rejects wrong root, tampered proof, and a false provider value", () => {
  const wrongRoot = root.slice();
  wrongRoot[0] ^= 1;
  assert.throws(() => verifyManagedStateProof(wrongRoot, key, value, proof), /requested root/);

  const tampered = proof[0].slice();
  tampered[tampered.length - 1] ^= 1;
  assert.throws(() => verifyManagedStateProof(root, key, value, [tampered]), /requested root/);

  const falseValue = value.slice();
  falseValue[0] ^= 1;
  assert.throws(() => verifyManagedStateProof(root, key, falseValue, proof), /does not match/);
});

function hex(value) {
  return Uint8Array.from(value.match(/.{2}/g).map((byte) => Number.parseInt(byte, 16)));
}
