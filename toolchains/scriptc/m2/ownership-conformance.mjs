import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import vm from "node:vm";
import ts from "typescript5/lib/typescript.js";

const vectors = await loadVectors();
const source = await readFile(new URL("./runtime.ts", import.meta.url), "utf8");
const javascript = ts.transpileModule(source, {
  compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2020 },
}).outputText;
const module = { exports: {} };
vm.runInNewContext(javascript, {
  Buffer,
  TextEncoder,
  Uint8Array,
  Math,
  module,
  exports: module.exports,
}, { filename: "runtime.ts" });

function bytes(hex) {
  return Uint8Array.from(hex.slice(2).match(/../g) ?? [], (pair) => Number.parseInt(pair, 16));
}

function hex(value) {
  return `0x${Buffer.from(value).toString("hex")}`;
}

for (const vector of vectors) {
  const value = { version: 1, kind: vector.kind, public: bytes(vector.public) };
  assert.equal(hex(module.exports.encodeOwnership(value)), vector.canonical, vector.name);
  assert.equal(hex(module.exports.ownershipKey(value)), vector.key, vector.name);
  const cursor = { input: bytes(vector.canonical), offset: 0 };
  const decoded = module.exports.decodeOwnershipAt(cursor);
  assert.equal(decoded.version, value.version, vector.name);
  assert.equal(decoded.kind, value.kind, vector.name);
  assert.deepEqual(Array.from(decoded.public), Array.from(value.public), vector.name);
  assert.equal(cursor.offset, bytes(vector.canonical).length, vector.name);
}
console.log("JAMSCRIPT_OWNERSHIP_KEY_SCRIPTC_PARITY=PASS");

async function loadVectors() {
  try {
    return JSON.parse(await readFile(new URL("../../../test-vectors/ownership-v1.json", import.meta.url))).vectors;
  } catch (error) {
    if (error?.code !== "ENOENT") throw error;
    return [
      [0, "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f", "01002000000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f", "76ded8286e67ea442e884ab7d79d5ba42534f5368b866430199de51fd4cbad19"],
      [1, "202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f", "01012000202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f", "8b2dba2c0a2016616f8178d4100808009d2d2e3fd962a9d67c9c48d8ccfb0e81"],
      [2, "02404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f", "0102210002404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f", "eab5ba0f8f4f88daed79dffe70d1a652799fb689528a377373e3ed5d76b391b4"],
      [3, "606162636465666768696a6b6c6d6e6f70717273", "01031400606162636465666768696a6b6c6d6e6f70717273", "6c0e402a3030ea26b717d56db0bbe8d25c4b70dcaceece84a2860f9e6e1c59ec"],
      [4, "7475767778797a7b7c7d7e7f808182838485868788898a8b8c8d8e8f90919293", "010420007475767778797a7b7c7d7e7f808182838485868788898a8b8c8d8e8f90919293", "c86d314201f5257882179547191990558e378743decf6d2c478ad53c3a1c6522"],
    ].map(([kind, pub, canonical, key], index) => ({ name: `vector-${index}`, kind, public: `0x${pub}`, canonical: `0x${canonical}`, key: `0x${key}` }));
  }
}
