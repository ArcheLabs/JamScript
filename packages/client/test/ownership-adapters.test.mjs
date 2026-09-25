import assert from "node:assert/strict";
import { createPrivateKey, createPublicKey, verify as verifySignature } from "node:crypto";
import test from "node:test";
import { encodeMatrixControlClaimProofV1 } from "../dist/ownership/matrix/proof.js";
import { createMatrixOwnershipAdapter } from "../dist/ownership/matrix/service.js";
import { OWNERSHIP_KIND } from "../dist/crypto.js";

const proofFixture = {
  userId: "@alice:example.org",
  selfSigningPublicKey: fromHex("8139770ea87d175f56a35466c34c7ecccb8d8a91b4ee37a25df60f5b8fc9b394"),
  masterSignature: fromHex("4b46de57790ce84e2c308ff13e3078bdb8c20b7026b0ad22ab754f7a798e3292c87e72d01d1be3dad936ce1e1ec9ca3f4f6298a3a79dffdfbed94814a31c2607"),
  deviceId: "DEVICE",
  algorithms: ["m.olm.v1.curve25519-aes-sha2"],
  deviceCurve25519Key: new Uint8Array(32).fill(4),
  deviceEd25519Key: fromHex("ed4928c628d1c2c6eae90338905995612959273a5c63f93636c14614ac8737d1"),
  selfSigningSignature: fromHex("024da97d568567c89a2b3fe71fe720122c3bbfff2873246d9ba6a3ff8a76fb97e6d82534a03eccaede5d81189bed267eb458b009f4a8c7c155c296832919450d"),
};

test("Matrix Ownership adapter verifies M→S→D using generic Ed25519", () => {
  const adapter = createMatrixOwnershipAdapter(verifyEd25519);
  const subject = { version: 1, kind: OWNERSHIP_KIND.ED25519_KEY, public: publicFromSeed(1) };
  const controller = { version: 1, kind: OWNERSHIP_KIND.ED25519_KEY, public: proofFixture.deviceEd25519Key };
  const encoded = encodeMatrixControlClaimProofV1(proofFixture);

  assert.equal(adapter.verify(subject, controller, encoded), true);

  const badMaster = { ...proofFixture, masterSignature: proofFixture.masterSignature.slice() };
  badMaster.masterSignature[0] ^= 1;
  assert.equal(adapter.verify(subject, controller, encodeMatrixControlClaimProofV1(badMaster)), false);

  const badDevice = { ...proofFixture, selfSigningSignature: proofFixture.selfSigningSignature.slice() };
  badDevice.selfSigningSignature[0] ^= 1;
  assert.equal(adapter.verify(subject, controller, encodeMatrixControlClaimProofV1(badDevice)), false);

  assert.equal(adapter.verify({ ...subject, public: new Uint8Array(32).fill(9) }, controller, encoded), false);
  assert.equal(adapter.verify(subject, { ...controller, public: new Uint8Array(32).fill(9) }, encoded), false);
  assert.equal(adapter.verify(subject, controller, encoded.slice(0, -1)), false);
  assert.equal(adapter.verify(subject, controller, Uint8Array.from([...encoded, 0])), false);
  assert.equal(adapter.verify(subject, controller, Uint8Array.of(2)), false);
});

function verifyEd25519(publicKey, message, signature) {
  try {
    const key = createPublicKey({
      key: Buffer.concat([Buffer.from("302a300506032b6570032100", "hex"), Buffer.from(publicKey)]),
      format: "der",
      type: "spki",
    });
    return verifySignature(null, Buffer.from(message), key, Buffer.from(signature));
  } catch {
    return false;
  }
}

function publicFromSeed(seedByte) {
  const privateKey = createPrivateKey({
    key: Buffer.concat([Buffer.from("302e020100300506032b657004220420", "hex"), Buffer.alloc(32, seedByte)]),
    format: "der",
    type: "pkcs8",
  });
  return new Uint8Array(createPublicKey(privateKey).export({ format: "der", type: "spki" }).subarray(-32));
}

function fromHex(value) {
  return Uint8Array.from(Buffer.from(value, "hex"));
}
