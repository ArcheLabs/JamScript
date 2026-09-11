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
  stateKey,
  toHex,
  verifyManagedStateProof,
} from "../dist/index.js";

const backendEndpoint = process.env.JAMSCRIPT_E2E_BACKEND_URL ?? "http://127.0.0.1:8091";
const genesisHash = process.env.JAMSCRIPT_E2E_GENESIS_HASH;
const artifactsA = process.env.JAMSCRIPT_E2E_ARTIFACTS;
const serviceIdA = Number(process.env.JAMSCRIPT_E2E_SERVICE_ID);
const serviceKeyA = process.env.JAMSCRIPT_E2E_SERVICE_KEY;
const codeHashA = process.env.JAMSCRIPT_E2E_CODE_HASH;
const artifactsB = process.env.JAMSCRIPT_E2E_ARTIFACTS_B;
const serviceIdB = Number(process.env.JAMSCRIPT_E2E_SERVICE_ID_B);
const serviceKeyB = process.env.JAMSCRIPT_E2E_SERVICE_KEY_B;
const codeHashB = process.env.JAMSCRIPT_E2E_CODE_HASH_B;

if (!artifactsA || !Number.isInteger(serviceIdA) || !serviceKeyA || !codeHashA || !genesisHash) {
  throw new Error(
    "JAMSCRIPT_E2E_ARTIFACTS, JAMSCRIPT_E2E_SERVICE_ID, " +
      "JAMSCRIPT_E2E_SERVICE_KEY, JAMSCRIPT_E2E_CODE_HASH and " +
      "JAMSCRIPT_E2E_GENESIS_HASH are required",
  );
}
const hasServiceB = Boolean(artifactsB || serviceKeyB || codeHashB || Number.isFinite(serviceIdB));
if (hasServiceB && (!artifactsB || !Number.isInteger(serviceIdB) || !serviceKeyB || !codeHashB)) {
  throw new Error(
    "JAMSCRIPT_E2E_ARTIFACTS_B, JAMSCRIPT_E2E_SERVICE_ID_B, " +
      "JAMSCRIPT_E2E_SERVICE_KEY_B and JAMSCRIPT_E2E_CODE_HASH_B must be provided together",
  );
}

function readJsonFrom(directory, name) {
  return fs
    .readFile(path.join(directory, name), "utf8")
    .then((value) => JSON.parse(value));
}

async function managedStateValue(backend, deployment, key) {
  const context = await backend.call("minijam_getFinalizedContext");
  const encodedCommitment = await backend.call("minijam_getServiceStorageAt", [
    context.blockHash,
    deployment.serviceId,
    toHex(MANAGED_STATE_COMMITMENT_KEY_V1),
  ]);
  assert.ok(encodedCommitment, "managed-state commitment is missing");
  const commitment = decodeStateValue(hexToU8a(encodedCommitment));
  assert.equal(commitment.length, 34);
  assert.deepEqual(Array.from(commitment.slice(0, 2)), [1, 1]);
  const stateRoot = commitment.slice(2);
  const response = await backend.call("minijam_getManagedStateV1", {
    serviceId: deployment.serviceId,
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

function deployment(artifacts, serviceId, serviceKey, codeHash, abi) {
  return {
    artifacts,
    genesisHash,
    serviceKey,
    serviceId,
    codeHash,
    abiVersion: abi.abiVersion ?? abi.abi_version,
    abi,
  };
}

async function exerciseService(backend, service, seedValue, firstKey, secondKey, seed) {
  const client = new JamScriptClient(service.deployment, backend);
  const pair = sr25519PairFromSeed(hexToU8a("0x" + seed.repeat(32)));
  const signer = {
    publicKey: pair.publicKey,
    signRaw: async (message) => sr25519Sign(message, pair),
  };

  await client.validateDeployment();
  assert.equal(await client.readNonce(pair.publicKey), 0n);

  const seededAction = await client.submitAction(
    "seed",
    { key: firstKey, next: secondKey, value: seedValue },
    signer,
  );
  const seededResult = await client.waitForAction(
    seededAction.packageHash,
    seededAction.actionHash,
    { intervalMs: 500, timeoutMs: 120_000 },
  );
  assert.equal(seededResult.status, "imported");
  assert.equal(await client.readNonce(pair.publicKey), 1n);
  const valueKey = stateKey("test.values/v1", secondKey);
  const seeded = await managedStateValue(backend, service.deployment, valueKey);
  assert.ok(seeded);
  assert.deepEqual(Array.from(seeded.slice(0, 32)), Array.from(pair.publicKey));
  assert.equal(new DataView(seeded.buffer, seeded.byteOffset + 32, 4).getUint32(0, true), seedValue);

  const advanceAction = await client.submitAction("advance", { key: firstKey }, signer);
  const advanceResult = await client.waitForAction(
    advanceAction.packageHash,
    advanceAction.actionHash,
    { intervalMs: 500, timeoutMs: 120_000 },
  );
  assert.equal(advanceResult.status, "imported");
  assert.equal(await client.readNonce(pair.publicKey), 2n);
  const advanced = await managedStateValue(backend, service.deployment, valueKey);
  assert.ok(advanced);
  assert.deepEqual(Array.from(advanced.slice(0, 32)), Array.from(pair.publicKey));
  assert.equal(new DataView(advanced.buffer, advanced.byteOffset + 32, 4).getUint32(0, true), seedValue + 1);
  return { client, pair, valueKey };
}

async function main() {
  await cryptoWaitReady();
  const artifactDirectories = [artifactsA, ...(hasServiceB ? [artifactsB] : [])];
  const artifactMetadata = await Promise.all(
    artifactDirectories.map(async (directory) => ({
      metadata: await readJsonFrom(directory, "build.json"),
      abi: await readJsonFrom(directory, "service.abi.json"),
    })),
  );
  for (const { metadata, abi } of artifactMetadata) {
    assert.equal(metadata.language_version ?? metadata.languageVersion, "0.2");
    assert.equal(metadata.runtime_profile_version, "scriptc-deterministic-v1");
    assert.equal(metadata.runtimeRefineInputVersion, 1);
    assert.equal(abi.abiVersion ?? abi.abi_version, 1);
    assert.equal(abi.languageVersion ?? abi.language_version, "0.2");
  }

  const backend = new FetchRpcTransport(backendEndpoint);
  const serviceA = {
    deployment: deployment(artifactsA, serviceIdA, serviceKeyA, codeHashA, artifactMetadata[0].abi),
  };
  const serviceB = hasServiceB
    ? {
        deployment: deployment(artifactsB, serviceIdB, serviceKeyB, codeHashB, artifactMetadata[1].abi),
      }
    : null;
  const key1 = new Uint8Array(32).fill(0x11);
  const key2 = new Uint8Array(32).fill(0x22);
  const serviceAResult = exerciseService(
    backend,
    serviceA,
    10,
    key1,
    key2,
    "09",
  );
  const serviceBResult = serviceB
    ? exerciseService(
        backend,
        serviceB,
        20,
        new Uint8Array(32).fill(0x44),
        new Uint8Array(32).fill(0x55),
        "0a",
      )
    : null;
  const [aResult, bResult] = await Promise.all([serviceAResult, serviceBResult]);
  console.log("[dynamic] Service A and Service B submitted concurrently through one backend");
  if (bResult) {
    const aOnly = await managedStateValue(backend, serviceA.deployment, bResult.valueKey);
    const bOnly = await managedStateValue(backend, serviceB.deployment, aResult.valueKey);
    assert.equal(aOnly, null);
    assert.equal(bOnly, null);
    console.log("[dynamic] Service-scoped proofs kept A and B state isolated");
    console.log("DYNAMIC_SERVICE_A=PASS");
    console.log("DYNAMIC_SERVICE_B=PASS");
    console.log("MULTI_SERVICE_STATE_ISOLATION=PASS");
  }
  console.log("REAL_MINIJAM_E2E=PASS");
}

main().catch((error) => {
  console.error(error);
  process.exitCode = 1;
});
