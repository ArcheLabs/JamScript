import { readFile, mkdir, writeFile } from "node:fs/promises";
import { resolve } from "node:path";
import ts from "typescript5/lib/typescript.js";
import { compileLibrary } from "@scriptc/compiler";

/*
 * ScriptC receives the original service compilation unit, not a second
 * Legacy ComputeIr program. This adapter removes only JamScript's
 * declarative wrapper and turns the selected execute method into the single
 * stable f64 -> f64 ABI exposed by the M1 backend. The execute body and all
 * reachable top-level helpers remain TypeScript and are compiled by ScriptC.
 */

const specPath = resolve(process.argv[2] ?? "");
if (!specPath) throw new Error("missing M1 ScriptC action spec");
const spec = JSON.parse(await readFile(specPath, "utf8"));
const sourcePath = resolve(spec.source);
const source = await readFile(sourcePath, "utf8");
const output = resolve(spec.output);
await mkdir(output, { recursive: true });

const inputFields = spec.input_fields ?? [];
if (inputFields.length !== 1 || inputFields[0].type !== "u64") {
  throw new Error("ScriptC M1 supports exactly one u64 input field");
}

const transformed = transformService(source, spec.action, inputFields[0].name);
const transformedPath = resolve(output, "scriptc_action.transformed.ts");
await writeFile(transformedPath, transformed.source);

const profilePath = resolve(output, "scriptc_action.profile.json");
const action = spec.action;
const deny = (prefix, teaching, remediation) => ({ prefix, teaching, remediation });
const determinismFences = [
  deny("stdlib.date.now", "wall-clock access is not deterministic", "use action input or managed state"),
  deny("stdlib.math.random", "randomness is not deterministic", "use a signed input"),
  deny("node-builtin.process.env", "process state is not deterministic", "use explicit input"),
  deny("node-builtin.perf_hooks.performance.now", "clock access is not deterministic", "use action input or managed state"),
];
await writeFile(profilePath, JSON.stringify({
  profile_format: 1,
  name: `jamscript-m1-${action}`,
  entry: transformedPath,
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
  determinism: {
    teachings: {
      SC4008: "ScriptC reachable ambient or authority surfaces are forbidden in JAM services",
      JAM1117: "ScriptC reachable non-deterministic language surfaces are forbidden in JAM services",
    },
    remediations: {
      SC4008: "replace the surface with explicit action input or managed state",
      JAM1117: "replace the surface with explicit action input or managed state",
    },
    fences: determinismFences,
  },
}, null, 2));

const result = await compileLibrary({
  profilePath,
  outDir: output,
  outPath: resolve(output, "scriptc_action.lib.a"),
  emitIr: true,
});
if (!result.ok) throw new Error(JSON.stringify(result.diagnostics));

function transformService(text, actionName, inputName) {
  const file = ts.createSourceFile("service.ts", text, ts.ScriptTarget.Latest, true, ts.ScriptKind.TS);
  for (const statement of file.statements) {
    if (!ts.isImportDeclaration(statement)) continue;
    const module = ts.isStringLiteral(statement.moduleSpecifier) ? statement.moduleSpecifier.text : "";
    if (module !== "jam") throw new Error(`ScriptC M1 does not support runtime import \`${module}\``);
  }
  const actionDeclaration = findActionDeclaration(file, actionName);
  if (!actionDeclaration) throw new Error(`exported action \`${actionName}\` was not found`);
  const execute = findExecuteFunction(actionDeclaration.initializer);
  if (!execute || !execute.body) throw new Error("action execute must have a function body");

  const functions = new Map();
  for (const statement of file.statements) {
    if (ts.isFunctionDeclaration(statement) && statement.name) functions.set(statement.name.text, statement);
  }
  const reachable = reachableHelpers(execute, functions);
  inspectDeterminism(execute, reachable, functions);
  const executeInputName = execute.parameters[1]?.name?.getText(file);
  const rewrite = (node) => {
    if (ts.isPropertyAccessExpression(node)
      && ts.isIdentifier(node.expression)
      && node.expression.text === executeInputName
      && node.name.text === inputName) return ts.factory.createIdentifier("input");
    return ts.visitEachChild(node, rewrite, undefined);
  };
  const body = ts.visitNode(execute.body, rewrite);
  const inputParameter = ts.factory.createParameterDeclaration(
    undefined, undefined, "input", undefined,
    ts.factory.createKeywordTypeNode(ts.SyntaxKind.NumberKeyword), undefined,
  );
  const exported = ts.factory.createFunctionDeclaration(
    [ts.factory.createModifier(ts.SyntaxKind.ExportKeyword)], undefined, actionName, undefined, [inputParameter],
    ts.factory.createKeywordTypeNode(ts.SyntaxKind.NumberKeyword), body,
  );
  const retained = [];
  for (const statement of file.statements) {
    if (ts.isImportDeclaration(statement) || statement === actionDeclaration.statement) continue;
    if (ts.isFunctionDeclaration(statement) && statement.name && reachable.has(statement.name.text)) retained.push(statement);
  }
  retained.push(exported);
  const printer = ts.createPrinter({ newLine: ts.NewLineKind.LineFeed });
  return { source: retained.map((node) => printer.printNode(ts.EmitHint.Unspecified, node, file)).join("\n\n") + "\n" };
}

function findActionDeclaration(file, actionName) {
  for (const statement of file.statements) {
    if (!ts.isVariableStatement(statement)) continue;
    for (const declaration of statement.declarationList.declarations) {
      if (ts.isIdentifier(declaration.name) && declaration.name.text === actionName
        && declaration.initializer && ts.isCallExpression(declaration.initializer)
        && ts.isIdentifier(declaration.initializer.expression)
        && declaration.initializer.expression.text === "action") {
        return { statement, initializer: declaration.initializer };
      }
    }
  }
  return undefined;
}

function findExecuteFunction(actionCall) {
  const config = actionCall.arguments[0];
  if (!config || !ts.isObjectLiteralExpression(config)) return undefined;
  for (const property of config.properties) {
    const name = property.name && property.name.getText(config.getSourceFile());
    if (name !== "execute") continue;
    if (ts.isMethodDeclaration(property)) return property;
    if (ts.isPropertyAssignment(property) && ts.isFunctionExpression(property.initializer)) return property.initializer;
  }
  return undefined;
}

function calledFunctionNames(node) {
  const names = new Set();
  const visit = (current) => {
    if (ts.isCallExpression(current) && ts.isIdentifier(current.expression)) names.add(current.expression.text);
    ts.forEachChild(current, visit);
  };
  visit(node);
  return names;
}

function reachableHelpers(execute, functions) {
  const reachable = new Set();
  const queue = [...calledFunctionNames(execute.body)];
  while (queue.length) {
    const name = queue.shift();
    if (reachable.has(name) || !functions.has(name)) continue;
    reachable.add(name);
    queue.push(...calledFunctionNames(functions.get(name).body ?? functions.get(name)));
  }
  return reachable;
}

function inspectDeterminism(execute, reachable, functions) {
  inspectNode(execute.body, "execute");
  for (const name of reachable) inspectNode(functions.get(name).body ?? functions.get(name), name);
}

function inspectNode(root, owner) {
  const forbidden = (surface) => {
    throw new Error(`JAM1117: ScriptC deterministic profile forbids reachable surface ${surface} in ${owner}`);
  };
  const visit = (node) => {
    if (ts.isAwaitExpression(node) || (ts.isFunctionLike(node) && node.modifiers?.some((m) => m.kind === ts.SyntaxKind.AsyncKeyword))) forbidden("async/await");
    if (ts.isNewExpression(node) && ts.isIdentifier(node.expression) && node.expression.text === "Date") forbidden("Date");
    if (ts.isCallExpression(node)) {
      if (node.expression.kind === ts.SyntaxKind.ImportKeyword) forbidden("dynamic module loading");
      if (ts.isIdentifier(node.expression) && ["fetch", "setTimeout", "setInterval", "setImmediate", "eval", "require", "Function"].includes(node.expression.text)) forbidden(node.expression.text);
    }
    if (ts.isIdentifier(node) && ["Date", "Promise", "process", "globalThis", "ctx"].includes(node.text)) forbidden(node.text);
    if (ts.isPropertyAccessExpression(node)) {
      const full = node.getText(node.getSourceFile());
      if (full === "Math.random" || full === "Date.now" || full === "performance.now" || full.startsWith("process.")) forbidden(full);
    }
    ts.forEachChild(node, visit);
  };
  visit(root);
}
