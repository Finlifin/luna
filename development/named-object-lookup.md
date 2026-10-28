# Named Object lookup and build feature queries

`meta.object_get<T>(object, key: Symbol) -> ?T` selects the last named entry,
without merging or modifying the Object. Positional entries do not match.
Absence and a different stored domain return null; a mismatched final entry
does not fall back to a matching earlier entry. Nested optional presence and
normal-form Type identities survive extraction.

The privileged operation lowers to `(object-query (query named-value) ...)`.
Its packet evaluates Object and Symbol once in source order, retaining the
Object snapshot across key evaluation. The VM reuses `lookup_object_property`,
which validates all entries, including unselected and shadowed data. Query
failures acquire the operation's source range. Result copying uses the ordinary
representation path; no runtime representation or resource-copy permission is
introduced. NIR rewriting, copy budgets and comptime phase restrictions apply.

`build.TargetFeatures.get` and `build.MetadataFeatures.get` use this operation
instead of repeatedly transporting the complete Object into indexed queries.
Driver-provided snapshots still require unique names. Hand-constructed feature
containers now follow last-entry lookup if their entries contain duplicates;
the former library loop selected the first entry. Package ownership, provider
admission, cache keys and missing-configuration failures are unchanged.

## Work-counter comparison

The test `named_lookup_avoids_repeated_object_transport_during_feature_search`
compares the previous feature-search loop with the public named query. Both
construct the same Object with 64 unique Bool properties and return the final
property. Package elaboration/admission is outside the measured invocation;
Object construction and invocation work are included.

| Implementation | Charged VM steps |
| --- | --- |
| Previous indexed search | 88,255 |
| Named lookup | 1,824 |

This is a deterministic work-counter result, not a wall-clock benchmark. The
new path still transports the input and validates entries; it is not a
constant-time map or a general compiler speedup claim. The regression requires
a substantial reduction rather than pinning exact step totals.

## Validation

Working directory: `/home/fin/luna`. All final commands below exited 0.

- `python3 -B development/check.py --timeout 900 -- test --release -p luna --test library_meta_objects --test build_context --test comptime_object_contracts --test comptime_list_operations -- --nocapture`:
  87 tests passed; `/tmp/luna-tools-kxayn6re/result.json`. Covers duplicate keys,
  optional layers, Type identity, input order, early return, local snapshot
  replacement, invalid keys, runtime rejection, budgets and build contexts.
- `python3 -B development/check.py --timeout 900 -- test --release -p comptime_interpretor --lib object_queries`:
  3 tests passed; `/tmp/luna-tools-nq5079q1/result.json`. Includes rejection of
  an invalid unselected entry supplied directly to the VM.
- `python3 -B development/check.py --timeout 900 -- test --release -p nir --test phases --test copy_budget --test type_rewrite struct_reflection`:
  3 tests passed; `/tmp/luna-tools-u4z8upsn/result.json`. Each test now includes
  the named query among the existing reflection operations.
- `python3 -B development/library_check.py --release --timeout 900 --stage sysroot-meta --stage sysroot-build-target --stage sysroot-build-mode`:
  4 native tests passed across three stages;
  `/tmp/luna-library-c6ocbn08/summary.json`.
- `python3 -B development/command.py --timeout 120 -- cargo fmt --all -- --check`:
  `/tmp/luna-tools-9twopyae/result.json`; `git diff --check` also passed.

Initial failures were in new test fixtures: operator spacing, a nonunique
`new` entry name that selected StructBuilder's method, and whole-Object pointer
writeback, which remains unsupported. The snapshot test uses local replacement;
this change does not extend pointer writeback support. The final checks above
supersede those failed runs.
