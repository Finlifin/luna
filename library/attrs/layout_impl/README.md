# attrs.packed

- `attrs.packed.byte` produces `{ __flurry_packed_mode: .byte }` for byte-packed structs (`align = 1`, `size = sum(field.size)`, arbitrary runtime fields allowed, aligned field pointers forbidden).
- `attrs.packed.bit` produces `{ __flurry_packed_mode: .bit }` for bit-packed POD structs lowered directly to `BitVec<W>` sharing `bitvec_opt` (emits a non-fatal compiler warning when total bit width exceeds 256 bits).
