import { createHash } from "node:crypto";
import { mkdir, readFile, writeFile } from "node:fs/promises";
import { resolve } from "node:path";
import { compileLibrary } from "@scriptc/compiler";

const root = resolve(import.meta.dirname);
const output = resolve(process.env.SCRIPTC_M1_OUT ?? resolve(root, "out"));
const source = resolve(root, "conformance.ts");
const expectedManifest = JSON.parse(await readFile(resolve(root, "..", "SURFACE_MANIFEST.json"), "utf8"));
const manifest = await readFile(resolve(root, "..", "node_modules/@scriptc/compiler/surface-manifest.json"));
const actualManifestHash = createHash("sha256").update(manifest).digest("hex");
if (actualManifestHash !== expectedManifest.sha256) {
  throw new Error(`ScriptC surface manifest hash mismatch: expected ${expectedManifest.sha256}, got ${actualManifestHash}`);
}
await mkdir(output, { recursive: true });
const profilePath = resolve(output, "conformance.profile.json");
await writeFile(profilePath, JSON.stringify({
  profile_format: 1,
  name: "jamscript-m1-control-flow",
  entry: source,
  emission: "c",
  optimization: "dev",
  abi: {
    prefix: "jamscript_m0_scalar_",
    init_symbol: "jamscript_m0_scalar_init",
    sink_register_symbol: "jamscript_m0_scalar_set_panic_sink",
    collect_symbol: null,
    result_reset_symbol: null,
  },
  exports: [{ export: "add", symbol: "jamscript_m0_scalar_entry", params: ["f64", "f64"], returns: "f64" }],
  determinism: { fences: [] },
}, null, 2));
const result = await compileLibrary({
  profilePath,
  outDir: output,
  outPath: resolve(output, "conformance.lib.a"),
  emitIr: true,
});
if (!result.ok) throw new Error(JSON.stringify(result.diagnostics));
const fixtures = [
  ["arrays", resolve(root, "../m0/samples/arrays.ts"), "sum", [], "f64"],
  ["objects", resolve(root, "objects.ts"), "project", ["f64"], "f64"],
  ["strings", resolve(root, "../m0/samples/strings.ts"), "normalize", ["string"], "string"],
  ["uint8array", resolve(root, "../m0/samples/bytes.ts"), "firstByte", ["bytes"], "f64"],
];
const fixtureResults = [];
for (const [name, entry, exported, params, returns] of fixtures) {
  const fixtureDir = resolve(output, name);
  await mkdir(fixtureDir, { recursive: true });
  const fixtureProfile = resolve(fixtureDir, `${name}.profile.json`);
  await writeFile(fixtureProfile, JSON.stringify({
    profile_format: 1,
    name: `jamscript-m1-${name}`,
    entry,
    emission: "c",
    optimization: "dev",
    abi: {
      prefix: `jamscript_m1_${name}_`,
      init_symbol: `jamscript_m1_${name}_init`,
      sink_register_symbol: `jamscript_m1_${name}_set_panic_sink`,
      collect_symbol: null,
      result_reset_symbol: null,
    },
    exports: [{ export: exported, symbol: `jamscript_m1_${name}_entry`, params, returns }],
    determinism: { fences: [] },
  }, null, 2));
  const fixture = await compileLibrary({
    profilePath: fixtureProfile,
    outDir: fixtureDir,
    outPath: resolve(fixtureDir, `${name}.lib.a`),
    emitIr: true,
  });
  if (!fixture.ok) throw new Error(`${name}: ${JSON.stringify(fixture.diagnostics)}`);
  fixtureResults.push({ name, status: "PASS", c: fixture.cPath, archive: fixture.archivePath });
}
await writeFile(resolve(output, "result.json"), JSON.stringify({
  status: "PASS",
  source,
  archive: result.archivePath,
  c: result.cPath,
  backend: result.backend,
  fixtures: fixtureResults,
}, null, 2));
console.log(JSON.stringify({ status: "PASS", c: result.cPath, archive: result.archivePath, fixtures: fixtureResults }));
