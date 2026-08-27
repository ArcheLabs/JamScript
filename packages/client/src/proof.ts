import {
  trieNodeDec,
  validateProofs,
  type ProofTrieNode,
} from "@polkadot-api/substrate-bindings";

type TrieNode = ProofTrieNode | ReturnType<typeof trieNodeDec>;

export function verifyManagedStateProof(
  stateRoot: Uint8Array,
  key: Uint8Array,
  claimedValue: Uint8Array | null,
  proof: Uint8Array[],
): Uint8Array | null {
  if (stateRoot.length !== 32) throw new Error("managed-state root must be 32 bytes");
  if (proof.length === 0) throw new Error("managed-state proof is empty");

  const validated = validateProofs(proof);
  if (validated === null || normalizeHex(validated.rootHash) !== bytesToHex(stateRoot)) {
    throw new Error("managed-state proof does not match the requested root");
  }

  const keyNibbles = bytesToHex(key).slice(2);
  let offset = 0;
  let node: TrieNode = requireProofNode(validated.proofs, validated.rootHash);

  for (;;) {
    if (node.type === "Raw" || node.type === "Empty" || node.type === "Reserved") {
      if (node.type === "Empty") return assertClaimedValue(null, claimedValue);
      throw new Error("invalid managed-state proof node");
    }

    if (!keyNibbles.startsWith(node.partialKey, offset)) {
      return assertClaimedValue(null, claimedValue);
    }
    offset += node.partialKey.length;

    if (node.type === "Leaf" || node.type === "LeafWithHash") {
      const value = offset === keyNibbles.length
        ? resolveValue(node.type, node.value, validated.proofs)
        : null;
      return assertClaimedValue(value, claimedValue);
    }

    if (offset === keyNibbles.length) {
      const value = node.type === "BranchWithVal"
        ? hexToBytes(node.value)
        : node.type === "BranchWithHash"
          ? resolveHashedValue(node.value, validated.proofs)
          : null;
      return assertClaimedValue(value, claimedValue);
    }

    if (!("children" in node)) throw new Error("invalid managed-state branch proof");
    const child = node.children[keyNibbles[offset] as keyof typeof node.children];
    offset += 1;
    if (child === undefined) return assertClaimedValue(null, claimedValue);
    const childBytes = hexToBytes(child);
    node = childBytes.length === 32
      ? requireProofNode(validated.proofs, child)
      : trieNodeDec(childBytes);
  }
}

function resolveValue(
  type: "Leaf" | "LeafWithHash",
  value: string,
  proofs: Record<string, ProofTrieNode>,
): Uint8Array {
  return type === "Leaf" ? decodeCompactBytes(hexToBytes(value)) : resolveHashedValue(value, proofs);
}

function resolveHashedValue(valueHash: string, proofs: Record<string, ProofTrieNode>): Uint8Array {
  const node = requireProofNode(proofs, valueHash);
  if (node.type !== "Raw") throw new Error("managed-state proof is missing a hashed value");
  return hexToBytes(node.value);
}

function requireProofNode(
  proofs: Record<string, ProofTrieNode>,
  hash: string,
): ProofTrieNode {
  const node = proofs[normalizeHex(hash)];
  if (node === undefined) throw new Error("managed-state proof is incomplete");
  return node;
}

function assertClaimedValue(
  verified: Uint8Array | null,
  claimed: Uint8Array | null,
): Uint8Array | null {
  if (verified === null || claimed === null) {
    if (verified !== claimed) throw new Error("managed-state value does not match its proof");
    return verified;
  }
  if (verified.length !== claimed.length || verified.some((byte, index) => byte !== claimed[index])) {
    throw new Error("managed-state value does not match its proof");
  }
  return verified;
}

function normalizeHex(value: string): string {
  return "0x" + value.toLowerCase().replace(/^0x/, "");
}

function bytesToHex(value: Uint8Array): string {
  return "0x" + Array.from(value, (byte) => byte.toString(16).padStart(2, "0")).join("");
}

function hexToBytes(value: string): Uint8Array {
  const hex = normalizeHex(value).slice(2);
  if (hex.length % 2 !== 0 || !/^[0-9a-f]*$/.test(hex)) throw new Error("invalid proof hex");
  return Uint8Array.from(hex.match(/.{2}/g)?.map((byte) => Number.parseInt(byte, 16)) ?? []);
}

function decodeCompactBytes(encoded: Uint8Array): Uint8Array {
  if (encoded.length === 0) throw new Error("invalid compact proof value");
  const mode = encoded[0] & 3;
  let length: number;
  let prefixLength: number;
  if (mode === 0) {
    length = encoded[0] >>> 2;
    prefixLength = 1;
  } else if (mode === 1) {
    if (encoded.length < 2) throw new Error("invalid compact proof value");
    length = ((encoded[0] | (encoded[1] << 8)) >>> 2);
    prefixLength = 2;
  } else if (mode === 2) {
    if (encoded.length < 4) throw new Error("invalid compact proof value");
    length = new DataView(encoded.buffer, encoded.byteOffset, 4).getUint32(0, true) >>> 2;
    prefixLength = 4;
  } else {
    const lengthBytes = (encoded[0] >>> 2) + 4;
    if (lengthBytes > 6 || encoded.length < 1 + lengthBytes) {
      throw new Error("compact proof value is too large");
    }
    length = 0;
    for (let index = 0; index < lengthBytes; index += 1) {
      length += encoded[1 + index] * 2 ** (8 * index);
    }
    prefixLength = 1 + lengthBytes;
  }
  if (!Number.isSafeInteger(length) || prefixLength + length !== encoded.length) {
    throw new Error("invalid compact proof value length");
  }
  return encoded.slice(prefixLength);
}
