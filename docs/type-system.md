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

Record fields and enum indices are frozen in declaration order. The compiler
calculates `maxEncodedLen` for each bounded descriptor and rejects values that
would exceed the runtime action, state-key, or state-value limits.
