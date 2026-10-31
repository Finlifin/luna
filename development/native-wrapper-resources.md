# Native wrapper resource protocols

`library/builtin/protocols/native_resources.fl` adds conditional `Clone` and `Copy` for
native optional and single-error result types. The builtin module loader loads
these implementations alongside the resource trait definitions; users need no
special import beyond the normal builtin prelude.

- `?T` implements both traits when `T :- Copy`.
- `!E T` implements both when `E :- Copy` and `T :- Copy`. Both possible payload
  types matter, even when a particular expression constructs only success.
- Clone copies the complete wrapper representation without invoking a payload
  clone callback. Nested optional layers and the active error branch survive.
- Pointer payload copies retain their target dependencies. Copy does not extend
  the lifetime of local storage, and a copied wrapper cannot escape its target.

These implementations complement the unconditional Once implementations; they
do not grant Copy to affine payloads. General reconstruction from Clone-only
payloads and heterogeneous error rows remain unfinished. The separate module
also keeps scalar protocol fixtures independent of generic wrapper operations,
as with the existing tuple resource implementations.

The compiler now permits pointer-read formation when the configured Copy
protocol can be proved for the complete type, even before its generic layout is
known. This uses the existing type-requirement discharge path. It does not
manufacture structural Copy evidence: instantiated resource verification still
checks Copy/Drop legality, supported memory representation and live origins.

Validation, working directory `/home/fin/luna` unless stated otherwise:

- `python3 -B development/check.py --timeout 900 -- test --release -p luna --test library_protocols`:
  26 passed; `/tmp/luna-tools-jt0tihy9/result.json`. Rejections include affine
  optional payloads, affine success/error payloads (including an inactive
  error branch), nested affine error payloads, and copied local-pointer escape.
- `python3 -B development/library_check.py --release --timeout 900 --stage sysroot-resources`:
  three native tests passed; `/tmp/luna-library-ek4r84eo/summary.json`.
- `python3 -B development/reproduce.py --timeout 120 --binary /tmp/luna-dev-002/target/release/luna -- development/examples/sysroot-native-wrapper-copy.fl --sysroot library --test-native --opt-level 3`:
  three passed; `/tmp/luna-tools-7hzyyy2b/result.json`.
- `python3 -B development/reproduce.py --timeout 120 --binary /tmp/luna-dev-002/target/release/luna -- development/examples/sysroot-iteration.fl --sysroot library --test-native --inline-functions --fuel 1000000`:
  six passed with the new implementations loaded;
  `/tmp/luna-tools-0y6br1q2/result.json`.
- From `development`, `python3 -B -m unittest test_library_check`: 13 passed.
  The new sysroot stage requires exactly three tests with no failures.
- `python3 -B development/command.py --timeout 120 -- cargo fmt --all`:
  passed; `/tmp/luna-tools-j5my97p5/result.json`. `git diff --check` passed.
