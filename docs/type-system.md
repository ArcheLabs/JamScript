# JamScript type system

Only bounded values may cross the Action, State, Query, or client boundary.
The compiler represents every boundary type as one `TypeIr` descriptor:

- `unit`, `bool`, `u8/u16/u32/u64/u128`
- `i8/i16/i32/i64/i128`
- `address`, `fixedBytes(N)`, `bytes(N)`, `string(N)`
- `fixedArray(T,N)`, `array(T,N)`, `option(T)`, `tuple(...)`, `record({...})`
- bounded `enumType({...})` and `result(T, E)`

Unsigned and signed integers have declared widths. JavaScript `number`,
unbounded arrays/strings, `any`, `unknown`, objects without a descriptor, and
ambient or nondeterministic runtime values are not ABI types. `u64`, `u128`,
`i64`, and `i128` are represented as `bigint` in the browser client.

The three support boundaries are intentionally distinct:

- ABI/type support describes what `TypeIr` can represent canonically;
- execution support describes what the selected compiler/runtime can evaluate;
- client support describes the host-language representation used by callers.

In language 0.3, executable unsigned arithmetic is checked for every fixed
width. ScriptC lowers `u64` and `u128` to private fixed-limb values; application
code still writes `u64` and `u128`, and the client still receives them as
JavaScript `bigint`. A type existing in `TypeIr` is not, by itself, proof that
every execution backend supports it.

Record fields and enum indices are frozen in declaration order. The compiler
calculates `maxEncodedLen` for each bounded descriptor and rejects values that
would exceed the runtime action, state-key, or state-value limits.
