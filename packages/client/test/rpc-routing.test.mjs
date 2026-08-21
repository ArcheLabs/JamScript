import assert from "node:assert/strict";
import test from "node:test";
import { SplitRpcTransport } from "../dist/index.js";

test("SplitRpcTransport sends node queries to node and Work methods to formal RPC", async () => {
  const nodeCalls = [];
  const workCalls = [];
  const node = {
    async call(method, params) {
      nodeCalls.push({ method, params });
      return "node:" + method;
    },
  };
  const work = {
    async call(method, params) {
      workCalls.push({ method, params });
      return "work:" + method;
    },
  };
  const transport = new SplitRpcTransport(node, work);

  assert.equal(await transport.call("chain_getBlockHash", [0]), "node:chain_getBlockHash");
  assert.equal(
    await transport.call("minijam_getFinalizedContext"),
    "node:minijam_getFinalizedContext",
  );
  assert.equal(
    await transport.call("minijam_getServiceStorageAt", ["0x01", 1000, "0x02"]),
    "node:minijam_getServiceStorageAt",
  );
  assert.equal(await transport.call("minijam_submitWorkV1", {}), "work:minijam_submitWorkV1");
  assert.equal(
    await transport.call("minijam_getWorkStatusV1", { packageHash: "0x03" }),
    "work:minijam_getWorkStatusV1",
  );

  assert.deepEqual(
    nodeCalls.map(({ method }) => method),
    [
      "chain_getBlockHash",
      "minijam_getFinalizedContext",
      "minijam_getServiceStorageAt",
    ],
  );
  assert.deepEqual(
    workCalls.map(({ method }) => method),
    ["minijam_submitWorkV1", "minijam_getWorkStatusV1"],
  );
});
