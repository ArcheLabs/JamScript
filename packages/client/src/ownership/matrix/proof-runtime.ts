/** Bounded byte-only Matrix proof parser shared with deterministic service adapters. */
function readerOffset(reader: Uint8Array): number {
  return reader[0] | (reader[1] << 8) | (reader[2] << 16) | (reader[3] << 24);
}

function setReaderOffset(reader: Uint8Array, offset: number): void {
  reader[0] = offset & 0xff;
  reader[1] = (offset >>> 8) & 0xff;
  reader[2] = (offset >>> 16) & 0xff;
  reader[3] = (offset >>> 24) & 0xff;
}

function take(reader: Uint8Array, length: number): Uint8Array {
  const offset = readerOffset(reader);
  const end = offset + length;
  if (end + 4 > reader.length) throw new Error("truncated Matrix proof");
  const value = reader.slice(offset + 4, end + 4);
  setReaderOffset(reader, end);
  return value;
}

function readTextBytes(reader: Uint8Array, limit: number): Uint8Array {
  const prefix = take(reader, 2);
  const length = prefix[0] | (prefix[1] << 8);
  if (length > limit) throw new Error("invalid Matrix text length");
  const value = take(reader, length);
  for (let index = 0; index < value.length; index += 1) {
    const byte = value[index];
    if (byte < 0x20 || byte > 0x7e || byte === 0x22 || byte === 0x5c) {
      throw new Error("invalid Matrix text byte");
    }
  }
  return value;
}

/** Decode the stable wire format into a compact, numeric-offset byte view. */
export function decodeMatrixControlClaimProofV1Bytes(bytes: Uint8Array): Uint8Array {
  if (bytes.length > 2048) throw new Error("Matrix proof exceeds size limit");
  const reader = new Uint8Array(bytes.length + 4);
  reader.set(bytes, 4);
  if (take(reader, 1)[0] !== 1) throw new Error("unsupported Matrix proof version");
  const userId = readTextBytes(reader, 255);
  const selfSigningPublicKey = take(reader, 32);
  const masterSignature = take(reader, 64);
  const deviceId = readTextBytes(reader, 255);
  const algorithmCount = take(reader, 1)[0];
  if (algorithmCount > 8) throw new Error("too many Matrix algorithms");
  const algorithmBytes = new Uint8Array(8 * 131 - 1);
  let algorithmLength = 0;
  for (let index = 0; index < algorithmCount; index += 1) {
    const algorithm = readTextBytes(reader, 128);
    if (index > 0) algorithmBytes[algorithmLength++] = 44;
    algorithmBytes[algorithmLength++] = 34;
    for (let byteIndex = 0; byteIndex < algorithm.length; byteIndex += 1) {
      algorithmBytes[algorithmLength++] = algorithm[byteIndex];
    }
    algorithmBytes[algorithmLength++] = 34;
  }
  const deviceCurve25519Key = take(reader, 32);
  const deviceEd25519Key = take(reader, 32);
  const selfSigningSignature = take(reader, 64);
  if (readerOffset(reader) !== bytes.length) throw new Error("trailing Matrix proof bytes");

  const output = new Uint8Array(
    6 + userId.length + selfSigningPublicKey.length + masterSignature.length
      + deviceId.length + algorithmLength + deviceCurve25519Key.length
      + deviceEd25519Key.length + selfSigningSignature.length,
  );
  output[0] = userId.length & 0xff;
  output[1] = userId.length >>> 8;
  output[2] = deviceId.length & 0xff;
  output[3] = deviceId.length >>> 8;
  output[4] = algorithmLength & 0xff;
  output[5] = algorithmLength >>> 8;
  let offset = 6;
  output.set(userId, offset); offset += userId.length;
  output.set(selfSigningPublicKey, offset); offset += selfSigningPublicKey.length;
  output.set(masterSignature, offset); offset += masterSignature.length;
  output.set(deviceId, offset); offset += deviceId.length;
  output.set(algorithmBytes.slice(0, algorithmLength), offset); offset += algorithmLength;
  output.set(deviceCurve25519Key, offset); offset += deviceCurve25519Key.length;
  output.set(deviceEd25519Key, offset); offset += deviceEd25519Key.length;
  output.set(selfSigningSignature, offset);
  return output;
}
