import assert from "node:assert/strict";
import test from "node:test";
import {
  JamScriptClient,
  RpcError,
  actionSelector,
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

function contextResult(context) {
  return { packageHash: "0x" + "77".repeat(32), submissionHash: "0x" + "88".repeat(32), context };
}

test("submitAction reads finalized nonce and signs exactly once across stale retry", async () => {
  const calls = [];
  let contextReads = 0;
  let submissions = 0;
  const transport = {
    async call(method, params = []) {
      calls.push({ method, params });
      if (method === "chain_getBlockHash") return genesisHash;
      if (method === "minijam_getFinalizedContext") {
        contextReads += 1;
        return contextReads === 1 ? initialContext : refreshedContext;
      }
      if (method === "minijam_getServiceStorageAt") return null;
      if (method === "minijam_getManagedStateV1") {
        return { serviceId: 1000, stateRoot: emptyManagedStateRoot, keyBase64: params.keyBase64, valueBase64: null, proofBase64: ["AA=="] };
      }
      if (method === "minijam_submitWorkV1") {
        submissions += 1;
        if (submissions === 1) throw new RpcError("stale", -32010);
        return contextResult(refreshedContext);
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

  assert.equal(result.context.blockHash, refreshedContext.blockHash);
  assert.equal(signatures, 1);
  assert.equal(submissions, 2);
  const nonceRead = calls.find((call) => call.method === "minijam_getServiceStorageAt");
  assert.equal(nonceRead.params[0], initialContext.blockHash);
});

test("query reads and decodes state at the finalized block", async () => {
  const transport = {
    async call(method, params = []) {
      if (method === "minijam_getFinalizedContext") return initialContext;
      if (method === "minijam_getServiceStorageAt") {
        assert.equal(params[0], initialContext.blockHash);
        return managedCommitment(queryManagedStateRoot);
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
      if (method === "minijam_getManagedStateV1") throw new RpcError("unavailable root", -32030);
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
      if (method !== "minijam_getWorkStatusV1") throw new Error("unexpected RPC method");
      return {
        packageHash: "0x" + "77".repeat(32),
        workId: 3,
        status: "imported",
        executionReceipt: "0x" + "99".repeat(32),
        actionReceipts: [{ actionHash: "0x" + "aa".repeat(32), status: "failed", errorCode: 2 }],
        context: initialContext,
      };
    },
  };
  const client = new JamScriptClient(deployment, transport);
  const result = await client.waitForAction(
    "0x" + "77".repeat(32),
    "0x" + "aa".repeat(32),
    { intervalMs: 0, timeoutMs: 1000 },
  );
  assert.equal(result.status, "imported");
  assert.equal(result.actionReceipt.status, "failed");
  assert.equal(result.actionReceipt.errorCode, 2);
});
