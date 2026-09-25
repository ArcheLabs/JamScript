import fs from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";
import ts from "typescript";

const packageRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const repositoryRoot = path.resolve(packageRoot, "../..");
const fixture = path.join(repositoryRoot, "tests/scriptc-matrix-ownership-adapter");
const output = process.argv[2] ? path.resolve(process.argv[2]) : "";
if (!output) throw new Error("usage: bundle-matrix-scriptc-probe.mjs <output-directory>");

const adapterFiles = [
  "src/ownership/matrix/proof-runtime.ts",
  "src/ownership/matrix/service-payload.ts",
  "src/ownership/matrix/service-scriptc.ts",
].map(relative => path.join(packageRoot, relative));

function inlineModule(file, { stripExports = false, removeAdapterImport = false } = {}) {
  const original = ts.sys.readFile(file);
  if (original === undefined) throw new Error(`cannot read ${file}`);
  const source = ts.createSourceFile(file, original, ts.ScriptTarget.Latest, true, ts.ScriptKind.TS);
  const edits = [];
  const visit = node => {
    if (ts.isImportDeclaration(node) || ts.isExportDeclaration(node)) {
      const specifier = node.moduleSpecifier;
      const adapterImport = ts.isStringLiteral(specifier)
        && specifier.text === "@jamscript/client/ownership/matrix/service-scriptc";
      if (stripExports || (removeAdapterImport && adapterImport)) {
        edits.push([node.getFullStart(), node.end, ""]);
      }
    }
    if (stripExports && ts.canHaveModifiers(node)) {
      for (const modifier of ts.getModifiers(node) ?? []) {
        if (modifier.kind === ts.SyntaxKind.ExportKeyword) {
          edits.push([modifier.getStart(source), modifier.end, ""]);
        }
      }
    }
    ts.forEachChild(node, visit);
  };
  visit(source);
  let text = original;
  for (const [start, end, replacement] of edits.sort((a, b) => b[0] - a[0])) {
    text = text.slice(0, start) + replacement + text.slice(end);
  }
  return text;
}

await fs.rm(output, { recursive: true, force: true });
await fs.cp(fixture, output, { recursive: true });
const adapter = adapterFiles.map(file => inlineModule(file, { stripExports: true })).join("\n\n");
const serviceFile = path.join(output, "src/service.ts");
const service = inlineModule(serviceFile, { removeAdapterImport: true });
await fs.writeFile(serviceFile, `${adapter}\n\n${service}`, "utf8");
console.log("OWNERSHIP_MATRIX_SCRIPTC_PROBE_BUNDLE=PASS");
