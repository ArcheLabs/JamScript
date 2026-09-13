import assert from "node:assert/strict";
import ts from "typescript5/lib/typescript.js";
import {
  transformNumericFunction,
  transformNumericFunctionDeclaration,
} from "./transform.mjs";

const service = { states: [], helpers: [] };
const printer = ts.createPrinter({ newLine: ts.NewLineKind.LineFeed });

function bodyOf(sourceText) {
  const source = ts.createSourceFile("numeric-conformance.ts", sourceText, ts.ScriptTarget.Latest, true, ts.ScriptKind.TS);
  const declaration = source.statements.find((statement) => ts.isFunctionDeclaration(statement));
  assert.ok(declaration?.body, "conformance source must contain a function body");
  return { source, declaration };
}

function transform(sourceText, parameters = [], returnType = undefined) {
  const { source, declaration } = bodyOf(sourceText);
  const transformed = transformNumericFunction(declaration.body, parameters, returnType, service);
  return printer.printNode(ts.EmitHint.Unspecified, transformed, source);
}

const arithmetic = transform(
  "function execute(left: u128, right: u128): u128 { let value: u128 = left + 1n; value *= right; return value; }",
  [{ name: "left", type: "u128" }, { name: "right", type: "u128" }],
  "u128",
);
assert.match(arithmetic, /jamU128Const\("1"\)/);
assert.match(arithmetic, /jamU128AddChecked/);
assert.match(arithmetic, /jamU128MulChecked/);

const comparison = transform(
  "function execute(value: u64): boolean { return value >= 18446744073709551615n; }",
  [{ name: "value", type: "u64" }],
  "bool",
);
assert.match(comparison, /jamU64Compare/);
assert.match(comparison, /jamU64Const\("18446744073709551615"\)/);

const cast = transform(
  "function execute(value: u128): u64 { return toU64(value); }",
  [{ name: "value", type: "u128" }],
  "u64",
);
assert.match(cast, /jamU64FromU128/);

const helperSource = bodyOf("function add(left: u128, right: u128): u128 { return left + right; }").declaration;
const helperTransformed = transformNumericFunctionDeclaration(helperSource, service);
const helperText = printer.printNode(ts.EmitHint.Unspecified, helperTransformed, helperSource.getSourceFile());
assert.match(helperText, /JamU128/);
assert.match(helperText, /jamU128AddChecked/);

assert.throws(
  () => transform("function execute(): u128 { return 340282366920938463463374607431768211456n; }", [], "u128"),
  /JAM1204/,
);
assert.throws(
  () => transform(
    "function execute(left: u128, right: u64): u128 { return left + right; }",
    [{ name: "left", type: "u128" }, { name: "right", type: "u64" }],
    "u128",
  ),
  /JAM1203/,
);

console.log("SCRIPTC_NUMERIC_TRANSFORM=PASS");
