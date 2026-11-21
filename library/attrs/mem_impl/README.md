# `attrs.mem_impl`

Internal constructors for memory and MMIO access control attributes:

- `attrs.mem.volatile` (also re-exported as `attrs.volatile`) produces
  `{ __flurry_volatile: true }`.
- Bare `__flurry_volatile` keys and definition configurations (`.volatile true`)
  are forbidden; callers must attach `^attrs.mem.volatile` or `^attrs.volatile`.
- Valid targets are struct definitions, newtype definitions, and struct fields
  (incompatible with `packed` structs).
