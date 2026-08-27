import assert from "node:assert/strict";
import fs from "node:fs/promises";
import path from "node:path";
import process from "node:process";
import { hexToU8a, u8aToHex } from "@polkadot/util";
import {
  blake2AsHex,
  cryptoWaitReady,
  sr25519PairFromSeed,
  sr25519Sign,
} from "@polkadot/util-crypto";
import {
  actionSelector,
  decodeStateValue,
  encodeActionPayload,
  encodeSignedActionV2,
  FetchRpcTransport,
  JamScriptClient,
  signingDigestV2,
  SplitRpcTransport,
  stateKey,
  toHex,
  verifyManagedStateProof,
} from "../dist/index.js";

const nodeEndpoint = process.env.MINIJAM_NODE_RPC ?? "http://127.0.0.1:9944";
const workEndpoint = process.env.MINIJAM_WORK_RPC ?? "http://127.0.0.1:8090";
const stateEndpoint = process.env.MINIJAM_STATE_RPC ?? workEndpoint;
const artifacts = process.env.JAMSCRIPT_E2E_ARTIFACTS;
const serviceId = Number(process.env.JAMSCRIPT_E2E_SERVICE_ID);
const serviceKey = process.env.JAMSCRIPT_E2E_SERVICE_KEY;
const codeHash = process.env.JAMSCRIPT_E2E_CODE_HASH;
const genesisHash = process.env.JAMSCRIPT_E2E_GENESIS_HASH;
const workerMetrics = [9616, 9617, 9618].map(
  (port) => "http://127.0.0.1:" + port + "/metrics",
);
const workerLogDir = process.env.JAMSCRIPT_E2E_LOG_DIR;

if (!artifacts || !Number.isInteger(serviceId) || !codeHash || !genesisHash) {
  throw new Error(
      "JAMSCRIPT_E2E_ARTIFACTS, JAMSCRIPT_E2E_SERVICE_ID, " +
      "JAMSCRIPT_E2E_CODE_HASH and JAMSCRIPT_E2E_GENESIS_HASH are required",
  );
}

class RecordingTransport {
  constructor(inner) {
    this.inner = inner;
    this.calls = [];
    this.lastSubmitWork = null;
  }

  async call(method, params) {
    this.calls.push(method);
    if (method === "minijam_submitWorkV1") {
      this.lastSubmitWork = structuredClone(params);
    }
    return this.inner.call(method, params);
  }
}

class CaptureAndAbortTransport extends RecordingTransport {
  async call(method, params) {
    if (method === "minijam_submitWorkV1") {
      this.lastSubmitWork = structuredClone(params);
      throw new Error("captured Work request");
    }
    return super.call(method, params);
  }
}

class StaleOnceTransport extends RecordingTransport {
  constructor(inner, staleContext) {
    super(inner);
    this.staleContext = staleContext;
    this.submittedPayloads = [];
  }

  async call(method, params) {
    if (method === "minijam_submitWorkV1") {
      this.submittedPayloads.push(params.payloadBase64);
      if (this.submittedPayloads.length === 1) {
        return super.call(method, { ...params, context: this.staleContext });
      }
    }
    return super.call(method, params);
  }
}

class TestSr25519Signer {
  constructor(pair) {
    this.pair = pair;
    this.publicKey = pair.publicKey;
    this.signRawCalls = 0;
  }

  async signRaw(message) {
    this.signRawCalls += 1;
    return sr25519Sign(message, this.pair);
  }
}

function contextParams(context) {
  return {
    blockHash: context.blockHash,
    stateRoot: context.stateRoot,
    slot: context.slot,
  };
}

function makeRun(steps) {
  const bytes = new Uint8Array(12 + steps.length * 4);
  const view = new DataView(bytes.buffer);
  view.setUint32(0, 1, true);
  view.setUint32(4, 100, true);
  view.setUint32(8, steps.length, true);
  steps.forEach((step, index) => {
    const offset = 12 + index * 4;
    bytes[offset] = step.opcode;
    bytes[offset + 1] = step.amount;
  });
  return bytes;
}

function tamperSignature(payloadBase64) {
  const bytes = Buffer.from(payloadBase64, "base64");
  let offset = 1 + 32 + 32 + 8 + 1;
  const publicKeyLength = bytes[offset];
  offset += 1 + publicKeyLength + 8 + 8 + 32;
  const signatureLength = bytes[offset];
  assert.equal(signatureLength, 64, "SignedActionV2 signature must be 64 bytes");
  bytes[offset + 1] ^= 1;
  return bytes.toString("base64");
}

function fromBase64(value) {
  return Uint8Array.from(Buffer.from(value, "base64"));
}

async function readMetrics() {
  const values = [];
  for (const endpoint of workerMetrics) {
    const response = await fetch(endpoint);
    const text = await response.text();
    const metrics = {};
    for (const match of text.matchAll(/^([a-zA-Z0-9_]+) ([-0-9]+)$/gm)) {
      metrics[match[1]] = Number(match[2]);
    }
    values.push(metrics);
  }
  return values;
}

async function readWorkerLogs() {
  if (!workerLogDir) return "";
  const names = ["worker-1.log", "worker-2.log", "worker-3.log"];
  const chunks = [];
  for (const name of names) {
    try {
      chunks.push(await fs.readFile(path.join(workerLogDir, name), "utf8"));
    } catch {
      // The launcher reports the complete log path if the run fails.
    }
  }
  return chunks.join("\n");
}

async function waitWorkerEvidence(before) {
  const deadline = Date.now() + 120_000;
  for (;;) {
    const after = await readMetrics();
    const bundleReady = after.some(
      (metrics, index) =>
        metrics.minijam_worker_bundle_ready_total >
        before[index].minijam_worker_bundle_ready_total,
    );
    const logs = await readWorkerLogs();
    const candidate = /submitted_candidates=[1-9][0-9]*/.test(logs);
    const vote = /vote_tasks_or_submitted=[1-9][0-9]*/.test(logs);
    if (bundleReady && candidate && vote) return;
    if (Date.now() >= deadline) {
      throw new Error(
        "worker evidence did not show bundle, candidate and vote activity",
      );
    }
    await new Promise((resolve) => setTimeout(resolve, 1000));
  }
}

function decodeServiceCodeHash(encodedHex) {
  const stateValue = decodeStateValue(hexToU8a(encodedHex));
  assert.ok(stateValue.length >= 33, "ServiceInfo is truncated");
  return u8aToHex(stateValue.slice(1, 33));
}

function readBuildMetadata() {
  return fs
    .readFile(path.join(artifacts, "build.json"), "utf8")
    .then((value) => JSON.parse(value));
}

function readAbi() {
  return fs
    .readFile(path.join(artifacts, "service.abi.json"), "utf8")
    .then((value) => {
      const raw = JSON.parse(value);
      return {
        ...raw,
        abiVersion: raw.abiVersion ?? raw.abi_version,
        languageVersion: raw.languageVersion ?? raw.language_version,
        actions: raw.actions.map((action) => ({
          ...action,
          executeOutput:
            action.executeOutput ??
            action.execute_output ??
            action.computeOutput ??
            action.compute_output,
        })),
      };
    });
}

async function main() {
  await cryptoWaitReady();
  const [metadata, abi] = await Promise.all([readBuildMetadata(), readAbi()]);
  const resolvedServiceKey = serviceKey ?? metadata.serviceKey ?? metadata.service_key;
  if (!resolvedServiceKey) throw new Error("build metadata does not contain serviceKey");
  assert.equal(metadata.code_hash.toLowerCase(), codeHash.toLowerCase());
  const deployment = {
    genesisHash,
    serviceKey: resolvedServiceKey,
    serviceId,
    codeHash,
    abiVersion: abi.abiVersion,
    abi,
  };

  const node = new RecordingTransport(new FetchRpcTransport(nodeEndpoint));
  const work = new RecordingTransport(new FetchRpcTransport(workEndpoint));
  const state = new RecordingTransport(new FetchRpcTransport(stateEndpoint));
  const transport = new SplitRpcTransport(node, work, state);
  const client = new JamScriptClient(deployment, transport);
  const pair = sr25519PairFromSeed(hexToU8a("0x" + "09".repeat(32)));
  const signer = new TestSr25519Signer(pair);

  function signedRawAction(nonce, validUntil, payload) {
    const unsigned = {
      version: 2,
      networkDomain: hexToU8a(genesisHash),
      serviceKey: hexToU8a(resolvedServiceKey),
      actionSelector: actionSelector("submitRun"),
      signerScheme: 0,
      publicKey: pair.publicKey,
      nonce: BigInt(nonce),
      validUntil: BigInt(validUntil),
      payloadHash: hexToU8a(blake2AsHex(payload, 256)),
      payload,
    };
    return encodeSignedActionV2({
      ...unsigned,
      signature: sr25519Sign(signingDigestV2(unsigned), pair),
    });
  }

  function workRequest(action, context) {
    return {
      context: contextParams(context),
      serviceId,
      serviceCodeHash: codeHash,
      payloadBase64: Buffer.from(action).toString("base64"),
      extrinsicsBase64: [],
    };
  }

  async function submitRaw(action, method = "minijam_submitWorkV1") {
    const context = await node.call("minijam_getFinalizedContext");
    const submitted = await work.call(method, workRequest(action, context));
    const status = await client.waitForWork(submitted.packageHash, {
      intervalMs: 500,
      timeoutMs: 120_000,
    });
    return { submitted, status };
  }

  async function prediction(packageHash) {
    return work.call("jamscript_testGetPredictionV1", { packageHash });
  }

  await client.validateDeployment();
  console.log("[network] genesis hash: " + genesisHash);
  const serviceContext = await node.call("minijam_getFinalizedContext");
  const serviceInfo = await node.call("minijam_getServiceInfoAt", [
    serviceContext.blockHash,
    serviceId,
  ]);
  assert.ok(serviceInfo, "finalized ServiceInfo is missing");
  assert.equal(
    decodeServiceCodeHash(serviceInfo).toLowerCase(),
    codeHash.toLowerCase(),
  );
  console.log("[provision] finalized code hash verified: " + codeHash);

  const nonceBefore = await client.readNonce(pair.publicKey);
  const scoreBefore = await client.query("getBestScore", pair.publicKey);
  assert.equal(nonceBefore, 0n);
  assert.equal(scoreBefore.value, null);
  console.log("[action] initial nonce: 0");
  console.log("[action] initial best score: null");

  const fullRun = makeRun([
    { opcode: 1, amount: 5 },
    { opcode: 2, amount: 10 },
    { opcode: 3, amount: 2 },
  ]);
  const expectedScore = 131n;
  assert.equal(expectedScore, 131n);
  const metricsBefore = await readMetrics();
  const submitted = await client.submitAction(
    "submitRun",
    { run: fullRun },
    signer,
  );
  assert.match(submitted.packageHash, /^0x[0-9a-f]{64}$/);
  assert.match(submitted.submissionHash, /^0x[0-9a-f]{64}$/);
  assert.equal(signer.signRawCalls, 1);
  console.log("[action] submitRun package: " + submitted.packageHash);
  const imported = await client.waitForWork(submitted.packageHash, {
    intervalMs: 500,
    timeoutMs: 120_000,
  });
  assert.equal(imported.status, "imported");
  assert.notEqual(imported.workId, null);
  assert.notEqual(imported.executionReceipt, null);
  const nonceAfter = await client.readNonce(pair.publicKey);
  const scoreAfter = await client.query("getBestScore", pair.publicKey);
  assert.equal(nonceAfter, 1n);
  assert.equal(scoreAfter.value, expectedScore);
  console.log("[action] finalized work: imported");
  console.log("[action] Work ID: " + imported.workId);
  console.log("[action] nonce: " + nonceAfter);
  console.log("[action] best score: " + scoreAfter.value);
  const historicalRoot = scoreAfter.stateRoot;

  assert.equal(node.calls.includes("minijam_submitWorkV1"), false);
  assert.equal(node.calls.includes("minijam_getWorkStatusV1"), false);
  assert.equal(work.calls.includes("chain_getBlockHash"), false);
  assert.equal(work.calls.includes("minijam_getFinalizedContext"), false);
  assert.equal(work.calls.includes("minijam_getServiceStorageAt"), false);
  assert.ok(work.calls.includes("minijam_submitWorkV1"));
  assert.ok(work.calls.includes("minijam_getWorkStatusV1"));

  const original = work.lastSubmitWork;
  assert.ok(original, "the signed Work request was not recorded");
  const replayContext = await node.call("minijam_getFinalizedContext");
  await assert.rejects(
    work.call("minijam_submitWorkV1", {
      ...original,
      context: contextParams(replayContext),
    }),
    (error) => error?.code === -32031,
  );
  assert.equal(await client.readNonce(pair.publicKey), 1n);
  assert.equal((await client.query("getBestScore", pair.publicKey)).value, 131n);
  console.log("[security] replay rejected by managed-state preflight; state unchanged");

  const capture = new CaptureAndAbortTransport(new FetchRpcTransport(workEndpoint));
  const captureClient = new JamScriptClient(
    deployment,
    new SplitRpcTransport(node, capture, state),
  );
  const lowRun = makeRun([{ opcode: 1, amount: 1 }]);
  await assert.rejects(
    captureClient.submitAction("submitRun", { run: lowRun }, signer),
    /captured Work request/,
  );
  assert.ok(capture.lastSubmitWork, "tamper request was not captured");
  const tamperContext = await node.call("minijam_getFinalizedContext");
  await assert.rejects(
    work.call("minijam_submitWorkV1", {
      ...capture.lastSubmitWork,
      context: contextParams(tamperContext),
      payloadBase64: tamperSignature(capture.lastSubmitWork.payloadBase64),
    }),
    (error) => error?.code === -32030,
  );
  assert.equal(await client.readNonce(pair.publicKey), 1n);
  assert.equal((await client.query("getBestScore", pair.publicKey)).value, 131n);
  console.log("[security] invalid signature rejected by preflight; state unchanged");

  const staleWork = new StaleOnceTransport(
    new FetchRpcTransport(workEndpoint),
    original.context,
  );
  const staleClient = new JamScriptClient(
    deployment,
    new SplitRpcTransport(node, staleWork, state),
  );
  const lower = await staleClient.submitAction("submitRun", { run: lowRun }, signer);
  const lowerResult = await staleClient.waitForWork(lower.packageHash, {
    intervalMs: 500,
    timeoutMs: 120_000,
  });
  assert.equal(lowerResult.status, "imported");
  assert.equal(await client.readNonce(pair.publicKey), 2n);
  const finalScore = await client.query("getBestScore", pair.publicKey);
  assert.equal(finalScore.value, 131n);
  assert.equal(staleWork.submittedPayloads.length, 2);
  assert.equal(staleWork.submittedPayloads[0], staleWork.submittedPayloads[1]);
  console.log("[action] lower score accepted");
  console.log("[action] lower package: " + lower.packageHash + " Work ID: " + lowerResult.workId);
  console.log("[action] nonce: 2");
  console.log("[action] best score remains: 131");
  console.log("[stale] same SignedAction rebuilt against refreshed finalized root");

  const logicalScoreKey = stateKey("best-score/v1", pair.publicKey);
  const historical = await state.call("minijam_getManagedStateV1", {
    serviceId,
    stateRoot: historicalRoot,
    keyBase64: Buffer.from(logicalScoreKey).toString("base64"),
  });
  const historicalValue = verifyManagedStateProof(
    hexToU8a(historicalRoot),
    logicalScoreKey,
    historical.valueBase64 === null ? null : fromBase64(historical.valueBase64),
    historical.proofBase64.map(fromBase64),
  );
  assert.equal(new DataView(historicalValue.buffer, historicalValue.byteOffset, 8).getBigUint64(0, true), 131n);
  console.log("[query] historical R1 proof remains valid after R2 finalization");

  const validPayload = encodeActionPayload(abi, "submitRun", { run: lowRun });
  const now = await node.call("minijam_getFinalizedContext");
  await assert.rejects(
    submitRaw(signedRawAction(99, now.slot + 64, validPayload)),
    (error) => error?.code === -32031,
  );
  assert.equal(await client.readNonce(pair.publicKey), 2n);
  assert.equal((await client.queryLatest("getBestScore", pair.publicKey)).value, 131n);
  console.log("[security] wrong nonce produced no state transition");

  const validSigned = signedRawAction(2, now.slot + 64, validPayload);
  const invalidSignature = Buffer.from(
    tamperSignature(Buffer.from(validSigned).toString("base64")),
    "base64",
  );
  await assert.rejects(
    submitRaw(invalidSignature),
    (error) => error?.code === -32030,
  );
  assert.equal(await client.readNonce(pair.publicKey), 2n);
  console.log("[security] invalid signature produced no state transition");

  const invalidNativeRun = makeRun([{ opcode: 9, amount: 1 }]);
  const invalidNativePayload = encodeActionPayload(abi, "submitRun", { run: invalidNativeRun });
  await assert.rejects(
    submitRaw(signedRawAction(2, now.slot + 64, invalidNativePayload)),
    (error) => error?.code === -32031 && error?.data?.errorCode === 0x80000009,
  );
  assert.equal(await client.readNonce(pair.publicKey), 2n);
  assert.equal((await client.queryLatest("getBestScore", pair.publicKey)).value, 131n);
  console.log("[security] native failure rolled back nonce with exact 0x80000009 code");

  const expiryContext = await node.call("minijam_getFinalizedContext");
  await assert.rejects(
    submitRaw(signedRawAction(2, Math.max(0, expiryContext.slot - 1), validPayload)),
    (error) => error?.code === -32032,
  );
  assert.equal(await client.readNonce(pair.publicKey), 2n);
  assert.equal((await client.queryLatest("getBestScore", pair.publicKey)).value, 131n);
  console.log("[security] expired transition was not accumulated");

  await assert.rejects(
    submitRaw(
      signedRawAction(2, expiryContext.slot + 64, validPayload),
      "jamscript_testSubmitTamperedWitnessV1",
    ),
    (error) => error?.code === -32033,
  );
  assert.equal(await client.readNonce(pair.publicKey), 2n);
  assert.equal((await client.queryLatest("getBestScore", pair.publicKey)).value, 131n);
  console.log("[security] tampered witness produced no state transition");

  const malformedPayload = new Uint8Array([0xff]);
  const applicationFailure = await submitRaw(
    signedRawAction(2, expiryContext.slot + 64, malformedPayload),
  );
  assert.equal(
    (await prediction(applicationFailure.submitted.packageHash)).receipts[0].errorCode,
    1,
  );
  assert.equal(await client.readNonce(pair.publicKey), 3n);
  assert.equal((await client.queryLatest("getBestScore", pair.publicKey)).value, 131n);
  console.log("[security] application failure consumed nonce under established semantics");

  await work.call("jamscript_testCorruptNextQueryV1", { mode: "value" });
  await assert.rejects(
    client.queryLatest("getBestScore", pair.publicKey),
    /does not match its proof/,
  );
  await work.call("jamscript_testCorruptNextQueryV1", { mode: "proof" });
  await assert.rejects(
    client.queryLatest("getBestScore", pair.publicKey),
    /requested root|invalid managed-state proof/,
  );
  console.log("[query] false value and bad proof rejected by client");

  await work.call("jamscript_testForgetProviderV1", {});
  const missingRootContext = await node.call("minijam_getFinalizedContext");
  await assert.rejects(
    work.call(
      "minijam_submitWorkV1",
      workRequest(
        signedRawAction(3, missingRootContext.slot + 64, validPayload),
        missingRootContext,
      ),
    ),
    (error) => error?.code === -32030,
  );
  console.log("[builder] missing canonical Provider root failed closed");

  await waitWorkerEvidence(metricsBefore);
  const logs = await readWorkerLogs();
  assert.match(logs, /submitted_candidates=[1-9][0-9]*/);
  assert.match(logs, /vote_tasks_or_submitted=[1-9][0-9]*/);
  console.log("[worker] bundle processed");
  console.log("[worker] candidate observed");
  console.log("[worker] vote observed");
  console.log("[network] node/formal RPC routing verified");
  console.log("[network] final query block/hash: " + finalScore.context.blockNumber + "/" + finalScore.context.blockHash);
  console.log("JamScript MiniJAM network E2E: PASS");
}

await main();
