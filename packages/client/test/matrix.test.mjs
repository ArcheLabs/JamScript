import assert from "node:assert/strict";
import test from "node:test";
import { decodeMatrixControlClaimProofV1, encodeMatrixControlClaimProofV1 } from "../dist/matrix.js";

const bytes = (length, seed) => Uint8Array.from({ length }, (_, index) => (index + seed) & 0xff);
const hexBytes = (value) => Uint8Array.from(value.match(/../g), (pair) => Number.parseInt(pair, 16));

const fixture = {
  userId: "@alice:example.org",
  selfSigningPublicKey: hexBytes("8139770ea87d175f56a35466c34c7ecccb8d8a91b4ee37a25df60f5b8fc9b394"),
  masterSignature: hexBytes("4b46de57790ce84e2c308ff13e3078bdb8c20b7026b0ad22ab754f7a798e3292c87e72d01d1be3dad936ce1e1ec9ca3f4f6298a3a79dffdfbed94814a31c2607"),
  deviceId: "DEVICE",
  algorithms: ["m.olm.v1.curve25519-aes-sha2"],
  deviceCurve25519Key: new Uint8Array(32).fill(4),
  deviceEd25519Key: hexBytes("ed4928c628d1c2c6eae90338905995612959273a5c63f93636c14614ac8737d1"),
  selfSigningSignature: hexBytes("024da97d568567c89a2b3fe71fe720122c3bbfff2873246d9ba6a3ff8a76fb97e6d82534a03eccaede5d81189bed267eb458b009f4a8c7c155c296832919450d"),
};

test("MatrixControlClaimProofV1 matches the Rust wire layout", () => {
  const encoded = encodeMatrixControlClaimProofV1(fixture);
  const expectedHex = "01120040616c6963653a6578616d706c652e6f72678139770ea87d175f56a35466c34c7ecccb8d8a91b4ee37a25df60f5b8fc9b3944b46de57790ce84e2c308ff13e3078bdb8c20b7026b0ad22ab754f7a798e3292c87e72d01d1be3dad936ce1e1ec9ca3f4f6298a3a79dffdfbed94814a31c26070600444556494345011c006d2e6f6c6d2e76312e637572766532353531392d6165732d736861320404040404040404040404040404040404040404040404040404040404040404ed4928c628d1c2c6eae90338905995612959273a5c63f93636c14614ac8737d1024da97d568567c89a2b3fe71fe720122c3bbfff2873246d9ba6a3ff8a76fb97e6d82534a03eccaede5d81189bed267eb458b009f4a8c7c155c296832919450d";
  assert.equal(encoded.length, 284);
  assert.equal(Buffer.from(encoded).toString("hex"), expectedHex);
  assert.deepEqual(decodeMatrixControlClaimProofV1(encoded), fixture);
  assert.equal(encoded[0], 1);
  assert.equal(encoded[1], fixture.userId.length);
  assert.equal(encoded[2], 0);
  assert.equal(encoded[encoded.length - 64], 0x02);
});

test("MatrixControlClaimProofV1 rejects malformed text and lengths", () => {
  assert.throws(() => encodeMatrixControlClaimProofV1({ ...fixture, userId: "@alice\\:example.org" }), /invalid Matrix user id/);
  assert.throws(() => encodeMatrixControlClaimProofV1({ ...fixture, deviceEd25519Key: bytes(31, 4) }), /32 bytes/);
  const encoded = encodeMatrixControlClaimProofV1(fixture);
  assert.throws(() => decodeMatrixControlClaimProofV1(Uint8Array.from([...encoded, 0])), /trailing Matrix proof bytes/);
});
