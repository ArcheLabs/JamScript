import assert from "node:assert/strict";
import test from "node:test";
import {
  JamScriptClient,
  RpcError,
  TransactionWaitTimeoutError,
  actionSelector,
  decodeSignedActionV1,
  toHex,
} from "../dist/index.js";

const genesisHash = "0x" + "11".repeat(32);
const codeHash = "0x" + "22".repeat(32);
const serviceKey = "0x" + "aa".repeat(32);
const emptyManagedStateRoot = "0x03170a2e7597b7b7e3d84c05391d139a62b157e78786d8c082f29dcf4c111314";
const queryManagedStateRoot = "0x802e9d7821c66ad0f0747bf287bf46410819ca9f2aa787411522f26577e766bd";
const queryProofBase64 = Buffer.from("7f1901090073636f7265732f76310101010101010101010101010101010101010101010101010101010101010101202a00000000000000", "hex").toString("base64");
const initialContext = {
  blockHash: "0x" + "33".repeat(32),
  blockNumber: 10,
  stateRoot: "0x" + "44".repeat(32),
  slot: 10,
};
const refreshedContext = {
  blockHash: "0x" + "55".repeat(32),
  blockNumber: 11,
  stateRoot: "0x" + "66".repeat(32),
  slot: 11,
};
const deployment = {
  genesisHash,
  networkDomain: genesisHash,
  serviceKey,
  serviceId: 1000,
  codeHash,
  abiVersion: 1,
  abi: {
    abiVersion: 1,
    languageVersion: "0.2",
    package: { name: "game", version: "0.1.0" },
    actions: [
      {
        name: "submit",
        selector: toHex(actionSelector("submit")),
        auth: "wallet",
        input: [{ name: "score", type: "u64" }],
        executeOutput: "u64",
      },
    ],
    queries: [
      {
        name: "getScore",
        kind: "state_get",
        state: "scores",
        keyType: "address",
        output: { type: "u64", nullable: true },
      },
    ],
    types: { u64: { kind: "u64", max: null } },
    state: [
      {
        name: "scores",
        schema: "scores/v1",
        kind: "map",
        keyType: "address",
        valueType: "u64",
      },
    ],
  },
};

function stateU64(value) {
  const bytes = new Uint8Array(9);
  bytes[0] = 0x20;
  new DataView(bytes.buffer).setBigUint64(1, BigInt(value), true);
  return "0x" + Array.from(bytes, (byte) => byte.toString(16).padStart(2, "0")).join("");
}

function rawU64(value) {
  const bytes = new Uint8Array(8);
  new DataView(bytes.buffer).setBigUint64(0, BigInt(value), true);
  return Buffer.from(bytes).toString("base64");
}

function managedCommitment(root) {
  return "0x88" + "0101" + root.slice(2);
}

function transactionResult(transactionId = "0x" + "99".repeat(32)) {
  return { transactionId, status: "queued", packageHash: null, itemIndex: null, actionIndex: 0 };
}

test("submitAction signs once and submits a logical transaction", async () => {
  const calls = [];
  let contextReads = 0;
  let submissions = 0;
  const transport = {
    async call(method, params = []) {
      calls.push({ method, params });
      if (method === "chain_getBlockHash") return genesisHash;
      if (method === "minijam_getFinalizedContext") {
        contextReads += 1;
        return initialContext;
      }
      if (method === "minijam_getServiceStorageAt") return null;
      if (method === "jamscript_getStateV1") {
        return { serviceId: 1000, stateRoot: emptyManagedStateRoot, keyBase64: params.keyBase64, valueBase64: null };
      }
      if (method === "minijam_getManagedStateV1") {
        return { serviceId: 1000, stateRoot: emptyManagedStateRoot, keyBase64: params.keyBase64, valueBase64: null, proofBase64: ["AA=="] };
      }
      if (method === "jamscript_submitTransactionV1") {
        submissions += 1;
        return transactionResult();
      }
      throw new Error("unexpected RPC method: " + method);
    },
  };
  let signatures = 0;
  const signer = {
    publicKey: new Uint8Array(32).fill(9),
    async signRaw(message) {
      signatures += 1;
      assert.equal(message.length, 32);
      return new Uint8Array(64).fill(10);
    },
  };

  const client = new JamScriptClient(deployment, transport);
  const result = await client.submitAction("submit", { score: 9n }, signer);

  assert.match(result.transactionId, /^0x/);
  assert.equal(signatures, 1);
  assert.equal(submissions, 1);
  assert.equal(contextReads, 1);
  const nonceRead = calls.find((call) => call.method === "minijam_getServiceStorageAt");
  assert.equal(nonceRead.params[0], initialContext.blockHash);
});

test("Ownership action preparation, wallet signature, and submission are separate phases", async () => {
  const calls = [];
  const ownershipDeployment = {
    ...deployment,
    abi: {
      ...deployment.abi,
      actions: [...deployment.abi.actions, {
        name: "createAsset",
        selector: toHex(actionSelector("createAsset")),
        auth: { kind: "ownership", version: 1 },
        input: [{ name: "subject", type: "ownership" }, { name: "initialSupply", type: "u64" }],
        executeOutput: "unit",
      }],
    },
  };
  const transport = {
    async call(method, params = []) {
      calls.push({ method, params });
      if (method === "chain_getBlockHash") return genesisHash;
      if (method === "minijam_getFinalizedContext") return initialContext;
      if (method === "minijam_getServiceStorageAt") return null;
      if (method === "jamscript_getStateV1") {
        return { serviceId: 1000, stateRoot: emptyManagedStateRoot, keyBase64: params.keyBase64, valueBase64: null };
      }
      if (method === "jamscript_submitTransactionV1") return transactionResult("0xprepared");
      throw new Error("unexpected RPC method: " + method);
    },
  };
  const signer = {
    publicKey: new Uint8Array(32).fill(7),
    signatures: 0,
    async getController() {
      return { version: 1, kind: 0, public: new Uint8Array(32).fill(7) };
    },
    signJamScriptAction(request) {
      this.signatures += 1;
      assert.equal(request.message.length > 0, true);
      return Promise.resolve(new Uint8Array([1, 2, 3]));
    },
  };
  const client = new JamScriptClient(ownershipDeployment, transport);
  const preparationPhases = [];
  const prepared = await client.prepareOwnershipAction("createAsset", {
    subject: { version: 1, kind: 0, public: new Uint8Array(32).fill(8) },
    initialSupply: 12n,
  }, signer, { onProgress: (phase) => preparationPhases.push(phase) });

  assert.equal(prepared.phase, "prepared");
  assert.deepEqual(preparationPhases, ["VALIDATING_DEPLOYMENT", "READING_FINALIZED_CONTEXT", "READING_MANAGED_STATE", "READING_NONCE"]);
  assert.equal(signer.signatures, 0);
  const callsBeforeSignature = calls.length;
  const signing = client.signPreparedOwnershipAction(prepared);
  assert.equal(signer.signatures, 1, "wallet signing starts synchronously in the signature phase");
  assert.equal(calls.length, callsBeforeSignature, "signature phase performs no RPC work");
  const signed = await signing;
  assert.equal(signed.phase, "signed");
  assert.match(signed.actionHash, /^0x[0-9a-f]{64}$/i);

  const submitting = client.submitSignedOwnershipAction(signed);
  assert.equal(calls.at(-1).method, "jamscript_submitTransactionV1");
  const submitted = await submitting;
  assert.equal(submitted.transactionId, "0xprepared");
  assert.equal(submitted.actionHash, signed.actionHash);
});

test("a rejected Ownership wallet signature releases its prepared controller reservation", async () => {
  const ownershipDeployment = {
    ...deployment,
    abi: {
      ...deployment.abi,
      actions: [...deployment.abi.actions, {
        name: "createAsset",
        selector: toHex(actionSelector("createAsset")),
        auth: { kind: "ownership", version: 1 },
        input: [{ name: "subject", type: "ownership" }],
        executeOutput: "unit",
      }],
    },
  };
  const transport = {
    async call(method, params = []) {
      if (method === "chain_getBlockHash") return genesisHash;
      if (method === "minijam_getFinalizedContext") return initialContext;
      if (method === "minijam_getServiceStorageAt") return null;
      if (method === "jamscript_getStateV1") {
        return { serviceId: 1000, stateRoot: emptyManagedStateRoot, keyBase64: params.keyBase64, valueBase64: null };
      }
      throw new Error("unexpected RPC method: " + method);
    },
  };
  let rejectSignature = true;
  const signer = {
    async getController() { return { version: 1, kind: 0, public: new Uint8Array(32).fill(7) }; },
    signJamScriptAction() {
      if (rejectSignature) return Promise.reject(new Error("user rejected signature"));
      return Promise.resolve(new Uint8Array([1]));
    },
  };
  const client = new JamScriptClient(ownershipDeployment, transport);
  const input = { subject: { version: 1, kind: 0, public: new Uint8Array(32).fill(8) } };
  const first = await client.prepareOwnershipAction("createAsset", input, signer);
  await assert.rejects(client.signPreparedOwnershipAction(first), /user rejected signature/);
  rejectSignature = false;
  const second = await client.prepareOwnershipAction("createAsset", input, signer);
  assert.equal((await client.signPreparedOwnershipAction(second)).phase, "signed");
});

test("abandoning a wallet signature timeout prevents a late signature from producing a submittable action", async () => {
  const ownershipDeployment = {
    ...deployment,
    abi: {
      ...deployment.abi,
      actions: [...deployment.abi.actions, {
        name: "createAsset",
        selector: toHex(actionSelector("createAsset")),
        auth: { kind: "ownership", version: 1 },
        input: [{ name: "subject", type: "ownership" }],
        executeOutput: "unit",
      }],
    },
  };
  const transport = {
    async call(method, params = []) {
      if (method === "chain_getBlockHash") return genesisHash;
      if (method === "minijam_getFinalizedContext") return initialContext;
      if (method === "minijam_getServiceStorageAt") return null;
      if (method === "jamscript_getStateV1") {
        return { serviceId: 1000, stateRoot: emptyManagedStateRoot, keyBase64: params.keyBase64, valueBase64: null };
      }
      throw new Error("unexpected RPC method: " + method);
    },
  };
  let resolveSignature;
  let signatureCalls = 0;
  const signer = {
    async getController() { return { version: 1, kind: 0, public: new Uint8Array(32).fill(7) }; },
    signJamScriptAction() {
      signatureCalls += 1;
      if (signatureCalls > 1) return Promise.resolve(new Uint8Array([1]));
      return new Promise((resolve) => { resolveSignature = resolve; });
    },
  };
  const client = new JamScriptClient(ownershipDeployment, transport);
  const input = { subject: { version: 1, kind: 0, public: new Uint8Array(32).fill(8) } };
  const prepared = await client.prepareOwnershipAction("createAsset", input, signer);
  const signing = client.signPreparedOwnershipAction(prepared);
  client.abandonPreparedOwnershipAction(prepared);
  resolveSignature(new Uint8Array([1]));
  await assert.rejects(signing, /abandoned while the wallet request was open/);

  const replacement = await client.prepareOwnershipAction("createAsset", input, signer);
  assert.equal((await client.signPreparedOwnershipAction(replacement)).phase, "signed");
});

function nonceAwareTransport({ failFirstSubmission = false, submissionDelayMs = 0 } = {}) {
  const submissions = [];
  let submissionCount = 0;
  let activeSubmissions = 0;
  let maxActiveSubmissions = 0;
  return {
    submissions,
    get maxActiveSubmissions() { return maxActiveSubmissions; },
    async call(method, params = []) {
      if (method === "chain_getBlockHash") return genesisHash;
      if (method === "minijam_getFinalizedContext") return initialContext;
      if (method === "minijam_getServiceStorageAt") return null;
      if (method === "jamscript_getStateV1") {
        return { serviceId: 1000, stateRoot: emptyManagedStateRoot, keyBase64: params.keyBase64, valueBase64: null };
      }
      if (method === "jamscript_submitTransactionV1") {
        submissions.push(params);
        submissionCount += 1;
        if (failFirstSubmission && submissionCount === 1) throw new RpcError("admission failed", -32001);
        activeSubmissions += 1;
        maxActiveSubmissions = Math.max(maxActiveSubmissions, activeSubmissions);
        if (submissionDelayMs) await new Promise((resolve) => setTimeout(resolve, submissionDelayMs));
        activeSubmissions -= 1;
        return transactionResult("0x" + submissionCount.toString(16).padStart(2, "0").repeat(32));
      }
      throw new Error("unexpected RPC method: " + method);
    },
  };
}

test("client reserves per-signer nonces while all signers submit in parallel", async () => {
  const transport = nonceAwareTransport({ submissionDelayMs: 25 });
  const client = new JamScriptClient(deployment, transport);
  const alice = {
    publicKey: new Uint8Array(32).fill(1),
    async signRaw() { return new Uint8Array(64).fill(1); },
  };
  const bob = {
    publicKey: new Uint8Array(32).fill(2),
    async signRaw() { return new Uint8Array(64).fill(2); },
  };
  await Promise.all([
    client.submitAction("submit", { score: 1n }, alice),
    client.submitAction("submit", { score: 3n }, alice),
    client.submitAction("submit", { score: 2n }, bob),
  ]);
  assert.equal(transport.submissions.length, 3);
  assert.equal(transport.maxActiveSubmissions, 3);
  const nonces = transport.submissions.map((request) =>
    decodeSignedActionV1(Uint8Array.from(Buffer.from(request.payloadBase64, "base64"))));
  assert.deepEqual(
    nonces
      .filter((action) => action.publicKey[0] === 1)
      .map((action) => action.nonce)
      .sort((left, right) => (left < right ? -1 : left > right ? 1 : 0)),
    [0n, 1n],
  );
  console.log("CLIENT_PER_SIGNER_NONCE_RESERVATION=PASS");
  console.log("CLIENT_CROSS_SIGNER_PARALLEL=PASS");
});

test("client resynchronizes a signer nonce after admission failure", async () => {
  const transport = nonceAwareTransport({ failFirstSubmission: true });
  const client = new JamScriptClient(deployment, transport);
  const signer = {
    publicKey: new Uint8Array(32).fill(3),
    async signRaw() { return new Uint8Array(64).fill(3); },
  };
  await assert.rejects(client.submitAction("submit", { score: 3n }, signer), /admission failed/);
  await client.submitAction("submit", { score: 4n }, signer);
  const second = decodeSignedActionV1(Uint8Array.from(Buffer.from(transport.submissions[1].payloadBase64, "base64")));
  assert.equal(second.nonce, 0n);
  console.log("CLIENT_ADMISSION_FAILURE_RESYNC=PASS");
});

test("query reads and decodes state at the finalized block", async () => {
  const transport = {
    async call(method, params = []) {
      if (method === "minijam_getFinalizedContext") return initialContext;
      if (method === "minijam_getServiceStorageAt") {
        assert.equal(params[0], initialContext.blockHash);
        return managedCommitment(queryManagedStateRoot);
      }
      if (method === "jamscript_getStateV1") {
        return { serviceId: 1000, stateRoot: queryManagedStateRoot, keyBase64: params.keyBase64, valueBase64: rawU64(42) };
      }
      if (method === "minijam_getManagedStateV1") {
        return { serviceId: 1000, stateRoot: queryManagedStateRoot, keyBase64: params.keyBase64, valueBase64: rawU64(42), proofBase64: [queryProofBase64] };
      }
      throw new Error("unexpected RPC method: " + method);
    },
  };
  const client = new JamScriptClient(deployment, transport);
  const result = await client.queryLatest("getScore", new Uint8Array(32).fill(1));
  assert.equal(result.value, 42n);
  assert.equal(result.context.blockHash, initialContext.blockHash);
  assert.equal(result.stateRoot, queryManagedStateRoot);
});

test("managed-state provider unavailability does not fall back to Service KV by default", async () => {
  let storageReads = 0;
  const transport = {
    async call(method) {
      if (method === "minijam_getFinalizedContext") return initialContext;
      if (method === "minijam_getServiceStorageAt") {
        storageReads += 1;
        return storageReads === 1 ? null : stateU64(99);
      }
      if (method === "jamscript_getStateV1") throw new RpcError("unavailable root", -32031);
      throw new Error("unexpected RPC method: " + method);
    },
  };
  const client = new JamScriptClient(deployment, transport);
  await assert.rejects(client.queryLatest("getScore", new Uint8Array(32).fill(1)), /unavailable root/);
  assert.equal(storageReads, 1);
});

test("waitForWork tolerates not-finalized package lookup and stops at Imported", async () => {
  let reads = 0;
  const transport = {
    async call(method) {
      if (method !== "minijam_getWorkStatusV1") throw new Error("unexpected RPC method");
      reads += 1;
      if (reads === 1) throw new RpcError("not finalized", -32013);
      if (reads === 2) {
        return { packageHash: "0x" + "77".repeat(32), workId: 3, status: "accepted", executionReceipt: null, context: initialContext };
      }
      return { packageHash: "0x" + "77".repeat(32), workId: 3, status: "imported", executionReceipt: "0x" + "99".repeat(32), context: initialContext };
    },
  };
  const client = new JamScriptClient(deployment, transport);
  const result = await client.waitForWork("0x" + "77".repeat(32), { intervalMs: 0, timeoutMs: 1000 });
  assert.equal(result.status, "imported");
  assert.equal(reads, 3);
});

test("waitForAction distinguishes an imported failed application receipt", async () => {
  const transport = {
    async call(method) {
      if (method !== "jamscript_getTransactionStatusV1") throw new Error("unexpected RPC method");
      return {
        transactionId: "0x" + "77".repeat(32),
        status: "imported",
        executionReceipt: "0x" + "99".repeat(32),
        itemIndex: 0,
        actionIndex: 0,
        error: null,
        actionReceipts: [{ actionHash: "0x" + "aa".repeat(32), status: "failed", errorCode: 2 }],
      };
    },
  };
  const client = new JamScriptClient(deployment, transport);
  const result = await client.waitForAction(
    "0x" + "77".repeat(32),
    "0x" + "aa".repeat(32),
    { intervalMs: 0, timeoutMs: 1000 },
  );
  assert.equal(result.status, "failed");
  assert.equal(result.transactionStatus, "imported");
  assert.equal(result.actionReceipt.status, "failed");
  assert.equal(result.actionReceipt.errorCode, 2);
});

test("waitForTransaction timeout carries the last queued status without calling it failed", async () => {
  let reads = 0;
  const transport = {
    async call(method) {
      if (method !== "jamscript_getTransactionStatusV1") throw new Error("unexpected RPC method");
      reads += 1;
      return {
        transactionId: "0x" + "77".repeat(32),
        status: "queued",
        packageHash: null,
        itemIndex: null,
        actionIndex: null,
        executionReceipt: null,
        error: null,
      };
    },
  };
  const client = new JamScriptClient(deployment, transport);
  await assert.rejects(
    client.waitForTransaction("0x" + "77".repeat(32), { intervalMs: 0, timeoutMs: 0 }),
    (error) => {
      assert.ok(error instanceof TransactionWaitTimeoutError);
      assert.equal(error.lastStatus?.status, "queued");
      assert.equal(error.transactionId, "0x" + "77".repeat(32));
      return true;
    },
  );
  assert.equal(reads, 1);
});
