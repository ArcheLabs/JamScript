import { readFile, mkdir, writeFile, copyFile } from "node:fs/promises";
import { resolve } from "node:path";
import ts from "typescript5/lib/typescript.js";
import { compileLibrary } from "@scriptc/compiler";

const specPath = resolve(process.argv[2] ?? "");
if (!specPath) throw new Error("missing M2 ScriptC service spec");
const spec = JSON.parse(await readFile(specPath, "utf8"));
const sourcePath = resolve(spec.source);
const source = await readFile(sourcePath, "utf8");
const output = resolve(spec.output);
await mkdir(output, { recursive: true });

const transformedPath = resolve(output, "scriptc_service.transformed.ts");
const runtimePath = resolve(output, "scriptc_runtime.ts");
await copyFile(resolve(import.meta.dirname, "runtime.ts"), runtimePath);
await writeFile(transformedPath, transformService(source, spec));

const profilePath = resolve(output, "scriptc_service.profile.json");
const exports = spec.actions.map((action) => ({
  export: `__jamscript_action_${action.name}_v2`,
  symbol: `jamscript_scriptc_${action.name}_entry_v2`,
  params: ["bytes", "bytes", "bytes"],
  returns: "bytes",
}));
await writeFile(profilePath, JSON.stringify({
  profile_format: 1,
  name: `jamscript-m2-${spec.package_name}`,
  entry: transformedPath,
  emission: "c",
  optimization: "dev",
  abi: {
    prefix: "jamscript_scriptc_",
    init_symbol: "jamscript_scriptc_service_init",
    sink_register_symbol: "jamscript_scriptc_service_set_panic_sink",
    collect_symbol: null,
    result_reset_symbol: null,
    localize_runtime: false,
  },
  exports,
  determinism: spec.determinism,
}, null, 2));

const result = await compileLibrary({
  profilePath,
  outDir: output,
  outPath: resolve(output, "scriptc_service.lib.a"),
  emitIr: true,
});
if (!result.ok) throw new Error(JSON.stringify(result.diagnostics, null, 2));

function transformService(text, service) {
  const file = ts.createSourceFile("service.ts", text, ts.ScriptTarget.Latest, true, ts.ScriptKind.TS);
  validateImports(file);
  validateDeterminism(file, service);
  const actionBodies = new Map();
  const stateNames = new Set(service.states.map((state) => state.name));
  const actionNames = new Set(service.actions.map((action) => action.name));
  const queryNames = new Set((service.queries ?? []).map((query) => query.name));
  for (const statement of file.statements) {
    if (!ts.isVariableStatement(statement)) continue;
    for (const declaration of statement.declarationList.declarations) {
      if (!ts.isIdentifier(declaration.name) || !actionNames.has(declaration.name.text)) continue;
      const execute = findExecuteFunction(declaration.initializer);
      if (!execute?.body) throw new Error(`action ${declaration.name.text} has no execute body`);
      actionBodies.set(declaration.name.text, execute);
    }
  }
  const printer = ts.createPrinter({ newLine: ts.NewLineKind.LineFeed });
  const retained = [];
  for (const statement of file.statements) {
    if (ts.isImportDeclaration(statement)) continue;
    if (ts.isVariableStatement(statement)) {
      const names = statement.declarationList.declarations
        .filter((declaration) => ts.isIdentifier(declaration.name))
        .map((declaration) => declaration.name.text);
      if (names.some((name) => stateNames.has(name) || actionNames.has(name) || queryNames.has(name))) continue;
      if (isSchemaDeclaration(statement)) continue;
    }
    retained.push(printer.printNode(ts.EmitHint.Unspecified, statement, file));
  }

  const sections = [runtimeImports(), codecRuntime(), ...retained];
  for (const state of service.states) sections.push(generateStateBinding(state));
  for (const action of service.actions) {
    const execute = actionBodies.get(action.name);
    if (!execute) throw new Error(`action ${action.name} was not found in service source`);
    sections.push(generateAction(action, execute, printer, file));
  }
  return sections.join("\n\n") + "\n";
}

function validateImports(file) {
  for (const statement of file.statements) {
    if (!ts.isImportDeclaration(statement)) continue;
    const module = ts.isStringLiteral(statement.moduleSpecifier) ? statement.moduleSpecifier.text : "";
    if (module !== "jam") throw new Error(`ScriptC M2 does not support runtime import \`${module}\``);
  }
}

function validateDeterminism(file, service) {
  const publicActions = new Set(service.actions.filter((action) => action.auth === "Public").map((action) => action.name));
  const forbidden = (surface) => { throw new Error(`JAM1117: ScriptC deterministic profile forbids ${surface}`); };
  const visit = (node) => {
    if (ts.isAwaitExpression(node) || (ts.isFunctionLike(node) && node.modifiers?.some((m) => m.kind === ts.SyntaxKind.AsyncKeyword))) forbidden("async/await");
    if (ts.isNewExpression(node) && ts.isIdentifier(node.expression) && node.expression.text === "Date") forbidden("Date");
    if (ts.isCallExpression(node)) {
      if (node.expression.kind === ts.SyntaxKind.ImportKeyword) forbidden("dynamic import");
      if (ts.isIdentifier(node.expression) && ["fetch", "setTimeout", "setInterval", "setImmediate", "eval", "require", "Function"].includes(node.expression.text)) forbidden(node.expression.text);
    }
    if (ts.isPropertyAccessExpression(node)) {
      const full = node.getText(file);
      if (full === "Math.random" || full === "Date.now" || full === "performance.now" || full.startsWith("process.")) forbidden(full);
    }
    ts.forEachChild(node, visit);
  };
  visit(file);
  for (const statement of file.statements) {
    if (!ts.isVariableStatement(statement)) continue;
    for (const declaration of statement.declarationList.declarations) {
      if (!ts.isIdentifier(declaration.name) || !publicActions.has(declaration.name.text)) continue;
      const execute = findExecuteFunction(declaration.initializer);
      if (execute?.body.getText(file).includes("ctx.sender")) {
        throw new Error(`JAM1118: public action ${declaration.name.text} cannot use ctx.sender`);
      }
    }
  }
}

function findExecuteFunction(initializer) {
  if (!initializer || !ts.isCallExpression(initializer)) return undefined;
  const config = initializer.arguments[0];
  if (!config || !ts.isObjectLiteralExpression(config)) return undefined;
  for (const property of config.properties) {
    const name = property.name?.getText(config.getSourceFile());
    if (name !== "execute") continue;
    if (ts.isMethodDeclaration(property)) return property;
    if (ts.isPropertyAssignment(property) && (ts.isFunctionExpression(property.initializer) || ts.isArrowFunction(property.initializer))) return property.initializer;
  }
  return undefined;
}

function isSchemaDeclaration(statement) {
  return statement.declarationList.declarations.some((declaration) => declaration.initializer
    && ts.isCallExpression(declaration.initializer)
    && ts.isIdentifier(declaration.initializer.expression)
    && ["bytes", "string", "record", "tuple", "fixedArray", "array", "option", "enumeration", "result"].includes(declaration.initializer.expression.text));
}

function runtimeImports() {
  return `import {\n  abort, applicationKeyV1, appliedResult, caughtResult, initializeStateView,\n  stateDeleteRaw, stateGetRaw, stateHasRaw, stateSetRaw,\n} from "./scriptc_runtime.js";\nexport { abort };`;
}

function codecRuntime() {
  return `type JamCursor = { input: Uint8Array; offset: number };\nfunction jamTake(cursor: JamCursor, length: number): Uint8Array { const end = cursor.offset + length; if (length < 0 || end < cursor.offset || end > cursor.input.length) throw new Error("invalid JAM bytes"); const value = cursor.input.slice(cursor.offset, end); cursor.offset = end; return value; }\nfunction jamU8(cursor: JamCursor): number { return jamTake(cursor, 1)[0]; }\nfunction jamU16(cursor: JamCursor): number { const b = jamTake(cursor, 2); return b[0] + b[1] * 256; }\nfunction jamU32(cursor: JamCursor): number { const b = jamTake(cursor, 4); return b[0] + b[1] * 256 + b[2] * 65536 + b[3] * 16777216; }\nfunction jamEncodeU8(value: number): Uint8Array { if (value < 0 || value > 255 || Math.floor(value) !== value) throw new Error("u8 out of range"); return new Uint8Array([value]); }\nfunction jamEncodeU16(value: number): Uint8Array { if (value < 0 || value > 65535 || Math.floor(value) !== value) throw new Error("u16 out of range"); return new Uint8Array([value & 255, (value >>> 8) & 255]); }\nfunction jamEncodeU32(value: number): Uint8Array { if (value < 0 || value > 4294967295 || Math.floor(value) !== value) throw new Error("u32 out of range"); return new Uint8Array([value & 255, (value >>> 8) & 255, (value >>> 16) & 255, (value >>> 24) & 255]); }\nfunction jamConcat(parts: Uint8Array[]): Uint8Array { let length = 0; for (const part of parts) length += part.length; const output = new Uint8Array(length); let offset = 0; for (const part of parts) { output.set(part, offset); offset += part.length; } return output; }\nfunction jamNatural(cursor: JamCursor): number { const first = jamU8(cursor); if (first < 128) return first; let length = 0; while (length < 8 && (first & (128 >>> length)) !== 0) length += 1; if (length === 0 || length > 7) throw new Error("invalid JAM natural"); const low = jamTake(cursor, length); let multiplier = 1; let value = 0; for (let index = 0; index < length; index += 1) { value += low[index] * multiplier; multiplier *= 256; } return value + (first & (127 >>> length)) * multiplier; }\nfunction jamEncodeNatural(value: number): Uint8Array { if (value < 0 || value > 4294967295 || Math.floor(value) !== value) throw new Error("natural out of range"); if (value < 128) return new Uint8Array([value]); let length = 1; let threshold = 16384; while (length < 4 && value >= threshold) { length += 1; threshold *= 128; } let divisor = 1; for (let index = 0; index < length; index += 1) divisor *= 256; const output = new Uint8Array(1 + length); output[0] = ((256 - (1 << (8 - length))) & 255) | (Math.floor(value / divisor) & (127 >>> length)); let multiplier = 1; for (let index = 0; index < length; index += 1) { output[index + 1] = Math.floor(value / multiplier) & 255; multiplier *= 256; } return output; }\nfunction jamFixed(value: Uint8Array, length: number): Uint8Array { if (value.length !== length) throw new Error("fixed bytes length"); return value.slice(); }\nfunction jamBounded(value: Uint8Array, max: number): Uint8Array { if (value.length > max) throw new Error("bounded bytes length"); return jamConcat([jamEncodeNatural(value.length), value]); }`;
}

function generateStateBinding(state) {
  const suffix = safe(state.name);
  const namespace = [...new TextEncoder().encode(state.schema)].join(", ");
  const encodeKey = state.kind === "Scalar" ? "new Uint8Array(0)" : encodeExpression(state.key_type, "key");
  const keyType = state.kind === "Scalar" ? "void" : tsType(state.key_type);
  const valueType = tsType(state.value_type);
  const decodeValue = decoderFunction(`decode_${suffix}_value`, state.value_type);
  const encodeValue = encoderFunction(`encode_${suffix}_value`, state.value_type, valueType);
  const keyParam = state.kind === "Scalar" ? "" : `key: ${keyType}`;
  const keyArg = state.kind === "Scalar" ? "" : "key";
  return `const namespace_${suffix} = new Uint8Array([${namespace}]);\n${decodeValue}\n${encodeValue}\nfunction key_${suffix}(${keyParam}): Uint8Array { const canonical = ${encodeKey}; return applicationKeyV1(namespace_${suffix}, canonical); }\nconst ${state.name} = {\n  get(${keyParam}): ${valueType} | null { const raw = stateGetRaw(key_${suffix}(${keyArg})); return raw === null ? null : decode_${suffix}_value(raw); },\n  has(${keyParam}): boolean { return stateHasRaw(key_${suffix}(${keyArg})); },\n  set(${state.kind === "Scalar" ? `value: ${valueType}` : `key: ${keyType}, value: ${valueType}`}): void { stateSetRaw(key_${suffix}(${keyArg}), encode_${suffix}_value(value)); },\n  delete(${keyParam}): void { stateDeleteRaw(key_${suffix}(${keyArg})); },\n};`;
}

function generateAction(action, execute, printer, file) {
  const suffix = safe(action.name);
  const body = printer.printNode(ts.EmitHint.Unspecified, execute.body, file);
  const fields = action.input.map((field) => `${field.name}: ${tsType(field.ty)}`).join("; ");
  const inputType = `{ ${fields} }`;
  const decode = decoderFunction(`decode_${suffix}_input`, { Record: { fields: action.input } });
  const senderCheck = action.auth === "Wallet" ? "if (sender.length !== 32) throw new Error(\"wallet sender length\");" : "if (sender.length !== 0) throw new Error(\"public sender must be empty\");";
  return `${decode}\nfunction execute_${suffix}(ctx: { sender: Uint8Array }, input: ${inputType}): void ${body}\nexport function __jamscript_action_${action.name}_v2(payload: Uint8Array, sender: Uint8Array, stateView: Uint8Array): Uint8Array { try { initializeStateView(stateView); ${senderCheck} const input = decode_${suffix}_input(payload); execute_${suffix}({ sender }, input); return appliedResult(); } catch (error) { return caughtResult(error); } }`;
}

function decoderFunction(name, type) {
  const lines = [];
  let id = 0;
  const value = decodeExpression(type, "cursor", lines, () => `v${id++}`);
  return `function ${name}(raw: Uint8Array): ${tsType(type)} { const cursor: JamCursor = { input: raw, offset: 0 }; ${lines.join(" ")} const result = ${value}; if (cursor.offset !== raw.length) throw new Error("trailing JAM bytes"); return result; }`;
}

function encoderFunction(name, type, typeName) {
  return `function ${name}(value: ${typeName}): Uint8Array { return ${encodeExpression(type, "value")}; }`;
}

function decodeExpression(type, cursor, lines, next) {
  const [kind, data] = typeParts(type);
  if (kind === "Unit") return "undefined";
  if (kind === "U8") return `jamU8(${cursor})`;
  if (kind === "U16") return `jamU16(${cursor})`;
  if (kind === "U32") return `jamU32(${cursor})`;
  if (kind === "Bool") { const name = next(); lines.push(`const ${name} = jamU8(${cursor}); if (${name} > 1) throw new Error("invalid bool");`); return `${name} === 1`; }
  if (kind === "Address") return `jamTake(${cursor}, 32)`;
  if (kind === "FixedBytes") return `jamTake(${cursor}, ${data.len})`;
  if (kind === "Bytes" || kind === "String") {
    const length = next();
    const value = next();
    if (kind === "String") throw new Error("ScriptC M2 string execution is not implemented yet");
    lines.push(`const ${length} = jamNatural(${cursor}); if (${length} > ${data.max}) throw new Error("bound exceeded"); const ${value} = jamTake(${cursor}, ${length});`);
    return value;
  }
  if (kind === "Record") {
    const fields = data.fields.map((field) => `${field.name}: ${decodeExpression(field.ty, cursor, lines, next)}`);
    return `{ ${fields.join(", ")} }`;
  }
  throw new Error(`ScriptC M2 refuses unsupported executable codec type ${kind}`);
}

function encodeExpression(type, value) {
  const [kind, data] = typeParts(type);
  if (kind === "Unit") return "new Uint8Array(0)";
  if (kind === "U8") return `jamEncodeU8(${value})`;
  if (kind === "U16") return `jamEncodeU16(${value})`;
  if (kind === "U32") return `jamEncodeU32(${value})`;
  if (kind === "Bool") return `jamEncodeU8(${value} ? 1 : 0)`;
  if (kind === "Address") return `jamFixed(${value}, 32)`;
  if (kind === "FixedBytes") return `jamFixed(${value}, ${data.len})`;
  if (kind === "Bytes") return `jamBounded(${value}, ${data.max})`;
  if (kind === "String") throw new Error("ScriptC M2 string execution is not implemented yet");
  if (kind === "Record") return `jamConcat([${data.fields.map((field) => encodeExpression(field.ty, `${value}.${field.name}`)).join(", ")}])`;
  throw new Error(`ScriptC M2 refuses unsupported executable codec type ${kind}`);
}

function tsType(type) {
  const [kind, data] = typeParts(type);
  if (["U8", "U16", "U32"].includes(kind)) return "number";
  if (kind === "Bool") return "boolean";
  if (["Address", "FixedBytes", "Bytes"].includes(kind)) return "Uint8Array";
  if (kind === "String") return "string";
  if (kind === "Unit") return "void";
  if (kind === "Record") return `{ ${data.fields.map((field) => `${field.name}: ${tsType(field.ty)}`).join("; ")} }`;
  throw new Error(`ScriptC M2 refuses unsupported executable type ${kind}`);
}

function typeParts(type) {
  if (typeof type === "string") return [type, {}];
  const kind = Object.keys(type)[0];
  return [kind, type[kind]];
}

function safe(name) {
  return name.replace(/[^A-Za-z0-9_]/g, "_");
}
