// This source intentionally exercises the native BigInt surface.  The probe
// is not part of the JamScript runtime; it records whether every ScriptC
// stage can carry the value before the production compiler chooses a backend.
export function execute(): number {
  const small = 123n;
  const wide = 18446744073709551616n;
  const max = 340282366920938463463374607431768211455n;
  const sum = small + wide;
  const product = 3n * 7n;
  return sum < max && product === 21n ? 1 : 0;
}
