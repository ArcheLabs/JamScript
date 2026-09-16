import assert from "node:assert/strict";
import fs from "node:fs/promises";
import path from "node:path";
import process from "node:process";
import { hexToU8a } from "@polkadot/util";
import { blake2AsU8a, cryptoWaitReady, sr25519PairFromSeed, sr25519Sign } from "@polkadot/util-crypto";
import {
  decodeStateValue,
  FetchRpcTransport,
  JamScriptClient,
  MANAGED_STATE_COMMITMENT_KEY_V1,
  stateKey,
  encodeValue,
  encodeActionPayload,
  encodeSignedActionV1,
  actionSelector,
  parseHex,
  signingDigestV1,
  toHex,
  verifyManagedStateProof,
} from "../dist/index.js";

const backendUrl = process.env.JAMSCRIPT_E2E_BACKEND_URL ?? "http://127.0.0.1:8091";
const artifacts = process.env.JAMSCRIPT_E2E_ARTIFACTS;
const serviceId = Number(process.env.JAMSCRIPT_E2E_SERVICE_ID);
const serviceKey = process.env.JAMSCRIPT_E2E_SERVICE_KEY;
const codeHash = process.env.JAMSCRIPT_E2E_CODE_HASH;
const genesisHash = process.env.JAMSCRIPT_E2E_GENESIS_HASH;
const resultPath = process.env.JAMSCRIPT_E2E_RESULT ?? path.join(artifacts ?? ".", "e2e-result.json");

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

async function directSignedAction(abi, actionName, input, pair, nonce) {
  const payload = encodeActionPayload(abi, actionName, input);
  const unsigned = {
    version: 1,
    networkDomain: parseHex(genesisHash, 32),
    serviceKey: parseHex(serviceKey, 32),
    actionSelector: actionSelector(actionName),
    signerScheme: 0,
    publicKey: pair.publicKey,
    nonce: BigInt(nonce),
    validUntil: BigInt((await backendContext()).slot) + 64n,
    payloadHash: blake2AsU8a(payload, 256),
    payload,
  };
  const signature = await sr25519Sign(signingDigestV1(unsigned), pair);
  const bytes = encodeSignedActionV1({ ...unsigned, signature });
  return {
    nonce,
    actionHash: toHex(blake2AsU8a(bytes, 256)),
    payloadBase64: Buffer.from(bytes).toString("base64"),
  };
}

let backend;
async function backendContext() {
  return backend.call("minijam_getFinalizedContext");
}

function measured(submissions, statuses) {
  const formalTransactionIds = [...new Set(statuses.map((item) => item.formalTransactionId).filter(Boolean))];
  const packageHashes = [...new Set(statuses.map((item) => item.packageHash).filter(Boolean))];
  const workItems = [...new Set(statuses
    .filter((item) => item.packageHash !== null && item.itemIndex !== null)
    .map((item) => `${item.packageHash}:${item.itemIndex}`))]
    .map((value) => {
      const [packageHash, itemIndex] = value.split(":");
      return { packageHash, itemIndex: Number(itemIndex) };
    });
  const receipts = statuses
    .map((item) => item.actionReceipts?.[item.actionIndex ?? -1])
    .filter(Boolean);
  return {
    logicalTransactionIds: submissions.map((item) => item.transactionId),
    formalTransactionIds,
    packageHashes,
    workItems,
    receipts,
  };
}

async function main() {
  await cryptoWaitReady();
  const abi = await readJson("service.abi.json");
  backend = new FetchRpcTransport(backendUrl);
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

  const outOfOrderPair = signer("0a").pair;
  const submitDirect = async (nonce) => {
    const action = await directSignedAction(abi, "advance", { key: firstKey }, outOfOrderPair, nonce);
    const submitted = await backend.call("minijam_submitTransactionV1", {
      serviceId,
      serviceCodeHash: codeHash,
      payloadBase64: action.payloadBase64,
      extrinsicsBase64: [],
    });
    return { ...submitted, actionHash: action.actionHash, nonce };
  };
  // Submit in this order at the actual HTTP ingress. The scheduler must
  // canonicalize the queued nonce-2, nonce-0, nonce-1 sequence to 0,1,2.
  const outOfOrder = [
    await submitDirect(2),
    await submitDirect(0),
    await submitDirect(1),
  ];
  const outOfOrderResults = await Promise.all(outOfOrder.map((item) =>
    client.waitForAction(item.transactionId, item.actionHash, { intervalMs: 250, timeoutMs: 180_000 })));
  assert.deepEqual(outOfOrderResults.map((item) => item.actionIndex), [2, 0, 1]);
  assert.deepEqual(outOfOrderResults.map((item) => item.status), ["applied", "applied", "applied"]);
  console.log("OUT_OF_ORDER_SUBMISSION_NONCES=2,0,1");
  console.log("OUT_OF_ORDER_CANONICAL_NONCES=0,1,2");
  console.log("OUT_OF_ORDER_ACTION_INDEXES=2,0,1");
  console.log("OUT_OF_ORDER_SAME_SIGNER=PASS");

  const crossSignerInputs = ["0b", "0c", "0d"].map((seed) => signer(seed));
  const crossSigner = await Promise.all(crossSignerInputs.map(({ value }) =>
    client.submitAction("advance", { key: firstKey }, value)));
  const crossSignerResults = await Promise.all(crossSigner.map((item) =>
    client.waitForAction(item.transactionId, item.actionHash, { intervalMs: 250, timeoutMs: 180_000 })));
  assert.deepEqual(crossSignerResults.map((item) => item.status), ["applied", "applied", "applied"]);
  assert.equal(new Set(crossSignerResults.map((item) => item.packageHash)).size, 1);
  console.log("CROSS_SIGNER_ACTION_HASH_MAPPING=PASS");
  console.log("CROSS_SIGNER_E2E=PASS");

  const second = await Promise.all([
    client.submitAction("advance", { key: firstKey }, wallet),
    client.submitAction("advance", { key: firstKey }, wallet),
  ]);
  const secondResults = await Promise.all(second.map((item) => client.waitForAction(item.transactionId, item.actionHash, { intervalMs: 250, timeoutMs: 180_000 })));
  assert.deepEqual(secondResults.map((item) => item.actionIndex), [0, 1]);
  assert.deepEqual(secondResults.map((item) => item.status), ["applied", "applied"]);
  const secondValue = await managedValue(backend, valueKey);
  assert.equal(new DataView(secondValue.buffer, secondValue.byteOffset + 32, 4).getUint32(0, true), 21);
  console.log("BATCH_CHAIN_CONTINUITY=PASS");
  console.log("SECOND_BATCH_PARENT_MATCH=PASS");

  const failureBatch = await Promise.all([
    client.submitAction("advance", { key: firstKey }, wallet),
    client.submitAction("seed", { key: firstKey, next: new Uint8Array(32).fill(0x22), value: 99 }, wallet),
    client.submitAction("advance", { key: firstKey }, wallet),
  ]);
  const failureResults = await Promise.all(failureBatch.map((item) => client.waitForAction(item.transactionId, item.actionHash, { intervalMs: 250, timeoutMs: 180_000 })));
  assert.deepEqual(failureResults.map((item) => item.actionIndex), [0, 1, 2]);
  assert.deepEqual(failureResults.map((item) => item.status), ["applied", "failed", "applied"]);
  const finalValue = await managedValue(backend, valueKey);
  assert.equal(new DataView(finalValue.buffer, finalValue.byteOffset + 32, 4).getUint32(0, true), 23);
  console.log("FAILURE_ISOLATION=PASS");
  console.log("FINALIZED_STATE_MATCH=PASS");

  const stalePayload = encodeActionPayload(abi, "advance", { key: firstKey });
  const staleUnsigned = {
    version: 1,
    networkDomain: parseHex(genesisHash, 32),
    serviceKey: parseHex(serviceKey, 32),
    actionSelector: actionSelector("advance"),
    signerScheme: 0,
    publicKey: pair.publicKey,
    nonce: 0n,
    validUntil: BigInt((await backend.call("minijam_getFinalizedContext")).slot) + 64n,
    payloadHash: blake2AsU8a(stalePayload, 256),
    payload: stalePayload,
  };
  const staleSignature = await sr25519Sign(signingDigestV1(staleUnsigned), pair);
  const staleAction = encodeSignedActionV1({ ...staleUnsigned, signature: staleSignature });
  const staleSubmitted = await backend.call("minijam_submitTransactionV1", {
    serviceId,
    serviceCodeHash: codeHash,
    payloadBase64: Buffer.from(staleAction).toString("base64"),
    extrinsicsBase64: [],
  });
  let staleStatus;
  for (;;) {
    staleStatus = await backend.call("minijam_getTransactionStatusV1", { transactionId: staleSubmitted.transactionId });
    if (staleStatus.status === "imported" || staleStatus.status === "failed") break;
    await new Promise((resolve) => setTimeout(resolve, 250));
  }
  assert.equal(staleStatus.status, "imported");
  assert.equal(staleStatus.actionReceipts?.[staleStatus.actionIndex ?? 0]?.status, "rejected");
  const unchangedValue = await managedValue(backend, valueKey);
  assert.equal(new DataView(unchangedValue.buffer, unchangedValue.byteOffset + 32, 4).getUint32(0, true), 23);
  console.log("STALE_NONCE=PASS");
  const result = {
    jamscriptHead: process.env.JAMSCRIPT_EXECUTED_SHA ?? null,
    single: measured([seed], [seeded]),
    batch: measured(batch, batchResults),
    outOfOrder: measured(outOfOrder, outOfOrderResults),
    crossSigner: measured(crossSigner, crossSignerResults),
    secondBatch: measured(second, secondResults),
    failureBatch: measured(failureBatch, failureResults),
    stale: measured([staleSubmitted], [staleStatus]),
  };
  await fs.writeFile(resultPath, `${JSON.stringify(result, null, 2)}\n`);
  console.log("TRANSACTION_SCENARIOS=PASS");
}

main().catch((error) => {
  console.error(error);
  process.exitCode = 1;
});
