# JamScript release conformance

This release uses one ABI type graph (`TypeIr`) for action inputs, managed
state keys and values, queries, generated ABI descriptors, and client
validation. JamScript does not define an independent binary codec. ABI values
use the canonical encoding rules implemented by Jambda's `jam-codec`.

## Sources of truth

- Jambda revision: `d33e0abf8116b23bbc551c6a8d7075eacb2994ce`
- `jam-codec`: `0.1.1`
- JAM Gray Paper serialization semantics: `0.7.2`
- MiniJAM client revision: `18de55e175abb1cb40679be2e538644e2387655f`
- ABI language: `0.2`

Fixed-width integers are little-endian. Booleans accept only `00` and `01`.
`bytes(N)` and `string(N)` encode the actual byte length using JAM general
natural encoding followed by the bytes; `N` is validation-only. Fixed bytes,
addresses, fixed arrays, tuples and records have no length/name metadata on the
wire. Options, results and enums use their one-byte variant tag.

The shared vectors under `test-vectors/abi-codec/` are normative ABI
conformance vectors and are consumed by both the Rust codec and TypeScript
client test suites. Decoders must consume the complete value and reject
malformed UTF-8, invalid tags, out-of-bound values, and trailing bytes.

## Compatibility identity

The ABI descriptor is versioned and deterministic. Changing a type, record
field order, enum index, action selector, or state schema changes the
application compatibility identity. A schema change is a state migration; the
runtime never silently rewrites existing managed state.

## Verification

The Rust `jamscript-codec` crate cross-checks JAM natural encoding against
`jam-codec 0.1.1`. The TypeScript client accepts both generated descriptors and
canonical rules.
The release gate additionally requires the workspace tests and client tests to
pass.

## Current conformance status

- ABI foundation conformance: PASS after this hardening round.
- Runtime execution conformance: defined by the Formal V1 dynamic-state
  ScriptC fixture.
- Live MiniJAM conformance: requires an operator-controlled live node.

JamScript remains pre-stable. Development protocol generations before Formal V1
are not compatibility contracts; Formal V1 is the first supported wire/runtime
protocol. Language `0.2` is the sole supported source language version.

JamScript uses repository-local generic fixtures, such as `examples/typed-state`,
for compiler and runtime conformance. Real applications live in downstream
application repositories. The canonical JNS implementation is maintained by
ArcheLabs/jam-os and is not a JamScript release dependency.
