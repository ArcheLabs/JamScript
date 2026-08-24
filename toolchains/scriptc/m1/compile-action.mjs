import { readFile, mkdir, writeFile } from "node:fs/promises";
import { resolve } from "node:path";
import { compileLibrary } from "@scriptc/compiler";

const specPath = resolve(process.argv[2] ?? "");
if (!specPath) throw new Error("missing M1 ScriptC action spec");
const spec = JSON.parse(await readFile(specPath, "utf8"));
const output = resolve(spec.output);
await mkdir(output, { recursive: true });
const profilePath = resolve(output, "scriptc_action.profile.json");
const action = spec.action;
await writeFile(profilePath, JSON.stringify({
  profile_format: 1,
  name: `jamscript-m1-${action}`,
  entry: resolve(spec.source),
  emission: "c",
  optimization: "dev",
  abi: {
    prefix: `jamscript_scriptc_${action}_`,
    init_symbol: `jamscript_scriptc_${action}_init`,
    sink_register_symbol: `jamscript_scriptc_${action}_set_panic_sink`,
    collect_symbol: null,
    result_reset_symbol: null,
  },
  exports: [{ export: action, symbol: `jamscript_scriptc_${action}_entry`, params: ["f64"], returns: "f64" }],
  determinism: { fences: [] },
}, null, 2));
const result = await compileLibrary({ profilePath, outDir: output, outPath: resolve(output, "scriptc_action.lib.a"), emitIr: true });
if (!result.ok) throw new Error(JSON.stringify(result.diagnostics));
