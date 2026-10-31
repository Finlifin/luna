# Checked comptime Object slot transfers

Comptime `*meta.Object` loads and whole-value writes now support immutable
metadata snapshots. The old VM branch rejected the Object domain before
inspecting its contents. The new path is confined to comptime Object storage;
it does not add a global Copy implementation or a runtime Object layout.

Before reading, immutable admission checks the current value, then every
retained entry domain is checked for transfer permission. Type descriptions
are comptime data; nested Objects are checked recursively. Optional, error
and list domains check every declared payload, including inactive variants
and empty lists. Other domains use the existing budgeted pointer-Copy proof,
so nominal payloads must have Copy without Drop. Active pointers, storage and
callables fail immutable admission rather than becoming hidden shared state.

Before writing, the same checks apply to both the incoming snapshot and the
old slot value. Only then is the ordinary storage write committed. Validation,
permission or budget failure leaves the slot intact at its post-operand state;
the VM can retry after a budget failure. This does not roll back explicit
operand side effects or an entire invocation. Returned and saved snapshots remain
independent. The operation retains entry order, duplicate keys and Type heads.

This is not move/Drop support for arbitrary Object payloads. Objects containing
such resources still require a separate transfer model; their representation
alone cannot justify copying or silently discarding the old value. The shared
validator now also covers List pointer loads, replacement and fused append.
The initial Object-only delivery left a bypass: List<Item> could duplicate a
nominal payload without Copy. A source rejection test reproduced that behavior
before the List paths were connected. List<Object> cannot hide such entries,
and an empty list still checks its declared element domain.

The fused append validates the existing list and suffix permissions before
mutation. Its final combined representation/budget check retains the existing
truncate-on-failure rollback. Ordinary List value operations are not redefined
by this pointer-transfer fix.

## Validation

Working directory: `/home/fin/luna`. All commands exited 0.

- `python3 -B development/check.py --timeout 900 -- test --release -p luna --test comptime_object_storage --test comptime_memory --test library_meta_objects -- --nocapture`:
  41 tests passed; `/tmp/luna-tools-bvymzqyb/result.json`. The new group checks
  nested snapshots, Type heads, key-evaluation side effects, nominal Copy,
  Drop rejection, old/new payload checks and inactive resource domains.
- `python3 -B development/check.py --timeout 900 -- test --release -p comptime_interpretor --lib object_storage`:
  3 tests passed; `/tmp/luna-tools-qaqrlvbw/result.json`. Sweeps budgets 1–99,
  checks no mutation on failure and successful retry, rejects malformed old
  and new data and active pointers, and retains the runtime phase boundary.
- `python3 -B development/check.py --timeout 900 -- test --release -p comptime_interpretor --lib pointer_copy_budget_tests`:
  2 tests passed; `/tmp/luna-tools-bkpiqzcb/result.json`. Existing Copy proof
  fuel accounting and ordinary pointer transfer recovery remain intact.
- `python3 -B development/library_check.py --release --timeout 900 --stage sysroot-meta`:
  one native test passed; `/tmp/luna-library-9tky9aj1/summary.json`. It includes
  a public `*meta.Object` helper that replaces a named value while preserving
  the caller's saved snapshot.
- `python3 -B development/command.py --timeout 120 -- cargo fmt --all -- --check`:
  `/tmp/luna-tools-t_tud8mv/result.json`; `git diff --check` also passed.

## List transfer follow-up

The shared implementation now resides in `immutable_storage.rs`. The original
commands above record the Object-only stage; the current VM unit group is
`immutable_storage`.

Before the List fix, the new source rejection test returned `Bool(true)` for
reading a `List<Item>` pointer without a Copy implementation. The expected
rejection failed in `/tmp/luna-tools-ono4mm47/result.json` (exit 101). The final
regression covers loads, replacement, fused append, nested Object payloads,
inactive optional domains, successful Copy snapshots and Drop rejection.

Final validation from `/home/fin/luna`, all exit 0:

- `python3 -B development/check.py --timeout 900 -- test --release -p luna --test comptime_object_storage --test comptime_list_operations --test comptime_memory -- --nocapture`:
  54 passed; `/tmp/luna-tools-oogsqj1w/result.json`.
- `python3 -B development/check.py --timeout 900 -- test --release -p comptime_interpretor --lib immutable_storage`:
  4 passed; `/tmp/luna-tools-15ofzqrp/result.json`. The new fused-append test
  sweeps budgets 1–199, verifies unchanged contents on failure and retries
  with sufficient fuel. It calls the fused path directly rather than
  accidentally verifying only its fallback.
- `python3 -B development/library_check.py --release --timeout 900 --stage sysroot-meta --stage sysroot-builders --stage sysroot-enum-builder`:
  5 native tests passed; `/tmp/luna-library-nmlly4vp/summary.json`.
- `python3 -B development/command.py --timeout 120 -- cargo fmt --all -- --check`:
  `/tmp/luna-tools-g_k3pmaz/result.json`; `git diff --check` also passed.
