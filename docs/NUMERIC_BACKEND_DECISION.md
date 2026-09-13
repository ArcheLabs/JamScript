# Numeric backend decision

The ScriptC 0.0.34 capability probe is checked in under
`toolchains/scriptc/m2/numeric-probe/`. It reports that the pinned TypeScript
front end accepts and type-checks BigInt, but ScriptC lowering rejects
`BigIntLiteral` with `SC1090`. C emission, PolkaVM linking, and PVM execution
therefore cannot be treated as supported native-BigInt stages.

JamScript 0.3 uses the fixed-limb backend for executable unsigned integers:

- `u8/u16/u32` remain exact checked numeric values;
- `u64` is represented privately as two 32-bit limbs;
- `u128` is represented privately as four 32-bit limbs;
- multiplication uses 16-bit sub-limbs, and division/modulo use fixed-width
  binary long division;
- user source and the public ABI remain `u64`/`u128`.

This keeps arbitrary values out of JavaScript `number`, preserves the existing
JAM little-endian wire format, and gives overflow, underflow, division-by-zero,
and checked-cast failures deterministic fatal codes. Native BigInt may only be
selected after all six probe stages pass on the pinned toolchain.
