# JamScript canonical codec

JamScript does not define an independent binary codec. `TypeIr` values use the
canonical rules of Jambda's `jam-codec 0.1.1`.

Fixed-width primitives are little-endian. Boolean values are exactly `00` or
`01`. `bytes(N)` and `string(N)` are encoded as `Compact(actual byte length)`
followed by raw bytes; `N` is a validation-only upper bound. UTF-8 byte length,
not JavaScript character count, is used. `fixedBytes(N)`, addresses, fixed
arrays, tuples, and records have no length prefix. Composite tags are one byte.

Every decoder must consume the complete value. Trailing bytes, invalid UTF-8,
unknown enum/result/option tags, malformed compact values, and bound violations
are errors. Golden vectors live under `test-vectors/abi-codec/`.
