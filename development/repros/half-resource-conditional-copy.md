# Conditional Copy and error wrappers

The finite-disjunction eligibility fix supports the shared half-precision
`Clone` implementation. `Copy` currently remains two concrete declarations.

To reproduce the remaining integration failure, replace those two declarations
in `library/builtin/protocols/native_resources.fl` with:

```flurry
impl Copy for T where T, requires T == f16 or T == bf16 {}
```

Run from the repository root:

```sh
python3 -B development/check.py --timeout 900 -- test --release -p luna --test frontend_cli sysroot_half_float
```

The shared half-precision Clone body passes, but formation of the existing
`impl Clone for !E T` body rejects `self.*` because Copy evidence is not
established. The isolated
`native_wrapper_copy_requires_every_payload_and_preserves_pointer_lifetime`
test in `library_protocols` reproduces the same failure.

A marker-only conditional trait alongside an error-wrapper implementation does
not reproduce it. The remaining cause must be investigated with the actual
Copy/Clone contracts and pointer-read formation; it is not established that
error-wrapper candidate selection alone is responsible.
