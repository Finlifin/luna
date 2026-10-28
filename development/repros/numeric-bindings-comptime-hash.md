# Numeric bindings validation and comptime hash failure

Worktree: `/home/fin/luna`, base `1feb3125`, branch `dev`.
Numeric modules now reuse `builtin.primitives` instead of repeating raw
builtin accesses. Public sysroot checks and isolated fixture checks follow.
All commands below ran through `python3 -B development/check.py --timeout 900 --`.

- `test --release -p luna --test frontend_cli sysroot_`: exit 0. Report: `/tmp/luna-tools-5e1mg4jh/result.json`.
- `test --release -p luna --test comparison_resources --test equality_protocol --test ordering_protocol --test library_hash --test library_comptime_comparison --test host_intrinsic_protocol`: exit 101. Report: `/tmp/luna-tools-kdedtdv0/result.json`.
- `test --release -p luna --test target_integers --test library_protocols`: exit 0. Report: `/tmp/luna-tools-u9qg283v/result.json`.
- `test --release -p luna --test ordering_protocol`: exit 0. Report: `/tmp/luna-tools-yzl9o6mw/result.json`.
- `test --release -p luna --test host native_integer_math_wrappers_cover_widths_bits_and_once_only_inputs`: exit 0. Report: `/tmp/luna-tools-404i1b9b/result.json`.
- `test --release -p luna --test host native_funnel_shifts_and_rotations_cover_modulo_width_and_argument_order`: exit 0. Report: `/tmp/luna-tools-wapair77/result.json`.
- `test --release -p luna --test host native_float_classification_generic_library_preserves_ieee_classes`: exit 0. Report: `/tmp/luna-tools-4gzqhdat/result.json`.
- `test --release -p luna --test host native_saturating_integer_math_covers_all_widths_and_signs`: exit 0. Report: `/tmp/luna-tools-es_nr0dm/result.json`.
- `test --release -p luna --test host native_integer_minmax_math_covers_all_widths_and_signs`: exit 0. Report: `/tmp/luna-tools-ic3v7e5a/result.json`.
- `test --release -p luna --test host native_wrapping_absolute_value_all_widths_and_argument_once`: exit 0. Report: `/tmp/luna-tools-86jejgph/result.json`.
- `test --release -p luna --test host native_bitwise_library_operations_preserve_width_sign_and_operand_order`: exit 0. Report: `/tmp/luna-tools-vld809hf/result.json`.

The combined protocol run passed comparison_resources (15), equality_protocol
(5), host_intrinsic_protocol (37), and library_comptime_comparison (3).
It stopped at library_hash (8 passed, 2 failed); ordering_protocol was run
separately and passed all 4 tests. library_protocols and target_integers each
passed 27 tests. The sysroot filter passed 24 tests.

## Failure reproduced before the dispatch fix

`library_hash::wide_hash_collisions_still_require_equality` and
`library_hash::text_and_symbol_cache_keys_compare_content_and_recover_after_budget_exhaustion`
fail in `Protocol::hash` with `Protocol("interpretation InvalidCallPlan")`.
Runtime and ordinary comptime scalar hash tests still pass.

For a control run, extracted `HEAD:library/builtin/numeric/comparison.fl`
from 1feb3125 to `/tmp/luna-numeric-baseline-comparison.fl`, temporarily pointed
only the library_hash fixture's comparison include at that source, and ran:

```sh
python3 -B development/check.py --timeout 900 -- test --release -p luna --test library_hash
```

The same two failures reproduced (exit 101; report
`/tmp/luna-tools-7yu_x90_/result.json`). Restored the fixture afterward; it has
no committed changes. This isolates the failures from the comparison-binding
refactor, but is not a clean full-HEAD worktree baseline: other preexisting
worktree edits were deliberately preserved.

Start investigation at `compiler/luna/src/comptime_keys.rs`: Protocol::prepare
materializes selected dispatch roots, then Protocol::invoke enters
Interpreter::invoke_instantiated. Determine which call-plan validation fails
before changing validation or increasing budgets. These failures are resolved by the dispatch fix below.

## Resolution

VM world validation failed on the parent-interface closure check. NIR
interpretation registers every unconditional concrete implementation, but
entry-root dispatch generation previously omitted their generic parent worlds
(e.g. Clone's generic Once implementation). Hashing itself was not at fault.

`materialization::build` now seeds concrete interface worlds for Dispatch and
Invocation roots as it already did for All roots. The scan and copied terms
consume the existing budget. Function body reachability remains rooted;
IIR retains its independent reachable-world policy. VM validation is unchanged.

Validation from `/home/fin/luna`:

- `python3 -B development/check.py --timeout 900 -- test --release -p luna --test library_hash`:
  11 passed, including both original failures and explicit generic-parent,
  missing-parent, and budget-failure/recovery checks.
  Report: `/tmp/luna-tools-hwr1fty7/result.json`.
- `python3 -B development/check.py --timeout 900 -- test --release -p materialization`:
  passed, including the check that unrelated function bodies remain unvisited.
  Report: `/tmp/luna-tools-n08ltkib/result.json`.
- `python3 -B development/check.py --timeout 900 -- test --release -p luna --test execution comptime_key`:
  9 passed. Report: `/tmp/luna-tools-dkkx3ivm/result.json`.
- `python3 -B development/check.py --timeout 900 -- test --release -p luna --test execution memoized_`:
  11 passed. Report: `/tmp/luna-tools-o7zarppo/result.json`.
- Formatting through `development/command.py` and `git diff --check` passed.
