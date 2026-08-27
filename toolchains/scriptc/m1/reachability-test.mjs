import { mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { resolve } from "node:path";
import { spawnSync } from "node:child_process";

const root = resolve(import.meta.dirname);
const compiler = resolve(root, "compile-action.mjs");

async function run(name, body) {
  const dir = await mkdtemp(resolve(tmpdir(), `jamscript-scriptc-${name}-`));
  try {
    const source = resolve(dir, "service.ts");
    const output = resolve(dir, "out");
    await writeFile(source, `import { action, wallet, u64 } from "jam";\n${body}\n`);
    const spec = resolve(dir, "spec.json");
    await writeFile(spec, JSON.stringify({
      source,
      action: "run",
      input_fields: [{ name: "value", type: "u64" }],
      output,
    }));
    return spawnSync(process.execPath, [compiler, spec], { encoding: "utf8" });
  } finally {
    await rm(dir, { recursive: true, force: true });
  }
}

const unreachable = await run("unreachable", `
function unusedClock() { return Date.now(); }
export const run = action({ auth: wallet(), input: { value: u64 }, execute(ctx, input) {
  return input.value + 1;
} });`);
if (unreachable.status !== 0) {
  throw new Error(`unreachable forbidden surface was rejected: ${unreachable.stderr}`);
}

const reachable = await run("reachable", `
export const run = action({ auth: wallet(), input: { value: u64 }, execute(ctx, input) {
  return Date.now();
} });`);
if (reachable.status === 0 || !`${reachable.stdout}${reachable.stderr}`.includes("JAM1117")) {
  throw new Error("reachable Date.now was not rejected by the deterministic policy");
}

console.log("ScriptC reachability policy: PASS");
