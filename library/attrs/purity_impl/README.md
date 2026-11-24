# `attrs.purity_impl`

Internal constants for runtime purity tier attributes (`__flurry_purity: Symbol`):

- `attrs.purity.p0` .. `attrs.purity.p5` produce `{ __flurry_purity: .p0 }` ..
  `{ __flurry_purity: .p5 }`.
- Bare `__flurry_purity` keys and definition configurations (`.__flurry_purity .p4`)
  are forbidden; callers must attach `^attrs.purity.{p0, p1, p2, p3, p4, p5}`.
- Valid targets are runtime function definitions and declarations; the compiler
  verifies the declared minimum purity tier at compile time.
