import assert from "node:assert/strict";
import { mkdtemp, readFile, writeFile } from "node:fs/promises";
import { resolve } from "node:path";
import { spawnSync } from "node:child_process";
import vm from "node:vm";
import ts from "typescript5/lib/typescript.js";

const root = import.meta.dirname;
const fixture = resolve(root, "decoder-conformance.ts");
const output = await mkdtemp("/tmp/jamscript-decoder-conformance.");
const specPath = resolve(output, "decoder-service.json");
const spec = {
  source: fixture,
  output,
  package_name: "decoder-conformance",
  language_version: "0.3",
  states: [],
  actions: [
    {
      name: "fixedDynamicFixed",
      auth: "Wallet",
      input: [
        { name: "head", ty: "U32" },
        { name: "payload", ty: { Bytes: { max: 8 } } },
        { name: "tail", ty: "U128" },
      ],
    },
    {
      name: "twoDynamicFields",
      auth: "Wallet",
      input: [
        { name: "head", ty: "U8" },
        { name: "name", ty: { Bytes: { max: 64 } } },
        { name: "symbol", ty: { Bytes: { max: 16 } } },
        { name: "tail", ty: "U128" },
      ],
    },
    {
      name: "locusCreateAssetShape",
      auth: "Wallet",
      input: [
        { name: "issuerId", ty: "Address" },
        { name: "nonce", ty: "U64" },
        { name: "assetId", ty: "Address" },
        { name: "name", ty: { Bytes: { max: 64 } } },
        { name: "symbol", ty: { Bytes: { max: 16 } } },
        { name: "decimals", ty: "U8" },
        { name: "initialSupply", ty: "U128" },
      ],
    },
  ],
  queries: [],
};

await writeFile(specPath, JSON.stringify(spec));
const compile = spawnSync(process.execPath, [resolve(root, "compile-service.mjs"), specPath], {
  cwd: root,
  encoding: "utf8",
});
if (compile.status !== 0) throw new Error(compile.stderr || compile.stdout);

const generated = await readFile(resolve(output, "scriptc_service.transformed.ts"), "utf8");
const cases = [
  {
    marker: "FIXED_DYNAMIC_FIXED",
    name: "fixedDynamicFixed",
    fields: [
      ["head", { kind: "u32" }, 0xff000001],
      ["payload", { kind: "bytes", max: 8 }, Uint8Array.of(0xaa, 0xbb)],
      ["tail", { kind: "u128" }, 0x112233445566778899aabbccddeeff00n],
    ],
  },
  {
    marker: "TWO_DYNAMIC_FIELDS",
    name: "twoDynamicFields",
    fields: [
      ["head", { kind: "u8" }, 0xff],
      ["name", { kind: "bytes", max: 64 }, Uint8Array.of(0x6e, 0x61, 0x6d, 0x65)],
      ["symbol", { kind: "bytes", max: 16 }, Uint8Array.of(0x41, 0x42, 0x43)],
      ["tail", { kind: "u128" }, 7n],
    ],
  },
  {
    marker: "LOCUS_CREATE_ASSET_SHAPE",
    name: "locusCreateAssetShape",
    fields: [
      ["issuerId", { kind: "fixed", len: 32 }, new Uint8Array(32).fill(0xff)],
      ["nonce", { kind: "u64" }, 3n],
      ["assetId", { kind: "fixed", len: 32 }, new Uint8Array(32).fill(0x11)],
      ["name", { kind: "bytes", max: 64 }, Uint8Array.of(0x41, 0x73, 0x73, 0x65, 0x74)],
      ["symbol", { kind: "bytes", max: 16 }, Uint8Array.of(0x41, 0x42, 0x43)],
      ["decimals", { kind: "u8" }, 18],
      ["initialSupply", { kind: "u128" }, 100000000000000000000n],
    ],
  },
];

for (const test of cases) {
  const encoded = encodeFields(test.fields);
  const reference = decodeSequential(test.fields, encoded);
  assert.deepEqual(
    reference,
    Object.fromEntries(test.fields.map(([name, type, value]) => [name, expectedValue(type, value)])),
  );

  assert.throws(
    () => decodeHoisted(test.fields, encoded),
    /invalid JAM natural|bound exceeded|trailing JAM bytes/,
    `${test.name}: the old hoisted decoder must fail on canonical bytes`,
  );
  const decoder = await loadGeneratedDecoder(generated, test.name);
  assert.deepEqual(normalize(decoder(encoded)), normalize(reference), `${test.name}: generated decoder round trip`);
  console.log(`${test.marker}=PASS`);
}

console.log("DYNAMIC_FIELD_REPRO=PASS");

function encodeFields(fields) {
  return concat(fields.map(([, type, value]) => encodeValue(type, value)));
}

function encodeValue(type, value) {
  if (type.kind === "u8") return Uint8Array.of(value);
  if (type.kind === "u32") return littleEndian(value, 4);
  if (type.kind === "u64") return littleEndian(value, 8);
  if (type.kind === "u128") return littleEndian(value, 16);
  if (type.kind === "fixed") {
    assert.equal(value.length, type.len);
    return value;
  }
  if (type.kind === "bytes") {
    assert.ok(value.length <= type.max);
    return concat([Uint8Array.of(value.length), value]);
  }
  throw new Error(`unsupported test type ${type.kind}`);
}

function expectedValue(type, value) {
  if (type.kind === "u64" || type.kind === "u128") return words(encodeValue(type, value));
  return value;
}

function decodeSequential(fields, input) {
  const cursor = { input, offset: 0 };
  const result = {};
  for (const [name, type] of fields) result[name] = decodeValue(type, cursor);
  assert.equal(cursor.offset, input.length, "reference decoder cursor must consume all bytes");
  return result;
}

function decodeHoisted(fields, input) {
  const cursor = { input, offset: 0 };
  const dynamic = new Map();
  for (const [name, type] of fields) {
    if (type.kind !== "bytes") continue;
    const length = natural(cursor);
    if (length > type.max) throw new Error("bound exceeded");
    dynamic.set(name, take(cursor, length));
  }
  const result = {};
  for (const [name, type] of fields) {
    result[name] = type.kind === "bytes" ? dynamic.get(name) : decodeValue(type, cursor);
  }
  if (cursor.offset !== input.length) throw new Error("trailing JAM bytes");
  return result;
}

function decodeValue(type, cursor) {
  if (type.kind === "u8") return take(cursor, 1)[0];
  if (type.kind === "u32") return numberFromLittleEndian(take(cursor, 4));
  if (type.kind === "u64") return words(take(cursor, 8));
  if (type.kind === "u128") return words(take(cursor, 16));
  if (type.kind === "fixed") return take(cursor, type.len);
  if (type.kind === "bytes") {
    const length = natural(cursor);
    if (length > type.max) throw new Error("bound exceeded");
    return take(cursor, length);
  }
  throw new Error(`unsupported test type ${type.kind}`);
}

function natural(cursor) {
  const first = take(cursor, 1)[0];
  if (first < 128) return first;
  let length = 0;
  while (length < 8 && (first & (128 >>> length)) !== 0) length += 1;
  if (length === 0 || length > 7) throw new Error("invalid JAM natural");
  const low = take(cursor, length);
  let multiplier = 1;
  let value = 0;
  for (const byte of low) {
    value += byte * multiplier;
    multiplier *= 256;
  }
  return value + (first & (127 >>> length)) * multiplier;
}

function take(cursor, length) {
  const end = cursor.offset + length;
  if (end > cursor.input.length) throw new Error("invalid JAM bytes");
  const value = cursor.input.slice(cursor.offset, end);
  cursor.offset = end;
  return value;
}

async function loadGeneratedDecoder(source, name) {
  const line = source.split("\n").find((candidate) => candidate.startsWith(`function decode_${name}_input(`));
  assert.ok(line, `generated decoder ${name} is present`);
  const helpers = `
    function jamTake(cursor, length) { const end = cursor.offset + length; if (length < 0 || end < cursor.offset || end > cursor.input.length) throw new Error("invalid JAM bytes"); const value = cursor.input.slice(cursor.offset, end); cursor.offset = end; return value; }
    function jamU8(cursor) { return jamTake(cursor, 1)[0]; }
    function jamU32(cursor) { const b = jamTake(cursor, 4); return b[0] + b[1] * 256 + b[2] * 65536 + b[3] * 16777216; }
    function jamNatural(cursor) { const first = jamU8(cursor); if (first < 128) return first; let length = 0; while (length < 8 && (first & (128 >>> length)) !== 0) length += 1; if (length === 0 || length > 7) throw new Error("invalid JAM natural"); const low = jamTake(cursor, length); let multiplier = 1; let value = 0; for (let index = 0; index < length; index += 1) { value += low[index] * multiplier; multiplier *= 256; } return value + (first & (127 >>> length)) * multiplier; }
    function jamDecodeU64(input, offset) { const b = input.slice(offset, offset + 8); return { w0: b[0] + b[1] * 256 + b[2] * 65536 + b[3] * 16777216, w1: b[4] + b[5] * 256 + b[6] * 65536 + b[7] * 16777216 }; }
    function jamDecodeU128(input, offset) { const b = input.slice(offset, offset + 16); return { w0: b[0] + b[1] * 256 + b[2] * 65536 + b[3] * 16777216, w1: b[4] + b[5] * 256 + b[6] * 65536 + b[7] * 16777216, w2: b[8] + b[9] * 256 + b[10] * 65536 + b[11] * 16777216, w3: b[12] + b[13] * 256 + b[14] * 65536 + b[15] * 16777216 }; }
    function jamU64(cursor) { const value = jamDecodeU64(cursor.input, cursor.offset); cursor.offset += 8; return value; }
    function jamU128(cursor) { const value = jamDecodeU128(cursor.input, cursor.offset); cursor.offset += 16; return value; }
  `;
  const javascript = ts.transpileModule(`${helpers}\n${line}\nthis.decoder = decode_${name}_input;`, {
    compilerOptions: { target: ts.ScriptTarget.ES2020 },
  }).outputText;
  const sandbox = {};
  new vm.Script(javascript).runInNewContext(sandbox);
  return sandbox.decoder;
}

function littleEndian(value, width) {
  const output = new Uint8Array(width);
  let remaining = BigInt(value);
  for (let index = 0; index < width; index += 1) {
    output[index] = Number(remaining & 255n);
    remaining >>= 8n;
  }
  return output;
}

function numberFromLittleEndian(bytes) {
  let value = 0n;
  for (let index = bytes.length - 1; index >= 0; index -= 1) value = (value << 8n) | BigInt(bytes[index]);
  return value <= BigInt(Number.MAX_SAFE_INTEGER) ? Number(value) : value;
}

function words(bytes) {
  const output = {};
  for (let index = 0; index < bytes.length / 4; index += 1) {
    output[`w${index}`] = numberFromLittleEndian(bytes.slice(index * 4, index * 4 + 4));
  }
  return output;
}

function concat(parts) {
  const output = new Uint8Array(parts.reduce((size, part) => size + part.length, 0));
  let offset = 0;
  for (const part of parts) {
    output.set(part, offset);
    offset += part.length;
  }
  return output;
}

function normalize(value) {
  if (value instanceof Uint8Array || ArrayBuffer.isView(value)) return Array.from(value);
  if (Array.isArray(value)) return value.map(normalize);
  if (value !== null && typeof value === "object") {
    return Object.fromEntries(Object.entries(value).map(([key, item]) => [key, normalize(item)]));
  }
  return value;
}
