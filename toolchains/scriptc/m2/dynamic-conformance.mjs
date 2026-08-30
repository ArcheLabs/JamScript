import { mkdir, readFile, writeFile } from "node:fs/promises";
import { resolve } from "node:path";
import { spawn } from "node:child_process";

const root = resolve(import.meta.dirname, "../../../");
const output = resolve(import.meta.dirname, "out-dynamic-conformance");
const source = resolve(root, "examples/dynamic-state-scriptc/src/service.ts");
await mkdir(output, { recursive: true });
const spec = {
  source,
  output,
  package_name: "dynamic-state-scriptc",
  states: [
    {
      name: "index",
      schema: "test.index/v1",
      kind: "Map",
      key_type: { Bytes: { max: 32 } },
      value_type: { Record: { fields: [{ name: "next", ty: { Bytes: { max: 32 } } }] } },
    },
    {
      name: "values",
      schema: "test.values/v1",
      kind: "Map",
      key_type: { Bytes: { max: 32 } },
      value_type: { Record: { fields: [
        { name: "owner", ty: "Address" },
        { name: "value", ty: "U32" },
      ] } },
    },
  ],
  actions: [{
    name: "advance",
    auth: "Wallet",
    input: [{ name: "key", ty: { Bytes: { max: 32 } } }],
  }],
  queries: [],
};
const specPath = resolve(output, "dynamic-service.json");
await writeFile(specPath, JSON.stringify(spec));
await run(process.execPath, [resolve(import.meta.dirname, "compile-service.mjs"), specPath]);
const generated = await readFile(resolve(output, "scriptc_service.transformed.ts"), "utf8");
if (!generated.includes("stateGetRaw") || !generated.includes("stateSetRaw") || !generated.includes("__jamscript_action_advance_v2")) {
  throw new Error("dynamic ScriptC conformance artifact is incomplete");
}
console.log("ScriptC M2 dynamic-state compilation: PASS");

function run(command, args) {
  return new Promise((resolvePromise, reject) => {
    const child = spawn(command, args, { stdio: "inherit" });
    child.on("error", reject);
    child.on("exit", (code) => code === 0 ? resolvePromise() : reject(new Error(`${command} exited with ${code}`)));
  });
}
