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

function transform(sourceText, parameters = [], returnType = undefined, transformService = service) {
  const { source, declaration } = bodyOf(sourceText);
  const transformed = transformNumericFunction(declaration.body, parameters, returnType, transformService);
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

assert.throws(
  () => transform(
    "function execute(value: number): u8 { let x: u8 = 1; let y: number = value; x = y; return x; }",
    [{ name: "value", type: "number" }],
    "u8",
  ),
  /JAM1203/,
);
assert.throws(
  () => transform("function execute(value: number): u8 { return value; }", [{ name: "value", type: "number" }], "u8"),
  /JAM1203/,
);
assert.throws(
  () => transform("function execute(): u8 { let value: u8 = 300; return value; }", [], "u8"),
  /JAM1204/,
);
assert.throws(
  () => transform("function execute(value: number): u8 { return value as u8; }", [{ name: "value", type: "number" }], "u8"),
  /JAM1208/,
);
const sameTypeAssertion = transform("function execute(value: u8): u8 { return value as u8; }", [{ name: "value", type: "u8" }], "u8");
assert.doesNotMatch(sameTypeAssertion, / as u8/);
assert.throws(
  () => transform(
    "function execute(value: number): u8 { return takeU8(value); }",
    [{ name: "value", type: "number" }],
    "u8",
    { states: [], helpers: [{ name: "takeU8", parameters: [{ type: "U8" }], return_type: "U8" }] },
  ),
  /JAM1203/,
);

for (const sourceType of ["u8", "u16", "u32", "u64", "u128"]) {
  for (const targetType of ["u8", "u16", "u32", "u64", "u128"]) {
    const targetName = targetType[0].toUpperCase() + targetType.slice(1);
    const text = transform(
      `function execute(value: ${sourceType}): ${targetType} { return to${targetName}(value); }`,
      [{ name: "value", type: sourceType }],
      targetType,
    );
    if (sourceType === targetType) {
      assert.doesNotMatch(text, /Identity/);
    } else if (["u8", "u16", "u32"].includes(sourceType)) {
      assert.match(text, new RegExp(`jam${targetName}FromNumber`));
    } else if (sourceType === "u64" && targetType === "u128") {
      assert.match(text, /jamU128FromU64/);
    } else if (sourceType === "u128" && targetType === "u64") {
      assert.match(text, /jamU64FromU128/);
    } else {
      assert.match(text, new RegExp(`jam${targetName}From${sourceType === "u64" ? "U64" : "U128"}`));
    }
  }
}

for (const targetType of ["u8", "u16", "u32", "u64", "u128"]) {
  const targetName = targetType[0].toUpperCase() + targetType.slice(1);
  const text = transform(
    `function execute(value: number): ${targetType} { return to${targetName}(value); }`,
    [{ name: "value", type: "number" }],
    targetType,
  );
  assert.match(text, new RegExp(`jam${targetName}FromNumber`));
}

console.log("SCRIPTC_NUMERIC_TRANSFORM=PASS");
