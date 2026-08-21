import fs from "node:fs/promises";
import process from "node:process";
import { hexToU8a, u8aToHex } from "@polkadot/util";
import {
  blake2AsHex,
  cryptoWaitReady,
  sr25519PairFromSeed,
  sr25519Sign,
} from "@polkadot/util-crypto";

const SEED = hexToU8a("0x" + "07".repeat(32));
const MIN_ITEM_GAS = 5_000_000;
const MIN_MEMO_GAS = 1_000_000;

function required(args, name) {
  const value = args.get(name);
  if (!value) throw new Error("missing argument: " + name);
  return value;
}

function parseArgs(argv) {
  const args = new Map();
  for (let index = 0; index < argv.length; index += 2) {
    const name = argv[index];
    if (!name.startsWith("--") || argv[index + 1] === undefined) {
      throw new Error("expected --name value arguments");
    }
    args.set(name.slice(2), argv[index + 1]);
  }
  return args;
}

async function request(baseUrl, path, init = {}) {
  if (path.includes("/work")) {
    throw new Error("Playground Work endpoint is forbidden in JamScript network E2E");
  }
  const response = await fetch(baseUrl.replace(/\/$/, "") + path, {
    ...init,
    headers: { "content-type": "application/json", ...(init.headers ?? {}) },
  });
  const text = await response.text();
  let body;
  try {
    body = text ? JSON.parse(text) : null;
  } catch {
    body = text;
  }
  if (!response.ok) {
    throw new Error("Playground " + path + " returned " + response.status + ": " + text);
  }
  return body;
}

function canonicalJson(value) {
  if (Array.isArray(value)) return value.map(canonicalJson);
  if (value && typeof value === "object") {
    return Object.fromEntries(
      Object.entries(value)
        .sort(([left], [right]) => left.localeCompare(right))
        .map(([key, entry]) => [key, canonicalJson(entry)]),
    );
  }
  return value;
}

function json(value) {
  return JSON.stringify(value);
}

function hashJson(value) {
  return blake2AsHex(
    new TextEncoder().encode(json(canonicalJson(value))),
    256,
  );
}

function signedAuthorization(pair, prepared) {
  const digest = hexToU8a(prepared.signingPayload);
  const prefix = new TextEncoder().encode("<Bytes>");
  const suffix = new TextEncoder().encode("</Bytes>");
  const wrapped = new Uint8Array(prefix.length + digest.length + suffix.length);
  wrapped.set(prefix);
  wrapped.set(digest, prefix.length);
  wrapped.set(suffix, prefix.length + digest.length);
  return {
    actionId: prepared.actionId,
    signature: u8aToHex(sr25519Sign(wrapped, pair)),
  };
}

async function prepareAndAuthorize(baseUrl, pair, action, params) {
  const prepared = await request(baseUrl, "/api/v1/actions/prepare", {
    method: "POST",
    body: json({
      account: u8aToHex(pair.publicKey),
      action,
      paramsHash: hashJson(params),
      expiry: Math.floor(Date.now() / 1000) + 600,
    }),
  });
  return signedAuthorization(pair, prepared);
}

async function waitOperation(baseUrl, operationId) {
  const deadline = Date.now() + 180_000;
  for (;;) {
    const operation = await request(
      baseUrl,
      "/api/v1/operations/" + encodeURIComponent(operationId),
      { method: "GET" },
    );
    if (operation.status === "succeeded") return operation;
    if (operation.status === "failed") {
      throw new Error("provisioning operation failed: " + (operation.error ?? "unknown error"));
    }
    if (Date.now() >= deadline) {
      throw new Error("timed out waiting for provisioning operation " + operationId);
    }
    await new Promise((resolve) => setTimeout(resolve, 1000));
  }
}

async function create(baseUrl, blobPath, codeHash) {
  const blob = (await fs.readFile(blobPath)).toString("base64");
  const params = {
    blobBase64: blob,
    codeHash,
    minItemGas: MIN_ITEM_GAS,
    minMemoGas: MIN_MEMO_GAS,
  };
  const pair = sr25519PairFromSeed(SEED);
  const authorization = await prepareAndAuthorize(baseUrl, pair, "create_service", params);
  const response = await request(baseUrl, "/api/v1/services", {
    method: "POST",
    body: json({ authorization, ...params }),
  });
  const operation = await waitOperation(baseUrl, response.operationId);
  const serviceId = operation.result?.serviceId;
  if (!Number.isInteger(serviceId)) {
    throw new Error("create operation did not return serviceId");
  }
  return { serviceId, operationId: operation.operationId, controller: u8aToHex(pair.publicKey) };
}

async function upgrade(baseUrl, serviceId, blobPath, codeHash) {
  const blob = (await fs.readFile(blobPath)).toString("base64");
  const params = {
    serviceId: Number(serviceId),
    blobBase64: blob,
    codeHash,
    minItemGas: MIN_ITEM_GAS,
    minMemoGas: MIN_MEMO_GAS,
  };
  const pair = sr25519PairFromSeed(SEED);
  const authorization = await prepareAndAuthorize(baseUrl, pair, "upgrade_service", params);
  const response = await request(baseUrl, "/api/v1/services/" + serviceId + "/upgrade", {
    method: "POST",
    body: json({ authorization, ...params }),
  });
  const operation = await waitOperation(baseUrl, response.operationId);
  return { operationId: operation.operationId, controller: u8aToHex(pair.publicKey) };
}

await cryptoWaitReady();
const args = parseArgs(process.argv.slice(2));
const command = required(args, "command");
const baseUrl = required(args, "base-url");
const blobPath = required(args, "blob");
const codeHash = required(args, "code-hash");
const result =
  command === "create"
    ? await create(baseUrl, blobPath, codeHash)
    : command === "upgrade"
      ? await upgrade(baseUrl, required(args, "service-id"), blobPath, codeHash)
      : (() => {
          throw new Error("unknown provisioning command: " + command);
        })();
console.log(JSON.stringify(result));
