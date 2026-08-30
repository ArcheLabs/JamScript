import { mkdir, rm } from "node:fs/promises";
import { spawnSync } from "node:child_process";
import { resolve } from "node:path";
import { compileLibrary } from "@scriptc/compiler";

const root = import.meta.dirname;
const output = resolve(root, "out-conformance");
await rm(output, { recursive: true, force: true });
await mkdir(output, { recursive: true });
const result = await compileLibrary({
  profilePath: resolve(root, "conformance.profile.json"),
  outDir: output,
  outPath: resolve(output, "conformance.a"),
  emitIr: true,
});
if (!result.ok) throw new Error(JSON.stringify(result.diagnostics, null, 2));
const binary = resolve(output, "conformance");
const compile = spawnSync("clang", [
  resolve(root, "conformance.c"),
  resolve(output, "conformance.a"),
  "-lm",
  "-pthread",
  "-l:libbsd.so.0",
  "-o",
  binary,
], { encoding: "utf8" });
if (compile.status !== 0) throw new Error(compile.stderr || compile.stdout);
const run = spawnSync(binary, [], { encoding: "utf8" });
if (run.status !== 0) throw new Error(run.stderr || run.stdout || `exit ${run.status}`);
process.stdout.write(run.stdout);
console.log("ScriptC M2 local-state runtime: PASS");
