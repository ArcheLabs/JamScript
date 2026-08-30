import assert from "node:assert/strict";
import fs from "node:fs/promises";
import path from "node:path";
import process from "node:process";
import { hexToU8a } from "@polkadot/util";
import {
  cryptoWaitReady,
  sr25519PairFromSeed,
  sr25519Sign,
} from "@polkadot/util-crypto";
import {
  decodeStateValue,
  FetchRpcTransport,
  JamScriptClient,
  MANAGED_STATE_COMMITMENT_KEY_V1,
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

if (!artifacts || !Number.isInteger(serviceId) || !serviceKey || !codeHash || !genesisHash) {
  throw new Error(
    "JAMSCRIPT_E2E_ARTIFACTS, JAMSCRIPT_E2E_SERVICE_ID, " +
      "JAMSCRIPT_E2E_SERVICE_KEY, JAMSCRIPT_E2E_CODE_HASH and " +
      "JAMSCRIPT_E2E_GENESIS_HASH are required",
  );
}

function readJson(name) {
  return fs
    .readFile(path.join(artifacts, name), "utf8")
    .then((value) => JSON.parse(value));
}

async function managedStateValue(node, state, key) {
  const context = await node.call("minijam_getFinalizedContext");
  const encodedCommitment = await node.call("minijam_getServiceStorageAt", [
    context.blockHash,
    serviceId,
    toHex(MANAGED_STATE_COMMITMENT_KEY_V1),
  ]);
  assert.ok(encodedCommitment, "managed-state commitment is missing");
  const commitment = decodeStateValue(hexToU8a(encodedCommitment));
  assert.equal(commitment.length, 34);
  assert.deepEqual(Array.from(commitment.slice(0, 2)), [1, 1]);
  const stateRoot = commitment.slice(2);
  const response = await state.call("minijam_getManagedStateV1", {
    serviceId,
    stateRoot: toHex(stateRoot),
    keyBase64: Buffer.from(key).toString("base64"),
  });
  return verifyManagedStateProof(
    stateRoot,
    key,
    response.valueBase64 === null
      ? null
      : Uint8Array.from(Buffer.from(response.valueBase64, "base64")),
    response.proofBase64.map((value) => Uint8Array.from(Buffer.from(value, "base64"))),
  );
}

async function main() {
  await cryptoWaitReady();
  const [metadata, abi] = await Promise.all([readJson("build.json"), readJson("service.abi.json")]);
  assert.equal(metadata.language_version ?? metadata.languageVersion, "0.2");
  assert.equal(metadata.runtime_profile_version, "scriptc-deterministic-v1");
  assert.equal(metadata.runtimeRefineInputVersion, 1);
  assert.equal(abi.abiVersion ?? abi.abi_version, 1);
  assert.equal(abi.languageVersion ?? abi.language_version, "0.2");

  const deployment = {
    genesisHash,
    serviceKey,
    serviceId,
    codeHash,
    abiVersion: abi.abiVersion ?? abi.abi_version,
    abi,
  };
  const node = new FetchRpcTransport(nodeEndpoint);
  const work = new FetchRpcTransport(workEndpoint);
  const state = new FetchRpcTransport(stateEndpoint);
  const client = new JamScriptClient(
    deployment,
    new SplitRpcTransport(node, work, state),
  );
  const pair = sr25519PairFromSeed(hexToU8a("0x" + "09".repeat(32)));
  const signer = {
    publicKey: pair.publicKey,
    signRaw: async (message) => sr25519Sign(message, pair),
  };
  const key1 = new Uint8Array(32).fill(0x11);
  const key2 = new Uint8Array(32).fill(0x22);

  await client.validateDeployment();
  assert.equal(await client.readNonce(pair.publicKey), 0n);

  const seed = await client.submitAction(
    "seed",
    { key: key1, next: key2, value: 10 },
    signer,
  );
  const seedResult = await client.waitForAction(seed.packageHash, seed.actionHash, {
    intervalMs: 500,
    timeoutMs: 120_000,
  });
  assert.equal(seedResult.status, "imported");
  assert.equal(await client.readNonce(pair.publicKey), 1n);
  const valueKey = stateKey("test.values/v1", key2);
  const seeded = await managedStateValue(node, state, valueKey);
  assert.ok(seeded);
  assert.deepEqual(Array.from(seeded.slice(0, 32)), Array.from(pair.publicKey));
  assert.equal(new DataView(seeded.buffer, seeded.byteOffset + 32, 4).getUint32(0, true), 10);
  console.log("[dynamic] seed inserted authenticated value at the second-order key");

  const advance = await client.submitAction("advance", { key: key1 }, signer);
  const advanceResult = await client.waitForAction(advance.packageHash, advance.actionHash, {
    intervalMs: 500,
    timeoutMs: 120_000,
  });
  assert.equal(advanceResult.status, "imported");
  assert.equal(await client.readNonce(pair.publicKey), 2n);
  const advanced = await managedStateValue(node, state, valueKey);
  assert.ok(advanced);
  assert.deepEqual(Array.from(advanced.slice(0, 32)), Array.from(pair.publicKey));
  assert.equal(new DataView(advanced.buffer, advanced.byteOffset + 32, 4).getUint32(0, true), 11);
  console.log("[dynamic] advance followed the authenticated pointer and committed the canonical root");
  console.log("REAL_MINIJAM_E2E=PASS");
}

main().catch((error) => {
  console.error(error);
  process.exitCode = 1;
});
