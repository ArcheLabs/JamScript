import assert from "node:assert/strict";
import fs from "node:fs";
import test from "node:test";
import {
  decodeRuntimeRefineOutputV1,
  encodeRuntimeRefineOutputV1,
  parseHex,
  toHex,
} from "../dist/index.js";

const vector = JSON.parse(
  fs.readFileSync(new URL("../../../test-vectors/runtime-refine-output-v1.json", import.meta.url)),
);

test("RuntimeRefineOutputV1 matches the Rust golden vector", () => {
  const output = {
    version: 1,
    parentRoot: parseHex(vector.parentRoot, 32),
    newRoot: parseHex(vector.newRoot, 32),
    transitionValidUntil: BigInt(vector.transitionValidUntil),
    recoveryCommitment: parseHex(vector.recoveryCommitment, 32),
    receipts: vector.receipts.map((receipt) => ({
      actionHash: parseHex(receipt.actionHash, 32),
      status: receipt.status,
      errorCode: receipt.errorCode,
    })),
    recoveryPayload: parseHex(vector.recoveryPayload),
  };
  assert.equal(toHex(encodeRuntimeRefineOutputV1(output)), vector.encoded);
  assert.deepEqual(decodeRuntimeRefineOutputV1(parseHex(vector.encoded)), output);
  assert.throws(
    () => decodeRuntimeRefineOutputV1(parseHex(vector.encoded + "00")),
    /trailing RuntimeRefineOutputV1 bytes/,
  );
});
