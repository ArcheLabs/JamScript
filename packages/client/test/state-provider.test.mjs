import assert from "node:assert/strict";
import test from "node:test";
import {
  FallbackStateProvider,
  JamScriptClient,
  StateProviderError,
  actionSelector,
  stateKey,
  toHex,
} from "../dist/index.js";

const root = "0x802e9d7821c66ad0f0747bf287bf46410819ca9f2aa787411522f26577e766bd";
const proof = [hex("7f1901090073636f7265732f76310101010101010101010101010101010101010101010101010101010101010101202a00000000000000")];
const key = new Uint8Array(32).fill(1);
const value = u64(42);
const deployment = {
  genesisHash: "0x" + "11".repeat(32),
  serviceKey: "0x" + "aa".repeat(32),
  serviceId: 1000,
  codeHash: "0x" + "22".repeat(32),
  abiVersion: 1,
  abi: {
    abiVersion: 1,
    languageVersion: "0.2",
    package: { name: "provider-test", version: "0.1.0" },
    actions: [{ name: "submit", selector: toHex(actionSelector("submit")), auth: "wallet", input: [], executeOutput: "unit" }],
    queries: [{ name: "getScore", kind: "state_get", state: "scores", keyType: "address", output: { type: "u64", nullable: true } }],
    types: { u64: { kind: "u64", max: null } },
    state: [{ name: "scores", schema: "scores/v1", kind: "map", keyType: "address", valueType: "u64" }],
  },
};

function u64(number) {
  const output = new Uint8Array(8);
  new DataView(output.buffer).setBigUint64(0, BigInt(number), true);
  return output;
}

function hex(value) {
  return Uint8Array.from(value.match(/.{2}/g).map((byte) => Number.parseInt(byte, 16)));
}

function transport() {
  return {
    async call(method) {
      if (method === "minijam_getFinalizedContext") return {
        blockHash: "0x" + "33".repeat(32),
        blockNumber: 1,
        stateRoot: "0x" + "44".repeat(32),
        slot: 1,
      };
      if (method === "minijam_getServiceStorageAt") return "0x880101" + root.slice(2);
      throw new Error("unexpected transport method: " + method);
    },
  };
}

function response(responseValue = value) {
  return {
    serviceId: 1000,
    stateRoot: root,
    key: stateKey("scores/v1", key),
    value: responseValue,
    proof,
  };
}

test("JamScriptClient accepts an independently injected StateProvider", async () => {
  let requests = 0;
  const provider = {
    async get(request) {
      requests += 1;
      assert.equal(request.serviceId, 1000);
      assert.equal(request.serviceKey, deployment.serviceKey);
      assert.equal(request.stateRoot, root);
      assert.deepEqual(request.key, stateKey("scores/v1", key));
      return response();
    },
  };
  const result = await new JamScriptClient(deployment, transport(), { stateProvider: provider })
    .queryLatest("getScore", key);
  assert.equal(result.value, 42n);
  assert.equal(requests, 1);
});

test("FallbackStateProvider skips an invalid proof and uses the next provider", async () => {
  const invalid = {
    async get() {
      return response(Uint8Array.of(41, 0, 0, 0, 0, 0, 0, 0));
    },
  };
  const valid = {
    async get() {
      return response();
    },
  };
  const result = await new JamScriptClient(
    deployment,
    transport(),
    { stateProvider: new FallbackStateProvider([invalid, valid]) },
  ).queryLatest("getScore", key);
  assert.equal(result.value, 42n);
});

test("a malicious provider cannot bypass client proof verification", async () => {
  const malicious = {
    async get() {
      return response(Uint8Array.of(41, 0, 0, 0, 0, 0, 0, 0));
    },
  };
  await assert.rejects(
    new JamScriptClient(deployment, transport(), { stateProvider: malicious }).queryLatest("getScore", key),
    /does not match/,
  );
  assert.ok(new StateProviderError("InvalidProof", "test") instanceof Error);
});
