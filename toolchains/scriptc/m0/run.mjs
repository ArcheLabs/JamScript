import { createHash } from "node:crypto";
import { mkdir, readFile, writeFile } from "node:fs/promises";
import { dirname, resolve } from "node:path";
import { compileLibrary } from "@scriptc/compiler";

const root = resolve(import.meta.dirname);
const expectedManifest = JSON.parse(await readFile(resolve(root, "..", "SURFACE_MANIFEST.json"), "utf8"));
const manifestPath = resolve(root, "..", "node_modules/@scriptc/compiler/surface-manifest.json");
const manifestBytes = await readFile(manifestPath);
const manifestHash = createHash("sha256").update(manifestBytes).digest("hex");
if (manifestHash !== expectedManifest.sha256) {
  throw new Error(`ScriptC surface manifest hash mismatch: expected ${expectedManifest.sha256}, got ${manifestHash}`);
}
const samples = [
  ["scalar", "scalar.ts", "add", ["f64", "f64"], "f64"],
  ["control-flow", "control-flow.ts", "sum", ["f64"], "f64"],
  ["arrays", "arrays.ts", "sum", [], "f64"],
  ["strings", "strings.ts", "normalize", ["string"], "string"],
  ["bytes", "bytes.ts", "firstByte", ["bytes"], "f64"],
];

const output = resolve(process.env.SCRIPTC_M0_OUT ?? resolve(root, "out"));
await mkdir(output, { recursive: true });
const results = [];

for (const [name, file, exported, params, returns] of samples) {
  const profilePath = resolve(output, `${name}.profile.json`);
  const outDir = resolve(output, name);
  await mkdir(outDir, { recursive: true });
  await writeFile(profilePath, JSON.stringify({
    profile_format: 1,
    name: `jamscript-m0-${name}`,
    entry: `../samples/${file}`,
    emission: "c",
    optimization: "dev",
    abi: {
      prefix: `jamscript_m0_${name.replaceAll("-", "_")}_`,
      init_symbol: `jamscript_m0_${name.replaceAll("-", "_")}_init`,
      sink_register_symbol: `jamscript_m0_${name.replaceAll("-", "_")}_set_panic_sink`,
      collect_symbol: null,
      result_reset_symbol: null,
    },
    exports: [{ export: exported, symbol: `jamscript_m0_${name.replaceAll("-", "_")}_entry`, params, returns }],
    determinism: { fences: [] },
  }, null, 2));

  let result;
  try {
    result = await compileLibrary({
      profilePath,
      outDir,
      outPath: resolve(outDir, `${name}.lib.a`),
      emitIr: true,
    });
  } catch (error) {
    results.push({ name, status: "fail", error: String(error) });
    continue;
  }
  if (result.ok) {
    results.push({ name, status: "pass", archive: result.archivePath, c: result.cPath, backend: result.backend });
  } else {
    results.push({ name, status: "fail", diagnostics: result.diagnostics });
  }
}

await writeFile(resolve(output, "results.json"), JSON.stringify(results, null, 2));
console.log(JSON.stringify(results, null, 2));
if (results.some((result) => result.status === "fail")) process.exitCode = 1;
