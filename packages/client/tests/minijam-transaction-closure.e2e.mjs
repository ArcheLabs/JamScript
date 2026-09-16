import assert from "node:assert/strict";
import fs from "node:fs/promises";
import path from "node:path";
import process from "node:process";
import { hexToU8a } from "@polkadot/util";
import { cryptoWaitReady, sr25519PairFromSeed, sr25519Sign } from "@polkadot/util-crypto";
import {
  decodeStateValue,
  FetchRpcTransport,
  JamScriptClient,
  MANAGED_STATE_COMMITMENT_KEY_V1,
  stateKey,
  encodeValue,
  toHex,
  verifyManagedStateProof,
} from "../dist/index.js";

const backendUrl = process.env.JAMSCRIPT_E2E_BACKEND_URL ?? "http://127.0.0.1:8091";
const artifacts = process.env.JAMSCRIPT_E2E_ARTIFACTS;
const serviceId = Number(process.env.JAMSCRIPT_E2E_SERVICE_ID);
const serviceKey = process.env.JAMSCRIPT_E2E_SERVICE_KEY;
const codeHash = process.env.JAMSCRIPT_E2E_CODE_HASH;
const genesisHash = process.env.JAMSCRIPT_E2E_GENESIS_HASH;

if (!artifacts || !Number.isInteger(serviceId) || !serviceKey || !codeHash || !genesisHash) {
  throw new Error("transaction E2E deployment variables are incomplete");
}

const readJson = (name) => fs.readFile(path.join(artifacts, name), "utf8").then(JSON.parse);

async function managedValue(backend, key) {
  const context = await backend.call("minijam_getFinalizedContext");
  const encoded = await backend.call("minijam_getServiceStorageAt", [
    context.blockHash,
    serviceId,
    toHex(MANAGED_STATE_COMMITMENT_KEY_V1),
  ]);
  const commitment = decodeStateValue(hexToU8a(encoded));
  const root = commitment.slice(2);
  const response = await backend.call("minijam_getManagedStateV1", {
    serviceId,
    stateRoot: toHex(root),
    keyBase64: Buffer.from(key).toString("base64"),
  });
  return verifyManagedStateProof(
    root,
    key,
    response.valueBase64 === null ? null : Uint8Array.from(Buffer.from(response.valueBase64, "base64")),
    response.proofBase64.map((value) => Uint8Array.from(Buffer.from(value, "base64"))),
  );
}

function deployment(abi) {
  return { artifacts, genesisHash, serviceKey, serviceId, codeHash, abiVersion: 1, abi };
}

function signer(seed) {
  const pair = sr25519PairFromSeed(hexToU8a(`0x${seed.repeat(32)}`));
  return {
    pair,
    value: { publicKey: pair.publicKey, signRaw: async (message) => sr25519Sign(message, pair) },
  };
}

async function main() {
  await cryptoWaitReady();
  const abi = await readJson("service.abi.json");
  const backend = new FetchRpcTransport(backendUrl);
  const client = new JamScriptClient(deployment(abi), backend, { stateVerification: "proof" });
  const { pair, value: wallet } = signer("09");
  const firstKey = new Uint8Array(32).fill(0x11);
  const valueKey = stateKey("test.values/v1", encodeValue({ kind: "bytes", max: 32 }, new Uint8Array(32).fill(0x22)));

  assert.equal(await client.readNonce(pair.publicKey), 0n);
  const seed = await client.submitAction("seed", { key: firstKey, next: new Uint8Array(32).fill(0x22), value: 10 }, wallet);
  assert.match(seed.transactionId, /^0x[0-9a-f]{64}$/i);
  const seeded = await client.waitForAction(seed.transactionId, seed.actionHash, { intervalMs: 250, timeoutMs: 180_000 });
  assert.equal(seeded.status, "applied");
  assert.equal(seeded.transactionStatus, "imported");
  const seededValue = await managedValue(backend, valueKey);
  console.log(`SEEDED_VALUE_BYTES=${toHex(seededValue)}`);
  assert.equal(new DataView(seededValue.buffer, seededValue.byteOffset + 32, 4).getUint32(0, true), 10);
  console.log("SINGLE_ACTION_SERVICE_DEPLOY=PASS");
  console.log("LOGICAL_TRANSACTION_COUNT=1");
  console.log("SINGLE_ACTION_E2E=PASS");

  const batch = await Promise.all([
    client.submitAction("advance", { key: firstKey }, wallet),
    client.submitAction("advance", { key: firstKey }, wallet),
    client.submitAction("advance", { key: firstKey }, wallet),
  ]);
  assert.equal(new Set(batch.map((item) => item.transactionId)).size, 3);
  const batchResults = await Promise.all(batch.map((item) => client.waitForAction(item.transactionId, item.actionHash, { intervalMs: 250, timeoutMs: 180_000 })));
  assert.deepEqual(batchResults.map((item) => item.status), ["applied", "applied", "applied"]);
  assert.equal(new Set(batchResults.map((item) => item.packageHash)).size, 1);
  assert.deepEqual(batchResults.map((item) => item.actionIndex), [0, 1, 2]);
  const batchedValue = await managedValue(backend, valueKey);
  assert.equal(new DataView(batchedValue.buffer, batchedValue.byteOffset + 32, 4).getUint32(0, true), 13);
  console.log("CONCURRENT_LOGICAL_TX_IDS_UNIQUE=PASS");
  console.log("CONCURRENT_BATCH_ORDER_DETERMINISTIC=PASS");
  console.log("BATCHED_ACTION_E2E=PASS");

  const second = await Promise.all([
    client.submitAction("advance", { key: firstKey }, wallet),
    client.submitAction("advance", { key: firstKey }, wallet),
  ]);
  const secondResults = await Promise.all(second.map((item) => client.waitForAction(item.transactionId, item.actionHash, { intervalMs: 250, timeoutMs: 180_000 })));
  assert.deepEqual(secondResults.map((item) => item.actionIndex), [0, 1]);
  assert.deepEqual(secondResults.map((item) => item.status), ["applied", "applied"]);
  const secondValue = await managedValue(backend, valueKey);
  assert.equal(new DataView(secondValue.buffer, secondValue.byteOffset + 32, 4).getUint32(0, true), 15);
  console.log("BATCH_CHAIN_CONTINUITY=PASS");
  console.log("FINALIZED_STATE_MATCH=PASS");
  console.log("REAL_MINIJAM_E2E=PASS");
}

main().catch((error) => {
  console.error(error);
  process.exitCode = 1;
});
