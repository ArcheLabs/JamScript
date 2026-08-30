import { mkdir, rm, writeFile } from "node:fs/promises";
import { resolve } from "node:path";
import { spawnSync } from "node:child_process";

const root = import.meta.dirname;
const output = resolve(root, "out-service-conformance");
await rm(output, { recursive: true, force: true });
await mkdir(output, { recursive: true });
const key = { FixedBytes: { len: 32 } };
const value = { Record: { fields: [
  { name: "owner", ty: "Address" },
  { name: "value", ty: "U32" },
] } };
const spec = {
  source: resolve(root, "service-conformance.ts"),
  output,
  package_name: "service-conformance",
  states: [{ name: "entries", schema: "test.entries/v1", kind: "Map", key_type: key, value_type: value }],
  actions: [
    { name: "create", auth: "Wallet", input: [{ name: "key", ty: key }, { name: "value", ty: "U32" }] },
    { name: "update", auth: "Wallet", input: [{ name: "key", ty: key }, { name: "value", ty: "U32" }] },
  ],
  queries: [],
};
const specPath = resolve(output, "service.json");
await writeFile(specPath, JSON.stringify(spec));
const run = spawnSync(process.execPath, [resolve(root, "compile-service.mjs"), specPath], { encoding: "utf8" });
if (run.status !== 0) throw new Error(run.stderr || run.stdout);
console.log("ScriptC M2 whole-service compilation: PASS");
