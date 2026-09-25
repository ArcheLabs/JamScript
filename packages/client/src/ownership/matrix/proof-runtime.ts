/** Minimal byte-only proof decoder shared with deterministic service adapters. */
export type MatrixControlClaimProofBytesV1 = {
  userId: Uint8Array;
  selfSigningPublicKey: Uint8Array;
  masterSignature: Uint8Array;
  deviceId: Uint8Array;
  algorithms: Uint8Array;
  algorithmsLength: number;
  deviceCurve25519Key: Uint8Array;
  deviceEd25519Key: Uint8Array;
  selfSigningSignature: Uint8Array;
};

type MatrixProofReader = { bytes: Uint8Array; offset: number };

function take(reader: MatrixProofReader, length: number): Uint8Array {
  const end = reader.offset + length;
  if (end > reader.bytes.length) throw new Error("truncated Matrix proof");
  const value = reader.bytes.slice(reader.offset, end);
  reader.offset = end;
  return value;
}

function readTextBytes(reader: MatrixProofReader, limit: number, label: string): Uint8Array {
  const prefix = take(reader, 2);
  const length = prefix[0] | (prefix[1] << 8);
  if (length > limit) throw new Error(`invalid Matrix ${label}`);
  const value = take(reader, length);
  for (let index = 0; index < value.length; index += 1) {
    const byte = value[index];
    if (byte < 0x20 || byte > 0x7e || byte === 0x22 || byte === 0x5c) {
      throw new Error(`invalid Matrix ${label}`);
    }
  }
  return value;
}

/** Decode bounded, ASCII-only proof fields without converting bytes to strings. */
export function decodeMatrixControlClaimProofV1Bytes(bytes: Uint8Array): MatrixControlClaimProofBytesV1 {
  const maxAlgorithms = 8;
  const maxAlgorithmBytes = 128;
  const reader: MatrixProofReader = { bytes, offset: 0 };
  if (take(reader, 1)[0] !== 1) throw new Error("unsupported Matrix proof version");
  const userId = readTextBytes(reader, 255, "user id");
  const selfSigningPublicKey = take(reader, 32);
  const masterSignature = take(reader, 64);
  const deviceId = readTextBytes(reader, 255, "device id");
  const algorithmCount = take(reader, 1)[0];
  if (algorithmCount > 8) throw new Error("too many Matrix algorithms");
  const algorithms = new Uint8Array(maxAlgorithms * (maxAlgorithmBytes + 3) - 1);
  let algorithmsLength = 0;
  for (let index = 0; index < algorithmCount; index += 1) {
    const algorithm = readTextBytes(reader, 128, "algorithm");
    if (index > 0) algorithms[algorithmsLength++] = 44;
    algorithms[algorithmsLength++] = 34;
    for (let byteIndex = 0; byteIndex < algorithm.length; byteIndex += 1) {
      algorithms[algorithmsLength++] = algorithm[byteIndex];
    }
    algorithms[algorithmsLength++] = 34;
  }
  const deviceCurve25519Key = take(reader, 32);
  const deviceEd25519Key = take(reader, 32);
  const selfSigningSignature = take(reader, 64);
  if (reader.offset !== bytes.length) throw new Error("trailing Matrix proof bytes");
  return {
    userId,
    selfSigningPublicKey,
    masterSignature,
    deviceId,
    algorithms,
    algorithmsLength,
    deviceCurve25519Key,
    deviceEd25519Key,
    selfSigningSignature,
  };
}
