# String Implementation (`std.string_impl`)

`library/std/string_impl/` holds internal implementations re-exported under
`std.string` in `library/std/lib.fl`, avoiding filename/module-directory
collisions with the public `pub mod string` namespace.

- `string.fl`: `Utf8Err`, RFC 3629 UTF-8 validation, `String<.A: Type = DefaultAllocator, quote .allocator: lift *A = DefaultAllocator.GLOBAL.ref>` (zero storage overhead via `Vec<u8, .A, .allocator>`, allocator-polymorphic `equals`/`cmp_ref`/`push_string`/`eq`), `Display`, `Eq`, `Ord`, and `Hash`.
