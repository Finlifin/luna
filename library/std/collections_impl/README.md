# Collections Implementation (`std.collections_impl`)

`library/std/collections_impl/` holds internal implementations re-exported under
`std.collections` in `library/std/lib.fl`, avoiding filename/module-directory
collisions with the public `pub mod collections` namespace.

- `vec.fl`: `Vec<T>`, `VecIter<T>`, `Drop`, `IntoIterator`/`Iterator`, `Eq`, and
  `ByteWriter for Vec<u8>`.
