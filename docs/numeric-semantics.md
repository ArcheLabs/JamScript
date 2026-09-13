# JamScript 0.3 numeric semantics

JamScript has one fixed-width numeric model. ABI support and executable support
are separate: `TypeIr` and the Rust/browser codecs already support the
canonical `u64` and `u128` values, while ScriptC 0.3 lowers them to private
fixed limbs before C/PolkaVM compilation.

Unsigned arithmetic is checked for `u8`, `u16`, `u32`, `u64`, and `u128`.
`+`, `-`, `*`, `/`, `%`, comparisons, and `+=`, `-=`, `*=` do not silently wrap.
Division and modulo by zero are fatal. Fixed-width operands must have the same
type; use an explicit checked `toU8`, `toU16`, `toU32`, `toU64`, or `toU128`
conversion for a type change. This rule also applies to assignment, returns,
state/helper arguments, and record fields: a runtime `number` is never silently
accepted as a fixed-width value. Integer literals are contextual and are checked
from their source text, so values above `2^53` never pass through a JS number.
TypeScript `as` assertions are not numeric casts; they are rejected for a
fixed-width source or target except for a same-type no-op assertion.

The runtime fatal codes are:

| Code | Meaning |
| --- | --- |
| `0x80000001` | uncaught runtime failure |
| `0x80000002` | invalid state view |
| `0x80000003` | numeric overflow |
| `0x80000004` | numeric underflow |
| `0x80000005` | division by zero |
| `0x80000006` | invalid numeric cast/value |

The language version is `0.3`; ABI version remains `1`, and state-view version
remains `1`. Canonical `u64` and `u128` encodings remain 8-byte and 16-byte
little-endian values. Browser clients continue to expose wide integers as
JavaScript `bigint` and JSON-facing values should use decimal strings.
