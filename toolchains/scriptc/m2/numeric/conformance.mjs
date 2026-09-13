import { copyFile, mkdir, rm } from "node:fs/promises";
import { resolve } from "node:path";
import { spawnSync } from "node:child_process";
import { compileLibrary } from "@scriptc/compiler";

const root = import.meta.dirname;
const output = resolve(root, "out-conformance");
await rm(output, { recursive: true, force: true });
await mkdir(resolve(output, "numeric"), { recursive: true });
await copyFile(resolve(root, "../runtime.ts"), resolve(output, "runtime.ts"));
await copyFile(resolve(root, "runtime.ts"), resolve(output, "numeric/scriptc_runtime.ts"));
const result = await compileLibrary({
  profilePath: resolve(root, "conformance.profile.json"),
  outDir: output,
  outPath: resolve(output, "numeric-conformance.a"),
  emitIr: true,
});
if (!result.ok) throw new Error(JSON.stringify(result.diagnostics, null, 2));
const binary = resolve(output, "numeric-conformance");
const cc = process.env.CC || "clang";
const compile = spawnSync(cc, [
  "-no-pie",
  resolve(root, "conformance.c"),
  result.archivePath,
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
console.log("SCRIPTC_FIXED_LIMB_NUMERIC=PASS");
