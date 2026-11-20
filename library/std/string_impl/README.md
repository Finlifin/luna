# String Implementation (`std.string_impl`)

`library/std/string_impl/` holds internal implementations re-exported under
`std.string` in `library/std/lib.fl`, avoiding filename/module-directory
collisions with the public `pub mod string` namespace.

- `string.fl`: `Utf8Error`, RFC 3629 UTF-8 validation, `String`, `Display`,
  `Eq`, `Ord`, and `Hash`.
