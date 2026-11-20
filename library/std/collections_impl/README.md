# Collections Implementation (`std.collections_impl`)

`library/std/collections_impl/` holds internal implementations re-exported under
`std.collections` in `library/std/lib.fl`, avoiding filename/module-directory
collisions with the public `pub mod collections` namespace.

- `vec.fl`: `Vec<T, .A: Type = DefaultAllocator, quote .allocator: lift *A = DefaultAllocator.GLOBAL.ref>` and `VecIter<T, .A, .allocator>` using zero-overhead runtime-dependent allocator clauses (distinct allocator identity per runtime instance, `Drop` env delivery, cross-allocator `Vec.append`), `IntoIterator`/`Iterator`, `Eq`, and `ByteWriter for Vec<u8, .A = A, .allocator = alloc>`.
- `vec_deque.fl`: power-of-two double-ended ring buffer `VecDeque<T, .A: Type = DefaultAllocator, quote .allocator: lift *A = DefaultAllocator.GLOBAL.ref>` and `VecDequeIter<T, .A, .allocator>` supporting wrap-around `push_front`/`push_back`/`pop_front`/`pop_back`, linearized ring growth, ZST no-allocation support, affine `Drop`, `Clone` (`try_clone`), `IntoIterator`/`Iterator`, and `Eq`.
- `hash_map.fl`: single-slab SoA open-addressed `HashMap<K, V, .A: Type = DefaultAllocator, quote .allocator: lift *A = DefaultAllocator.GLOBAL.ref>` and `HashMapIter<K, V, .A, .allocator>` using SplitMix64 mixing, 75% load factor with tombstone reclamation/rehashing, `V: Once` value support, ZST support, affine `Drop`, `try_clone`, and `IntoIterator`/`Iterator`.
- `hash_set.fl`: `HashSet<T, .A: Type = DefaultAllocator, quote .allocator: lift *A = DefaultAllocator.GLOBAL.ref>` and `HashSetIter<T, .A, .allocator>` structurally wrapping `HashMap<T, void, .A, .allocator>` with cross-allocator `is_subset`/`is_disjoint`, `try_clone`, and `IntoIterator`/`Iterator`.
