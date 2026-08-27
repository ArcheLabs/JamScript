export function firstByte(value: Uint8Array): number {
  return value.length === 0 ? 0 : value[0];
}
