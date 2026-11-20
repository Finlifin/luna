# Collections Implementation (`std.collections_impl`)

`library/std/collections_impl/` holds internal implementations re-exported under
`std.collections` in `library/std/lib.fl`, avoiding filename/module-directory
collisions with the public `pub mod collections` namespace.

- `vec.fl`: `Vec<T, .A: Type = DefaultAllocator, quote .allocator: lift *A = DefaultAllocator.GLOBAL.ref>` and `VecIter<T, .A, .allocator>` using zero-overhead runtime-dependent allocator clauses (distinct allocator identity per runtime instance, `Drop` env delivery, cross-allocator `Vec.append`), `IntoIterator`/`Iterator`, `Eq`, and `ByteWriter for Vec<u8, .A = A, .allocator = alloc>`.
