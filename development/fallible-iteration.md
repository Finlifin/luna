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

## Ordinary comptime values as error-row inputs

`!expr T` now accepts a named or computed `meta.List<Type>` value, rather than
requiring the error collection to be an HIR list literal. The expression is
checked in the comptime input phase, then passed through ordinary required
invocation and domain validation. Each delivered element must be Type. The
existing error-result constructor still handles duplicates and empty rows.
The supplied collection's domain must match its checked domain; an arbitrary
aggregate or list of values is not reinterpreted as Type labels.

A singleton Type expression keeps the existing type-description path, so
normal-form heads are not invoked or unfolded merely because they appear in
an error row. Literal rows retain support for symbolic signature parameters
without requiring an installed list protocol. Computed collections with a
finite shape can likewise contain restored generic Type parameters.

This is a prerequisite for general error-row inputs, not a solution to the
multi-label Copy reproducer. `!E T` is not uniquely decomposed against a
multi-label result: nested error qualifiers merge, and an unknown success
component can absorb error labels. Choosing one arbitrary decomposition or
turning pending into rejection would change inference semantics. A wholly
unresolved collection parameter still cannot be evaluated into a finite row;
no special Type-list parameter category or Copy-specific solver rule is added.

Validation in `/home/fin/luna`:

- `python3 -B development/check.py --timeout 900 -- test -p luna --test type_lists`:
  32 passed, exit 0; `/tmp/luna-tools-xleeguf1/result.json`. This includes
  named/computed rows, branch-selected empty rows, deduplication, scoped generic
  parameters, opaque normal-form identity, and rejection of wrong domains.
- Earlier combined `type_lists` / `error_result_flow` run: 31 + 11 passed,
  exit 0; `/tmp/luna-tools-7kdx4yln/result.json` (before the generic-row case).
- `python3 -B development/library_check.py --stage sysroot-fallible-iteration --timeout 300`:
  nine native cases passed, exit 0; `/tmp/luna-library-evund80q/summary.json`.
  The two new delivered cases exercise public meta.List constants, comptime
  row producers, duplicate collapse and empty-row success. The separate
  multi-label Copy reproducer is still not claimed as passing.
- `python3 -B development/check.py --timeout 900 -- test -p luna --test error_surface --no-fail-fast`:
  117 passed, one failed, exit 101; `/tmp/luna-tools-shmthiso/result.json`.
  `generic_error_labels_do_not_choose_ambiguous_permutations_or_unfold_heads`
  expects a call with an opaque head to reject argument inference, but it is
  accepted. The new identity test independently confirms no head unfolding;
  the broader inference assertion remains unresolved, not counted as passing.
- Library runner unit tests: 13 passed, exit 0.
- `python3 -B development/check.py --timeout 900 -- test -p elaboration --no-fail-fast`:
  passed, exit 0; `/tmp/luna-tools-r29xklk5/result.json`.
- Final formatting check exited 0 (`/tmp/luna-tools-iivke1oy/result.json`),
  and `git diff --check` passed. No full language suite was run.

## Retained-head inference regression alignment

The unresolved error-surface assertion above was stale: it rejected ordinary
candidate inference from retained normal-form inputs. The contract in
`docs/type/type-patterns.md` permits observing retained heads and arguments;
`docs/type/type-identity.md` separately forbids body unfolding and unsupported
inverse equality proofs. The inference implementation already proposes inputs
locally and retains the original application equality as an obligation (commit
43690fea); this change does not alter that implementation or the solver.

The error-row regression now accepts a same-head call whose type-producing
body would not terminate if invoked. It rejects erased labels, a different
head, conflicting ordinary arguments, and ambiguous unordered row assignments.
A positive control for the ordinary argument rules out unrelated signature
failure. PTC coverage independently keeps congruence distinct from injectivity.

Validation in `/home/fin/luna`:

- `python3 -B development/check.py --timeout 900 -- test -p luna --test error_surface --test normal_form_inference --no-fail-fast`:
  119 + 5 passed, exit 0; `/tmp/luna-tools-gthgsgeb/result.json`.
- `python3 -B development/check.py --timeout 900 -- test -p ptc --test solver -p type_resolution --test matching`:
  60 + 21 passed, exit 0; `/tmp/luna-tools-iq9ne_ex/result.json`.
- `python3 -B development/command.py --timeout 120 -- cargo fmt --all -- --check`:
  exit 0; `/tmp/luna-tools-gljkeb3a/result.json`; `git diff --check` passed.

No full language suite was run. General unresolved collection-row inference
and the multi-label Copy reproducer remain outstanding.

## Fallible reduction without an initial value

`Iterator.try_reduce` completes the reduction API using ordinary inline
iteration and the error-result ABI. Its callback consumes two owned items and
returns `!E Item`; the consumer returns `!E ?Item`. Empty input succeeds with
null, singleton input succeeds with its item, and neither invokes the callback.
Subsequent items combine in source order. The first error returns immediately,
retiring the source remainder and transferring the error payload to the caller.
No Clone or Copy constraint is added. A present optional item remains present
inside the successful optional reduction result, even when its payload is null.

The public sysroot fixture covers empty/singleton/nested optional results,
repeated affine accumulator replacement, early error and pending-source cleanup,
and cleanup of an unused callback capture. The existing Drop assertions detect
double retirement; assertions before and after result consumption check that
ownership remains with the returned success/error payload until consumed.
The implementation adds no compiler intrinsic, type rule or special dispatch.

Validation in `/home/fin/luna`:

- `python3 -B development/library_check.py --stage sysroot-fallible-iteration --timeout 300`:
  13 native tests passed, exit 0; `/tmp/luna-library-iyd9kf5v/summary.json`.
  This final run includes three-item successful affine reduction.
- `python3 -B -m unittest discover -s development -p test_library_check.py`:
  13 runner tests passed, exit 0.
- `git diff --check` passed. No Rust code changed and no full language suite
  was run. The previously recorded multi-label Copy limitation is unchanged.

## Nested pattern guards as program predicates

Boolean pattern selection now composes a nested effect-free guard with its
structural selection condition. Previously an error payload guard such as
`assumes input matches (error Low.value(value) if value < 255)` was admitted
but lost its bound before later arithmetic, unlike the equivalent match body.
The same mechanism applies to optional payloads and tuple fields.

Guard bindings are established in a forked successful pattern candidate.
The guard condition is translated back to existing input observations before
combining it with the structural selector; candidate locals and candidate-only
facts are not installed in the caller state. Unsupported or effectful guards
retain the conservative path. PTC is not involved.

Regression checks pair `< 255` acceptance with `<= 255` rejection, then check
negated selection, an unbounded alternative, and replacement after observation.
Predicate admission still rejects calls in both nested guards and match bodies.
The initial focused regression failed with the missing arithmetic-range proof
(`/tmp/luna-tools-a4y76b4k/result.json`), before this implementation change.

Validation in `/home/fin/luna`:

- `python3 -B development/check.py --timeout 900 -- test -p program_verification --no-fail-fast`:
  105 passed, exit 0; `/tmp/luna-tools-le7u393d/result.json`.
- `python3 -B development/check.py --timeout 900 -- test -p luna --test error_result_flow --test integer_contracts --test arithmetic_contexts --no-fail-fast`:
  14 + 18 + 10 passed, exit 0; `/tmp/luna-tools-i_ubauvm/result.json`.
- Final `error_result_flow` rerun after adding prior-field-binding compatibility:
  15 passed, exit 0; `/tmp/luna-tools-lfnupjwu/result.json`.
- `python3 -B development/library_check.py --stage sysroot-fallible-iteration --timeout 300`:
  13 native tests passed, exit 0; `/tmp/luna-library-zoc9juk3/summary.json`.
- `python3 -B development/command.py --timeout 120 -- cargo fmt --all -- --check`:
  exit 0; `/tmp/luna-tools-fd91da5d/result.json`; `git diff --check` passed.

No full language suite was run. The prior-field-binding test checks that ordinary
matching remains valid; it does not claim complete relational predicate export
for arbitrary cross-field guards. Multi-label Copy inference is unaffected.

## Ordered cross-field guard relations

Structural predicate selection now carries a private successful candidate
through fields in declaration order. A later field guard can use earlier
bindings, including those outside a nested tuple. Each field's condition is
translated back to original observations before that field is assumed true;
its own equalities cannot justify rewriting the predicate being exported.
The candidate state is discarded after construction. Ordinary variant matching
now uses the same complete structural selection after priming payload paths,
rather than analyzing each field with a separate binding environment.

Regression coverage includes tuples, enum payloads, nested tuples, error
payloads, and or-pattern alternatives that swap binding names. For Byte fields,
`first < second` proves `first + 1` safe; `first <= second` does not. Unbounded
alternatives, reassignment and negated equality continue rejecting unsafe
arithmetic. The initial cross-field regression failed with the missing range
proof (`/tmp/luna-tools-2wxkyp5a/result.json`) before this change.

This extends the previous nested-guard implementation; unsupported pattern
computations still retain the conservative path. No PTC rule or language
semantics changed, and this does not claim complete program verification.

Validation in `/home/fin/luna`:

- Final command: `python3 -B development/check.py --timeout 900 -- test -p luna --test error_result_flow --test snapshot_contracts --test normal_patterns --test integer_contracts -p program_verification --lib --no-fail-fast`.
  The selected source suites passed (18 + 2 + 42 + 18), as did all 99 program
  verification unit tests. Overall exit was 101 because `--lib` also selected
  luna's library tests: 70 passed, 12 existing host method-contract fixtures
  failed during HIR lowering due to missing bool bindings. This is not a green
  aggregate command; evidence: `/tmp/luna-tools-ml9wrkoj/result.json`.
- Earlier `test -p program_verification --no-fail-fast` passed all 105 tests;
  `/tmp/luna-tools-ssz94i69/result.json`. This precedes removal of the redundant
  final condition remap; the final unit tests above cover that final change.
- `python3 -B development/library_check.py --stage sysroot-fallible-iteration --timeout 300`:
  13 native tests passed, exit 0; `/tmp/luna-library-2hx1_9x3/summary.json`.
  This run also precedes removal of the redundant final remap; no final-version
  native rerun is claimed.
- `python3 -B development/command.py --timeout 120 -- cargo fmt --all -- --check`:
  exit 0; `/tmp/luna-tools-hcvz90pl/result.json`; `git diff --check` passed.

No full language suite was run. The unrelated host-fixture failures and
multi-label Copy inference remain outstanding.
