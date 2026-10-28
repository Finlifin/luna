# Package metadata wire v1

`--packages-v1` accepts one `--package-metadata-v1 HEX` argument. Resident
`--packages-stream-v1` requests carry the same argument using the existing
hex-argument frame encoding. This is an explicit immutable input, not a path,
source expression, environment lookup or instruction to execute a factory.
The standalone implicit-sysroot CLI does not yet accept this option.

Rust producers use `luna::build_context::metadata::wire::encode_hex(&inputs)`.
`decode_hex` returns typed `Input` records, retaining declaration identities and
opaque normal-form heads. Package compilation validates providers against its
actual source graph and installs the snapshot. Missing flags mean unavailable
configuration; a valid packet with zero providers installs an empty snapshot.
Duplicate flags are rejected. Duplicate providers/fields and unknown providers
are rejected by snapshot construction. Field names must be nonempty.

```rust
use luna::build_context::metadata::{Entry, Input, wire::encode_hex};
use nir::{Primitive, Term};
let fields = vec![("enabled".into(), Entry::new(
    Term::Primitive(Primitive::Bool), Term::Bool(true), &mut 1000,
).unwrap())];
let argument = encode_hex(&[Input { identity: 300, fields }]).unwrap();
// Append ["--package-metadata-v1", &argument] to the explicit package request.
```

The value domain may instead be the provider's actual nominal enum Type; its
value carries that same Type, variant index and fields. Identities must belong
to this compilation's package set. These handles are not persistent global
package names. Encoding a descriptor does not prove that its value inhabits
its type: selected values are still admitted by the comptime VM. No Copy or
resource permission is created by transport.

## Envelope and limits

Hex accepts either case; the encoder emits lowercase. Decoded bytes are at
most 131072 bytes. Every integer below is unsigned little-endian u64 unless
specified. Lengths count bytes or elements as stated, not Unicode characters.

- Header: four bytes `46 4d 44 01` (ASCII FMD and version 1).
- Provider count, then that many provider records.
- Provider: semantic package identity, field count, then fields.
- Field: UTF-8 string name, Type term, value term.
- String: byte length followed by exactly that many UTF-8 bytes.
- Sequence: element count followed by its elements in order.
- Declaration: package identity followed by local identity (must fit u32).
- Sort: one byte, 0 Type, 1 comptime value, 2 runtime.
- Flag: one byte, exactly 0 or 1.

Trailing bytes, truncation, invalid UTF-8, overflow, unknown tags and unknown
versions are errors. Term recursion must stay below 64 levels; traversal and
collection counts share 100000 units of decoding fuel. Closed descriptor
validation has a separate shared 100000-unit budget. Numeric operations share
the comptime arithmetic budget (1000000 steps, 4096 limbs). Byte, node and
numeric limits are checked before corresponding unbounded allocation/work.
Package snapshot installation and elaboration impose their own budgets too.

## Term tags

A term starts with one byte. The table lists the remaining payload. `term?`
means a flag followed by a term only when the flag is 1. Field sequences use
(string name, term) pairs. All wrappers preserve their explicit nesting.

| Tag | Term | Payload |
|---|---|---|
| 0 | Primitive | one-byte primitive tag; tag 255 additionally carries width |
| 1 | Unit value | none |
| 2 | Bool value | flag |
| 3 | Char value | Unicode scalar as u64, checked for u32 and scalar validity |
| 4 | String value | string |
| 5 | Symbol value | string |
| 6 | Integer value | string; encoder uses signed hexadecimal magnitude |
| 7 | f32 value | IEEE bits as u64, checked to fit u32 |
| 8 | f64 value | IEEE bits as u64 |
| 9 | BitVec value | width, sequence of u32 words stored as u64, least significant first |
| 10 | Float value | lossless scientific decimal string |
| 11 | Nominal Type | declaration, argument sequence |
| 12 | NormalForm | head declaration, sort, argument sequence |
| 13 | Optional Type | term |
| 14 | Pointer Type | term |
| 15 | Internal structural tuple Type | term sequence |
| 16 | Record Type | field sequence |
| 17 | Tuple value | term sequence |
| 18 | Record value | field sequence |
| 19 | Type-list value | term sequence |
| 20 | Nominal value | Type term, variant-present flag, optional u32 variant as u64, field term sequence |
| 21 | Optional value | Type term, term? |
| 22 | Error-result Type | error-label sequence, result Type term |
| 23 | Error value | Type term, label term?, payload term |
| 24 | Comptime function Type | signature term |
| 25 | Unsafe function Type | signature term |
| 26 | Function Type | parameter sequence of (sort, Type term), result Type term |
| 27 | Bound variable | depth (u32 as u64), index, sort; must be bound by a containing Type |
| 28 | Closure Type | environment Type sequence, signature term |
| 29 | Closure-context Type | none |
| 30 | Closure-context value | Type sequence |
| 31 | Contextual closure Type | context term, signature term |
| 32 | Type-list concatenation | term sequence |

Primitive tags 0 through 24, in order:
`Type Object Bool Char Str Symbol Integer Float Unit Void Never I8 I16 I32 I64
I128 U8 U16 U32 U64 U128 Isize Usize F32 F64`. Tag 255 is BitVec(width).
Other primitive tags are reserved and rejected. BitVec words must have exactly
the required count and zero padding; decoding never truncates excess bits.
IEEE NaN payloads and signed zero are preserved. Arbitrary precision numbers
are never converted through host floating-point or fixed-width integers.

The structural tuple tag is an internal descriptor view. It does not replace
Flurry's normal-form `Tuple` identity, which is encoded with tag 12 and its
Type-list input. Object is transported using its existing typed nominal-value
entry packets; keys, original field domains and values all survive transport.

Runtime references, store handles, free variables, callable values and closure
factory snapshots are not transportable. Computation/effect-row and associated
Type descriptors currently have no wire tags and are explicitly rejected by
the encoder. This is not a general NIR serializer. Future additions require
explicit tag/version compatibility work; diagnostic Display is not a parser.

## Cache and failure behavior

Decoded inputs are part of `Request` equality. Equivalent accepted hex spellings
produce equal inputs; source graph changes and type/value/presence changes use
different keys. Validation failures never return cached results as fallback.
Resident processing continues after an invalid request; subsequent valid
requests may reuse their own previously validated matching cache entries.

Coverage: `metadata_wire`, `build_context` and `compilation_cache` integration
tests include numeric/nominal identity, truncation at every offset of a packet,
invalid versions/flags/lengths, byte/depth limits, executable-value rejection,
resident recovery and cache invalidation. Selected value admission remains
covered separately by `build_metadata` and the VM immutable-input tests.
