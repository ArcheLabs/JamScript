import { mkdir, readFile, rm, writeFile } from "node:fs/promises";
import { resolve } from "node:path";
import ts from "typescript5/lib/typescript.js";
import { compileLibrary } from "@scriptc/compiler";

const root = import.meta.dirname;
const output = resolve(root, "out");
await rm(output, { recursive: true, force: true });
await mkdir(output, { recursive: true });

const source = await readFile(resolve(root, "numeric-probe.ts"), "utf8");
const file = ts.createSourceFile("numeric-probe.ts", source, ts.ScriptTarget.Latest, true, ts.ScriptKind.TS);
const parseDiagnostics = file.parseDiagnostics ?? [];
const parseOk = parseDiagnostics.length === 0;

// BigInt typechecking is intentionally performed with the pinned TypeScript
// package.  ScriptC's own lowering remains the authoritative later stages.
const options = {
  target: ts.ScriptTarget.ESNext,
  module: ts.ModuleKind.ESNext,
  lib: ["lib.esnext.d.ts"],
  noEmit: true,
  skipLibCheck: true,
};
const host = ts.createCompilerHost(options);
const originalRead = host.readFile;
host.readFile = (fileName) => fileName.endsWith("numeric-probe.ts") ? source : originalRead(fileName);
const originalFileExists = host.fileExists;
host.fileExists = (fileName) => fileName.endsWith("numeric-probe.ts") || originalFileExists(fileName);
const program = ts.createProgram(["numeric-probe.ts"], options, host);
const typeDiagnostics = ts.getPreEmitDiagnostics(program).filter((diagnostic) => diagnostic.file?.fileName.endsWith("numeric-probe.ts"));
const typecheckOk = typeDiagnostics.length === 0;

let lowerOk = false;
let cEmitOk = false;
let linkOk = false;
let runOk = false;
let diagnostics = [];
if (parseOk && typecheckOk) {
  try {
    const result = await compileLibrary({
      profilePath: resolve(root, "numeric-probe.profile.json"),
      outDir: output,
      outPath: resolve(output, "numeric-probe.a"),
      emitIr: true,
    });
    lowerOk = Boolean(result.ok);
    cEmitOk = lowerOk && Boolean(result.cPath);
    diagnostics = result.diagnostics ?? [];
  } catch (error) {
    diagnostics = [{ message: String(error) }];
  }
}

const report = {
  parse: parseOk ? "PASS" : "FAIL",
  typecheck: typecheckOk ? "PASS" : "FAIL",
  lower: lowerOk ? "PASS" : "FAIL",
  c_emit: cEmitOk ? "PASS" : "FAIL",
  polkavm_link: linkOk ? "PASS" : "FAIL",
  pvm_run: runOk ? "PASS" : "FAIL",
  diagnostics,
};
await writeFile(resolve(output, "report.json"), JSON.stringify(report, null, 2) + "\n");
for (const [name, value] of Object.entries({
  SCRIPTC_BIGINT_PARSE: report.parse,
  SCRIPTC_BIGINT_TYPECHECK: report.typecheck,
  SCRIPTC_BIGINT_LOWER: report.lower,
  SCRIPTC_BIGINT_C_EMIT: report.c_emit,
  SCRIPTC_BIGINT_POLKAVM_LINK: report.polkavm_link,
  SCRIPTC_BIGINT_PVM_RUN: report.pvm_run,
})) console.log(`${name}=${value}`);
console.log(`SCRIPTC_NUMERIC_BACKEND=${lowerOk && cEmitOk && linkOk && runOk ? "native-bigint" : "fixed-limb-u64-u128"}`);
