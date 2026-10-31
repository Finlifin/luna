# Fallible inline iteration and error-result flow facts

`Iterator.try_fold` and `Iterator.try_foreach` are ordinary library consumers
of the inline `emit_item` protocol. A callback's `!E T` result uses the native
error-result ABI; a failed callback returns from the consumer's lexical
handler and stops upstream production. This does not add an effect operation,
a resumable iterator state, or a compiler special case for these methods.

The accumulator, item, and error need no Copy/Clone constraint. Callback entry
transfers the accumulator/item once. Failure retires the remaining source and
callback captures, while the returned error remains owned by the caller.
Success transfers the final accumulator, including a nested optional layer.

## Compiler connection

Machine-arithmetic flow previously rejected `ErrorConvert`, even for a function
that only increments a bounded counter before returning an error. The program
verification layer now observes success/error constructors, selects error
patterns by their checked type label, and tracks tags separately from payload
paths. Tags use a stable S-expression dump. This remains outside PTC.

Known payload snapshots retain scalar ranges and pointer origins. Unknown
results receive branch-qualified observations, preventing a bound on one
branch from becoming a bound on another. Copies and assignment use the same
versioned aggregate machinery as other values. Known error-row widening maps
the selected tag by payload identity and discards stale source-layout paths.

At the initial implementation stage, precision limits remained: different-tag joins do not retain a disjunction of
payload snapshots; unknown-row embeddings and product transports discard
relational precision after checking the conversion and evaluating the input.
No facts are invented for those cases. Additional checked assumes may still
be needed to prove subsequent arithmetic. This phase does not provide full
error-result relational reasoning or borrowed wrapper cloning.

## Validation (2026-10-31, /home/fin/luna)

- `python3 -B development/check.py --timeout 900 -- test -p luna --test error_result_flow`:
  six passed, exit 0, `/tmp/luna-tools-wrdnrh51/result.json`. Positive and
  rejection cases cover bounded writes before error return, payload arithmetic,
  branch separation, joins, error-row tag remapping, and stale assignments.
- `python3 -B development/check.py --timeout 900 -- test -p program_verification --no-fail-fast`:
  105 passed, exit 0, `/tmp/luna-tools-fitpy_7j/result.json`.
- `python3 -B development/check.py --timeout 900 -- test -p luna --test integer_contracts --test arithmetic_contexts --test target_integers --no-fail-fast`:
  arithmetic_contexts (10) and integer_contracts (18) passed. target_integers
  had 16 passes and 11 failures: ten stop in existing fixture HIR/name binding
  before flow analysis (missing bool bindings), one reports ExecutableFileBusy.
  Overall exit 101, `/tmp/luna-tools-l3zife7g/result.json`. These failures are
  not counted as successful regression validation.
- An earlier command accidentally included luna's entire `--lib` suite:
  70 passed and 12 host method-contract fixture tests failed on unresolved
  bool before flow analysis, `/tmp/luna-tools-x1decwnt/result.json`. The
  intended standalone error-result tests and program-verification suite were
  then run with their own explicit selectors above.
- `python3 -B -m unittest discover -s development -p test_library_check.py`:
  13 passed, exit 0. The new selectable native stage requires all seven cases
  and zero failures; an empty or truncated suite does not count as success.

The first native attempt correctly rejected an unbounded test counter; its
source now supplies the checked upper bound. The next attempt exposed the
missing error-result flow support addressed here. After that fix, all five
initial native cases passed in `/tmp/luna-library-d_f0ljus/summary.json`.
Two further cases cover successful affine accumulation and optional payloads.

The optional-accumulator case also exposed a parser boundary defect: `!E ?T`
was read as an error-row expression with postfix `?`. Qualifier rows now stop
before the next optional type prefix, as they already do before other type
qualifiers. `#effect ?T`, `&reach ?T`, nested `??T`, and lambda parameter
boundaries are covered by `parse --test closure_types` (exit 0,
`/tmp/luna-tools-0dnpnn39/result.json`).

Final native acceptance:
`python3 -B development/library_check.py --stage sysroot-fallible-iteration --timeout 300`
passed all seven tests with zero failures, exit 0;
`/tmp/luna-library-c8jzy8oy/summary.json`. This uses the real sysroot, LLVM host
pipeline, resource checking, and joint inline expansion.

Parser regression suite:
`python3 -B development/check.py --timeout 900 -- test -p parse --no-fail-fast`
exited 0; `/tmp/luna-tools-8tfai0_j/result.json`. Final formatting check
`python3 -B development/command.py --timeout 120 -- cargo fmt --all -- --check`
exited 0 (`/tmp/luna-tools-a3y2y0fd/result.json`), and `git diff --check` passed.
No full language suite was run; unrelated fixture failures above remain open.


## Transport aggregate and unknown payload observations

The next implementation stage reuses NIR's checked representation-transport
plan for every error-row and nested tuple/anonymous-record conversion.
Snapshots retain their payloads and sibling ranges; their destination domains
are updated recursively. Frozen copy evidence maps each payload path through
the same plan, including paths below an unknown active branch. Identity
subplans share immutable evidence rather than duplicating it.

A row permutation changes numeric tags. Such tag observations are omitted
rather than renamed as though their numeric values stayed unchanged. Payload
identity is retained separately. Identity tag maps may retain tag evidence.
The analysis therefore preserves checked payload bounds but still does not
infer every destination tag or keep a disjunction at different-tag joins.
No layout fact establishes source-language type equality or unfolds NF heads.

Error-result patterns are now admitted in effect-free program predicates;
recursive operand and guard checks still reject calls and side effects.
A predicate expressed as a match can establish the payload bound before
transport. Nested pattern-guard composition remains limited by the existing
predicate analyzer and can lose precision; this phase does not claim general
predicate completeness.

Validation from `/home/fin/luna`:

- `python3 -B development/check.py --timeout 900 -- test -p nir -p program_verification --no-fail-fast`:
  585 passed, exit 0; `/tmp/luna-tools-okqbi1yc/result.json`.
- `python3 -B development/check.py --timeout 900 -- test -p luna --test error_result_flow --test integer_contracts --test arithmetic_contexts --no-fail-fast`:
  39 passed, exit 0; `/tmp/luna-tools-m_jgijqk/result.json`. This includes
  unknown-row payload transport, nested products, anonymous records, stale
  source assignment, overflow rejection, and impure predicate rejection.
- `python3 -B -m unittest discover -s development -p test_library_check.py`:
  13 passed, exit 0.

Public-library limitation found by native acceptance:
`development/repros/error-row-copy-resolution.fl` contains two valid intended
uses with multiple concrete error labels. The real sysroot leaves the generic
builtin Copy head `!E T` pending for `![Low,High] bool`; this prevents resource
checking before LLVM emission. The original native attempt reports seven
passes and those two failures in `/tmp/luna-library-mzxi3_jj/summary.json`.
The reproducer is retained separately from the delivered seven-case fallible
iterator suite, and no native support for multi-label Copy is claimed. This
requires row inference/resource-protocol work, not a bypass in program flow.

After the transport change, the delivered native suite was rerun with
`python3 -B development/library_check.py --stage sysroot-fallible-iteration --timeout 300`:
seven passed, exit 0; `/tmp/luna-library-b2m9sl4t/summary.json`.
Formatting check exited 0 (`/tmp/luna-tools-op9_9uul/result.json`), and
`git diff --check` passed. No full language suite was run.
