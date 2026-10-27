# Text iteration prerequisite: borrowed storage provenance

`development/library-prototypes/builtin/text_iteration.fl` composes the existing
checked byte query with inline item delivery. It is intentionally outside the
sysroot and has no public export. The algorithm works in unchecked reference
execution; this does not prove a valid lifetime for the text view.

## Actual blocking paths

- `program/places.rs::readable_storage_input` now forms runtime `str` descriptor
  loads and replacements, including descriptors nested in records and tuples.
  Formation does not authorize Copy or prove backing-region liveness. No `str`
  resource implementation was added to the library.
- `verification/memory.rs::owned_fields` explicitly rejects `str` because its
  backing character storage has no represented provenance. This also rejects
  passing a text view inside the proposed Bytes iterator, even when the fixture
  happens to construct it from a literal.
- Existing resource tests already require that nominal wrappers, tuples and
  explicit Copy evidence cannot turn a borrowed text view into pointer-free
  data. This is a safety boundary to preserve, not an incidental missing match
  arm to bypass.

## Required implementation

The next implementation needs an explicit backing-region dependency for text
views, separate from the slot storing the view. Literal and Symbol-name storage
can provide immutable static origins; input views need symbolic origins supplied
by callers. Returning or embedding a view must preserve those origins through
aggregates, calls, closures and inline delivery. Copying the view transports its
dependency without acquiring ownership of the backing storage. Reading bytes
must retain the region's liveness and read footprint; bounded byte access does
not itself prove a valid region lifetime.

Do not map Str to a pointer-free scalar or an unrestricted mutable byte pointer.
The current pointer-layout helpers model typed storage paths, not a variable-size
immutable character region. They must not silently reinterpret the view's host
pointer/length representation as an existing scalar pointer contract.

Acceptance needs successful static-backed and caller-backed view transport plus
rejection of dead owners, escaped local backing storage and invalidated views.
Only after those checks should the text resource protocols and public bytes
iterator enter the compiled library. The deferred String/allocator work must
use the same dependency representation rather than introducing another model.

## Current executable evidence

Working directory: `/home/fin/luna`.

```sh
python3 -B development/check.py --timeout 900 -- test --release -p luna --test text_iteration_prototype
```

Report `/tmp/luna-tools-3ucejhzl/result.json`, exit 0: two tests pass. Both execute
the prototype before and after inline expansion in the unchecked interpreter,
and separately assert that resource-checked execution rejects the missing view
provenance. Expected bytes are specified independently for ASCII, two-/three-/
four-byte UTF-8 characters and NUL. Empty input, filter composition, continue,
break and lexical return are exercised. These are algorithm and rejection
checks, not successful safe-mode or native execution of this feature.

## Implemented layout stage

`memory::borrowed_input_layout` now describes text-view slots separately from
ordinary pointer leaves. Text leaves retain a logical input path, physical
storage path, and enclosing variant guards. A pointer to a text slot remains an
ordinary pointer with Str pointee; it is not the view's backing character region.
Tuple heads use admitted layout metadata; arbitrary normal-form heads remain
opaque. Records, newtypes, enums, native optional/error payloads and captured
closure environments preserve their existing layout identities.

The existing `pointer_input_layout` API consumes this shared traversal but
rejects any text leaf, including its S-expression path in the diagnostic. Other
legacy Str rejection points remain in place. This refactors real layout consumers
without granting them missing provenance evidence. Unknown layouts, exhausted
budgets and excessive depth remain errors; failed queries leave no partial result
that can be used as a successful layout.

This stage does not supply backing-region origins, transport through function
summaries, or lifetime verification. Those remain required before the prototype
can be exported. Layout discovery is the input to that implementation, not a
replacement for it.

### Layout validation

Working directory: `/home/fin/luna`.

```sh
python3 -B development/check.py --timeout 900 -- check --release -p elaboration --tests -p luna --test text_view_layout
python3 -B development/check.py --timeout 900 -- test --release -p elaboration --lib -p luna --test text_view_layout --test text_iteration_prototype --test library_protocols
python3 -B development/check.py --timeout 900 -- test --release -p luna --test resources borrowed_string_views_do_not_gain_pointer_free_layout_from_copy -- --exact
python3 -B development/command.py --timeout 120 -- cargo fmt --all -- --check
git diff --check
```

The compile check passed (`/tmp/luna-tools-4y5h7yti/result.json`, exit 0).
The combined execution passed 286 elaboration unit tests, 80 luna unit tests,
25 library protocol tests, 2 prototype tests, and 1 source layout test
(`/tmp/luna-tools-gukf1ysj/result.json`, exit 0). The source test retains a real
Tuple normal-form head and checks nested enum/newtype text slots and guards.
Formatting passed (`/tmp/luna-tools-rhi0lfot/result.json`, exit 0), as did the
diff check. Runtime text-region transport is still not implemented.
The existing str/Copy regression also passed (one test, three source shapes;
`/tmp/luna-tools-578yimc5/result.json`, exit 0). Explicit Copy evidence still does
not authorize dropping the backing-storage dependency.

## Named-body region dependency analysis

`memory::infer_text_regions` now derives a separate `TextRegionSummary` for a
concrete named runtime body. Origins are either immutable static storage (literal
or Symbol/enum name views) or a caller parameter plus its logical text-leaf path.
They never refer to the local slot holding a copy of the view. The summary has a
stable S-expression Display and retains the concrete instance arguments.

The analysis transports dependencies through local bindings, branches and
returns, ordinary named calls, record/tuple fields, optional and error payloads,
newtype conversion edges and captured closure environments. Calls substitute
callee parameter origins with the actual argument origins; swapping arguments
therefore swaps their dependencies. Generic instances use declaration binder
sorts and their complete concrete arguments. A closure construction transports
its captured views without pretending to invoke the closure.

Explicit byte reads and text comparisons collect character-region read
obligations, including reads in discarded call results. Length queries observe
the descriptor without reading character bytes. These read sets do not include
implicit destructor footprints and are not complete access permissions. Cleanup
composition, region liveness and invalidation checks are still required.

Summaries are reused only inside one immutable query. Missing code,
indirect calls, pointer transfers,
effectful pattern observations remain
rejected in this analysis. They are never converted into static or empty-origin
evidence. The existing runtime resource checker still rejects unsupported text
views; this new analysis has not replaced that gate or exported the iterator.

The distinction matters: returning an input region describes a dependency, not a
proof that its backing owner survives. A subsequent integration must bind those
origins to live caller regions and compose cleanup before granting any operation.

### Region dependency validation

Working directory: `/home/fin/luna`.

```sh
python3 -B development/check.py --timeout 900 -- test --release -p luna --test text_regions --test text_view_layout --test text_iteration_prototype
python3 -B development/command.py --timeout 120 -- cargo fmt --all -- --check
git diff --check
```

All nine tests passed (`/tmp/luna-tools-gjzc4pmf/result.json`, exit 0): six
region-summary tests, one source layout test and two iteration prototype tests.
The summary tests exercise named-call substitution, discarded reads, text
comparison reads, descriptor-only length queries, optional absence, error
payloads, nested newtype conversions, generic instances and closure captures.
They also check budget failure/recovery, immutable package input and rejection
of recursive, indirect and mutation transfers. The prototype still verifies
unchecked execution separately from the required resource-checker rejection.
Formatting passed (`/tmp/luna-tools-0108jmy_/result.json`, exit 0).

This stage does not validate native text iteration or owner lifetime safety;
neither is enabled by the new summary API.

## Local writes and control-flow transport

Local assignment now replaces the dependency subtree at its destination after
evaluating the right-hand side. A field write preserves sibling dependencies;
assigning an absent optional clears the old payload dependency. This describes
view contents only, not the destructor accesses needed to replace an old value.
Writes through pointers remain unsupported.

Block scopes propagate writes to existing outer bindings without leaking new
locals. Branches join the environments of successors that can continue: a branch
that returns contributes its return dependencies and reads, but its local writes
do not flow into statements after the branch. Reads observed before replacement
remain part of the function summary.

Validation from `/home/fin/luna` used the same three test targets listed above:
`python3 -B development/check.py --timeout 900 -- test --release -p luna --test text_regions --test text_view_layout --test text_iteration_prototype`.
All ten tests passed (`/tmp/luna-tools-y40okuuy/result.json`, exit 0). Added
source cases check replacement, block propagation, conditional joins, an early
return, sibling-field preservation and clearing an optional payload. A byte-read
case checks that reads retain the old origin while the return uses the new one.
The workspace formatting check passed (`/tmp/luna-tools-hx76gvbt/result.json`,
exit 0), as did `git diff --check`.

## Loop fixed points

Text origins now flow across NIR loop backedges until the incoming environment
stabilizes. The initial environment remains part of the loop-head join; body
fallthrough and explicit continues supply backedges. Breaks supply a separate
exit environment, and lexical target identities route nested/labeled transfers.
Only bindings present at the target loop entry survive these transfers.

Known boolean conditions exclude unreachable branches. A loop without any normal
exit does not invent a returned value; explicit returns retain their own origin
summary. Named callee analysis uses a separate loop stack, so callee-local control
flow cannot bind to caller loops. Fuel bounds repeated traversal and dependency
joins, and exhausted inference does not publish a provisional result.

This computes a conservative origin fixed point, not termination or lifetime
proofs. Pointer writes, computed patterns and implicit cleanup remain separate
unsupported transfers or obligations as described above.

Validation from `/home/fin/luna`:

```sh
python3 -B development/check.py --timeout 900 -- test --release -p luna --test text_regions --test text_view_layout --test text_iteration_prototype
python3 -B development/command.py --timeout 120 -- cargo fmt --all -- --check
git diff --check
```

All eleven tests passed (`/tmp/luna-tools-4ehhtw2d/result.json`, exit 0).
Loop cases cover zero iterations, guaranteed replacement before break,
multi-iteration dependencies with a named call, nested labeled break/continue,
unreachable bodies, infinite loops, returns and budget failure/recovery.
Formatting passed (`/tmp/luna-tools-ghrf7e0x/result.json`, exit 0), as did the
diff check. The runtime provenance gate remains in place.


## Ordered pattern matching

Match subjects are evaluated once. Structural patterns bind text origins from
record/tuple fields, enum payloads and native optional/error branches; binding
remaps retain those dependencies. String literal patterns record character reads.
Arms run in order, excluding known mismatches and stopping after a guaranteed
match with successful guards. Top-level pattern guards and arm guards evaluate in
order, retaining reads and outer-local updates on candidate failure. Candidate
bindings never enter the next arm or escape the match.

Continuing arm environments join separately from returned values and lexical
loop transfers. Computed views, nested executable guards and alternative patterns
remain rejected rather than silently dropping candidate evaluation. This remains
an origin analysis; guard observation legality, cleanup and backing-region
liveness are separate checks.

Validation from `/home/fin/luna`:

```sh
python3 -B development/check.py --timeout 900 -- test --release -p luna --test text_regions --test text_view_layout --test text_iteration_prototype
python3 -B development/command.py --timeout 120 -- cargo fmt --all -- --check
git diff --check
```

All twelve tests passed (`/tmp/luna-tools-zftdvm60/result.json`, exit 0).
New source cases exercise optional/enum payload origins, record fields,
known boolean arm selection, string-pattern reads, guard fallback and arm-local
writes to an outer binding. Formatting passed
(`/tmp/luna-tools-xh31geov/result.json`, exit 0), as did the diff check.


## Default arguments and short-circuit evaluation

Named calls now validate the normalized argument plan before transporting text
origins. Explicit arguments evaluate in source order in the caller environment.
Defaults evaluate in declaration order in a fresh frame containing prepared
parameter values. The frame identities must match the named callee; caller
locals and loop targets cannot leak into default evaluation. A default's reads
remain caller obligations, while its resulting text origins enter the actual
argument map for summary substitution. A return inside a default is rejected by
this analysis rather than interpreted as a return from the caller.

Boolean `and` and `or` evaluate their left operand first. A known short-circuit
result skips the right operand entirely. For an unknown condition, the continuing
right-hand environment joins the skipped path; returns and divergence on the
right do not erase the skipped path. Reads before a transfer remain obligations.
These rules propagate origins and do not waive argument cleanup or lifetime
requirements.

Input-derived text default expressions still fail the source P3
stability/termination gate. The dependent text-default test therefore starts from typed explicit calls
and constructs a normalized default argument frame in NIR; it verifies this
analysis, not source-level certification. A separate source case exercises an
ordinary boolean default while transporting a text argument. Neither the P3 gate
nor resource permission checks were relaxed.

Validation from `/home/fin/luna`:

```sh
python3 -B development/check.py --timeout 900 -- test --release -p luna --test text_regions --test text_view_layout --test text_iteration_prototype
python3 -B development/check.py --timeout 900 -- test --release -p luna --test text_regions default_arguments_use_prepared_parameter_origins -- --exact
python3 -B development/command.py --timeout 120 -- cargo fmt --all -- --check
git diff --check
```

All fourteen tests passed (`/tmp/luna-tools-8sz_ouz7/result.json`, exit 0).
After adding malformed default-order/frame cases, the exact default-argument test
passed (`/tmp/luna-tools-j_suol4r/result.json`, exit 0). Short-circuit tests cover
skipped character reads, conditional and forced writes, and right-side returns.
Formatting passed (`/tmp/luna-tools-0ze7izq4/result.json`, exit 0), as did the diff
check. The earlier failed run (`/tmp/luna-tools-5hye7i8m/result.json`) exposed the
P3 source restriction and a missing static-literal read in the test expectation;
the implementation gates were retained and those test assumptions corrected.


## Static text default construction

P3 default validation now distinguishes a direct string-literal constructor from
copying an existing text input. A correctly typed string literal creates a view
of immutable static storage, completes without failure and does not acquire an
external owner. The same construction evidence applies when a literal is a field
of a directly delivered record. Conditional literal results remain finite when
their condition has ordinary P3 evidence.

This does not add Str to the scalar-Copy whitelist. Input-derived defaults and
local text temporaries still need resource evidence; runtime backing-region
permission checks are unchanged. The origin analyzer can now consume source
calls using literal text defaults and distinguish explicit parameter-backed
overrides from static default origins.

Validation from `/home/fin/luna`:

```sh
python3 -B development/check.py --timeout 900 -- test --release -p luna --test text_regions --test text_view_layout --test text_iteration_prototype
python3 -B development/check.py --timeout 900 -- test --release -p luna --test default_calls --test runtime_purity
python3 -B development/command.py --timeout 120 -- cargo fmt --all -- --check
git diff --check
```

All fifteen text tests passed (`/tmp/luna-tools-ghwc52em/result.json`, exit 0),
including source static-default construction and the continued rejection of
input copies and local text temporaries. The existing default-call and runtime
purity suites passed 43 and 64 tests respectively
(`/tmp/luna-tools-pa79jaft/result.json`, exit 0). Formatting passed
(`/tmp/luna-tools-ayhlzsuw/result.json`, exit 0), as did the diff check.


## Reads on failed structural patterns

A whole-pattern mismatch does not prove that matching performed no reads. For
example, `(text, false)` matched against `("x", true)` compares text before the
second field fails. The initial structural-match implementation skipped that
first read when the complete pattern was known to fail.

Pattern binding now returns match knowledge after visiting fields in matching
order. It retains reached string comparisons and stops at the first known
mismatch. Known variant tags select payload traversal before field matching;
unknown tags remain conservative. Failed candidates do not evaluate their guards
or bodies. The same traversal binds successful fields and remaps exported names,
so match knowledge cannot bypass the dependency walk.

Validation from `/home/fin/luna`:

```sh
python3 -B development/check.py --timeout 900 -- test --release -p luna --test text_regions failed_structural_patterns_keep_only_reads_reached_before_mismatch -- --exact
python3 -B development/check.py --timeout 900 -- test --release -p luna --test text_regions --test text_view_layout --test text_iteration_prototype
python3 -B development/command.py --timeout 120 -- cargo fmt --all -- --check
git diff --check
```

The exact regression failed before the fix with a missing parameter read
(`/tmp/luna-tools-keu_g4lm/result.json`, exit 101). After the fix, all sixteen
tests passed (`/tmp/luna-tools-4t4cbqky/result.json`, exit 0), including earlier
and later field mismatches, matching prefixes, enum payloads and skipped guards.
Formatting passed (`/tmp/luna-tools-q2pkpgg5/result.json`, exit 0), as did the
diff check. Existing runtime permission restrictions remain unchanged.


## Alternative and observational patterns

Or-pattern candidates start from independent binding environments. Failed
candidates retain their reads but export no bindings. Potentially successful
candidates join common exported identities; a guaranteed success stops later
alternatives. A guard outside the or-pattern runs after this choice and cannot
restart its alternatives on failure.

Nested guards, observation aliases and sequential `and ... is ...` computations
now transport text origins. A computed pattern receives the computed expression's
own origins and type, rather than inheriting its original subject's text fields.
Pattern observations may use private block temporaries, but assignments to
existing bindings are rejected. Nested observations use an isolated loop scope
and reject nonlocal returns or paths without normal completion; they do not
silently turn those transfers into candidate failure. These are conservative
analysis restrictions, not new language rules. Resource observation permission
and implicit cleanup remain independent obligations.

Validation from `/home/fin/luna`:

```sh
python3 -B development/check.py --timeout 900 -- test --release -p luna --test text_regions --test text_view_layout --test text_iteration_prototype
python3 -B development/command.py --timeout 120 -- cargo fmt --all -- --check
git diff --check
```

All eighteen tests passed (`/tmp/luna-tools-cy6uw1wd/result.json`, exit 0).
New cases cover swapped alternative bindings, first-success short circuit,
outer-guard failure without alternative retry, nested field guards and computed
boolean patterns reading text. Formatting passed
(`/tmp/luna-tools-hirfcpps/result.json`, exit 0), as did the diff check.


## Let-pattern success and failure

Let-patterns use the same ordered dependency walk as match arms. A successful
pattern installs its bindings in the enclosing block; a failure retains only the
outer binding environment, so a candidate name cannot shadow an outer name used
by the else branch. Guard reads and reachable outer updates are retained before
failure dispatch. A let-else failure branch must not complete normally; its
returns and lexical loop exits contribute to their ordinary destinations.

Known mismatches do not invent a successful continuation, while text comparisons
performed before the mismatch remain in the read summary. Irrefutability and
resource observation legality remain the frontend/resource checker's contracts;
this analysis does not replace either check.

Validation from `/home/fin/luna`:

```sh
python3 -B development/check.py --timeout 900 -- test --release -p luna --test text_regions --test text_view_layout --test text_iteration_prototype
python3 -B development/command.py --timeout 120 -- cargo fmt --all -- --check
git diff --check
```

All nineteen tests passed (`/tmp/luna-tools-d2mrb7dm/result.json`, exit 0).
New cases cover tuple/optional bindings, outer-name shadowing in else branches,
known failure, retained failed-pattern reads, guards, lexical break, alternatives
and computed-pattern exports. Formatting passed
(`/tmp/luna-tools-0x96hmiw/result.json`, exit 0), as did the diff check.


## Error-result representation transport

Text dependencies now use the existing checked representation-transport plan for
error-row embedding and product conversion. Destination paths follow the new
error tags, including conversions nested in tuple/record products. Each leaf's
origin continues to name its original caller input path; moving the view's slot
never changes the identity of its backing storage. Colliding destination entries
join origin sets rather than discarding evidence.

This handles error-result representation changes, not the algebraic-effect ABI.
It does not read characters or grant new lifetime permissions. Matching a
transported value does not treat repacking as a constructor with a known tag.

Validation from `/home/fin/luna`:

```sh
python3 -B development/check.py --timeout 900 -- test --release -p luna --test text_regions --test text_view_layout --test text_iteration_prototype
python3 -B development/check.py --timeout 900 -- test --release -p luna --test error_surface inferred_error
python3 -B development/command.py --timeout 120 -- cargo fmt --all -- --check
git diff --check
```

All twenty text tests passed (`/tmp/luna-tools-f2g8lbwo/result.json`, exit 0).
The widening fixture explicitly checks that the text tag changes while its
parameter origin does not. Generic tuple transport, sibling fields and budget
failure/recovery are also checked. An explicit NIR Product fixture exercises the
identity plan after concrete-body instantiation canonicalizes its rows; it is
not evidence of a nonidentity Product plan at that stage. Earlier fixture runs
failed because they incorrectly expected noncanonical row order to survive
source normalization or concrete instantiation (`/tmp/luna-tools-qwd2c5vs`,
`/tmp/luna-tools-7surs1bb`, `/tmp/luna-tools-niax6xmk`, and
`/tmp/luna-tools-uqwbm9_l`). Those assumptions were corrected without changing
normalization or the transport implementation.

The existing inferred-error regression group passed sixteen tests
(`/tmp/luna-tools-lg74csqc/result.json`, exit 0), including nested product
conversion and retained Tuple heads. Formatting passed
(`/tmp/luna-tools-_6bm5tmh/result.json`, exit 0), as did the diff check.

## Source propagation integration

Source `?` and `!` already lower to the matching and return forms handled by this
analysis; they need no separate text transfer rule. The integration test checks
optional presence, distinct error/success text payloads, widening on an early
error return, and a text comparison after successful propagation. The later read
depends on the success payload and its other operand, while the error return
retains the input error payload's origin.

From `/home/fin/luna`,
`python3 -B development/check.py --timeout 900 -- test --release -p luna --test text_regions optional_and_error_propagation_keep_separate_exit_dependencies -- --exact`
passed (`/tmp/luna-tools-c1h08fj3/result.json`, exit 0). This adds source-level
composition evidence for the existing transfer rules; it does not certify
cleanup or safe runtime execution of text views.


## Scalar computations surrounding text reads

Integer/float arithmetic, scalar bit operations and conversions now evaluate
their operands in order before returning an empty text-origin map. Earlier reads
and exits remain part of the expression's control flow. Dynamic bit-window
endpoints likewise follow the value in evaluation order. Arithmetic and bit
bounds remain independent proof obligations; this origin analysis does not
certify them.

Target intrinsics use `nir::intrinsics::checked` before their inputs are analyzed.
Only admitted scalar operations have the no-hidden-memory behavior needed here;
a tag without a valid binding is rejected. Their input expressions can still read
text, update local origins or exit early, and retain those dependencies.

Validation from `/home/fin/luna`:

```sh
python3 -B development/check.py --timeout 900 -- test --release -p luna --test text_regions --test text_view_layout --test text_iteration_prototype
python3 -B development/command.py --timeout 120 -- cargo fmt --all -- --check
git diff --check
```

All twenty-three tests passed (`/tmp/luna-tools-elk7xe93/result.json`, exit 0).
The byte-scanning fixture covers UTF-8, empty input, early success and failure,
integer index updates and explicit BitVec zero-extension. Its unchecked
interpreter result is asserted separately from its read summary. The scalar
intrinsic case covers float negation/addition, text-dependent FMA inputs, missing
admission rejection and recovery using the original package.

Earlier runs failed on fixture syntax (`+` whitespace,
`/tmp/luna-tools-zsy6ttq2`) and an unsupported direct u8-to-u64 conversion protocol
(`/tmp/luna-tools-4burvanw`). The corrected fixture uses the existing `bitvec`
view conversion; neither parsing nor conversion protocols were relaxed.


## Program-contract dependencies

Entry snapshots retain their parameter's initial text origins in fresh bindings.
Preconditions are analyzed before the body. Functions with postconditions collect
outer parameter/snapshot environments at every explicit return and normal body
completion, then analyze exit predicates with the joined actual result origins.
A function with no normal return does not invent a result binding or evaluate its
postconditions. Named calls substitute these predicate reads along with body
reads into the caller summary.

Predicates must be checkable and use supported observational expressions; writes
to existing bindings, unsupported control transfers and unknown operations are
still rejected. Recording predicate dependencies does not prove a predicate,
assert that the call cannot reject, or discharge cleanup/lifetime obligations.
Program predicates remain outside PTC.

The source flow checker still rejects general text predicates with
`unsupported-construct`; the initial source fixture demonstrated that boundary
(`/tmp/luna-tools-3_l5wpol/result.json`, exit 101). Text predicate propagation is
therefore tested with explicit well-typed NIR contracts. Separate source fixtures
exercise admitted boolean predicates and actual entry-snapshot elaboration.
This does not widen the source verifier's accepted specifications.

Validation from `/home/fin/luna`:

```sh
python3 -B development/check.py --timeout 900 -- test --release -p luna --test text_regions --test text_view_layout --test text_iteration_prototype
python3 -B development/check.py --timeout 900 -- test --release -p luna --test text_regions contracts_observe_entry_snapshots_and_all_normal_return_origins -- --exact
python3 -B development/command.py --timeout 120 -- cargo fmt --all -- --check
git diff --check
```

The three suites passed all 24 tests (`/tmp/luna-tools-ngfuaqd5/result.json`,
exit 0). The strengthened exact contract test subsequently passed
(`/tmp/luna-tools-8xu7ef3m/result.json`, exit 0): a parameter reassignment on the
fallthrough path joins with its original origin at an early return, whereas an
entry snapshot retains only the original origin. Other assertions cover named
callee preconditions, result-bound postconditions, nonreturning bodies and query
budget failure followed by recovery.


## Recursive instance fixed points

Named calls now register dependencies between concrete function instances instead
of recursively traversing Rust stack frames. A deterministic work queue starts
each discovered instance with no observed normal return and no reads. An instance
that gains reads or result origins schedules its callers again; joins preserve
all previously discovered dependencies. Self-recursion, mutual recursion and
concrete generic recursion use the same argument-origin substitution as ordinary
calls. An unproductive recursive cycle can have reads without a returned value.

The initial approximation is internal solver state, never a completed summary.
Every discovered instance must finish analysis before any result is exported.
Observations requiring normal completion may temporarily fail while a callee is
pending; dependency growth retries them. Any failure left when the queue drains
rejects the whole query, including unsupported operations in nested callees.
Fuel bounds both instance discovery and iteration. Failed queries retain no state
for a later invocation. This is dependency inference, not a termination proof or
an extension of text access permissions.

Validation from `/home/fin/luna`:

```sh
python3 -B development/check.py --timeout 900 -- test --release -p luna --test text_regions --test text_view_layout --test text_iteration_prototype
python3 -B development/command.py --timeout 120 -- cargo fmt --all -- --check
git diff --check
```

All 26 tests passed (`/tmp/luna-tools-5tzocjpc/result.json`, exit 0). New cases
cover self/mutual recursion, parameter permutation, concrete generic instances,
reads in nonreturning recursion, budget failure and query recovery. Nested
computed-pattern observations exercise retry after a pending callee gains a
normal-return summary. An unsupported indirect call inside a recursive callee
still rejects both direct and transitive queries.

Two earlier fixture attempts failed before validating that observation path:
`/tmp/luna-tools-z62tsqft/result.json` used the reserved name `predicate`, and
`/tmp/luna-tools-5_bumkkd/result.json` attached a function call as a program
predicate. The latter remains outside `checkable_predicate` admission. The final
fixture uses the supported computed-pattern observation path; neither keyword
rules nor contract admission was relaxed.

## Immediate closure invocation

A syntactically available closure callee now has an executable transfer rule.
Its environment is constructed before source-ordered arguments, preserving the
captured view even if an argument later reassigns the original caller slot.
Capture and parameter origins populate a separate invocation environment; writes
to those bindings do not replace the caller's descriptors. Nested closure bodies
and named calls retain their character-read dependencies.

The body uses separate return and loop state. A closure-local return contributes
to the invocation result, not the enclosing function's result or postcondition
exit environment. Conversely, an exit during callee construction or argument
preparation still belongs to the caller. A nonreturning closure does not invent
fallthrough. Named recursion through an immediate closure participates in the
existing instance fixed point.

Unknown callable values, callable parameters and local closure aliases remain
unsupported by this analysis. This rule supplies neither resource delivery
protocols nor permission to invoke a closure with dead captured storage; the
existing runtime gates remain in place.

Validation from `/home/fin/luna`:

```sh
python3 -B development/check.py --timeout 900 -- test --release -p luna --test text_regions --test text_view_layout --test text_iteration_prototype
python3 -B development/command.py --timeout 120 -- cargo fmt --all -- --check
git diff --check
```

All 27 tests passed (`/tmp/luna-tools-2pws6xt_/result.json`, exit 0). Source cases
cover direct parameter and captured returns, capture-before-argument mutation,
nested invocations, discarded closure-local returns and reads, recursion through
an immediate closure, and divergence. An explicit NIR result postcondition
checks that a closure return cannot contaminate the enclosing exit environment.
The iterator prototype continues to assert rejection by the resource checker.

## Stable local callable identities

Each concrete body now indexes local definitions whose callable identity is
stable: named functions, concrete instances, closure literals and alias chains.
Any assignment to a binding (including a field assignment), duplicate definition,
or explicit restored capture under that identity excludes it from this index.
Resolution is fuel-bounded. This is deliberately conservative about mutation;
it does not guess a candidate when a callable may have changed.

The index supplies code only. Invocation evaluates the actual local value and
uses its stored text-region environment, preserving captures made before a later
assignment to an outer source variable. Alias copying, nested captured callables,
branch-local invocation and named recursion share the existing transfer rules.
Unknown callable inputs, assigned callable identities, and callable results from
other functions still require additional value-flow tracking. Runtime lifetime
and resource checks are unchanged.

Validation from `/home/fin/luna`:

```sh
python3 -B development/check.py --timeout 900 -- test --release -p luna --test text_regions --test text_view_layout --test text_iteration_prototype
python3 -B development/command.py --timeout 120 -- cargo fmt --all -- --check
git diff --check
```

All 28 tests passed (`/tmp/luna-tools-bbvcawl9/result.json`, exit 0). Added source
cases cover named aliases, closure aliases retaining earlier captures, captured
callables inside another closure, reads, branches, recursion, changed-callable
rejection, and budget failure followed by recovery. Formatting passed
(`/tmp/luna-tools-cdeokr21/result.json`, exit 0); the diff check passed.
An initial compile failure (`/tmp/luna-tools-s1ckbo7x/result.json`, exit 101)
identified that LocalId provides ordering rather than hashing; the local index
uses ordered maps/sets without changing LocalId or its identity contract.

## Invocation-local callback parameters

Immediate or stable-local closure invocation now carries known callback code
alongside argument text origins. Inputs can supply named functions, concrete
instances, closure literals or stable aliases. Code identities are prepared in
the caller environment, installed only after all arguments have been evaluated,
and restored after body analysis on both success and failure. Captured text
continues to come from the actual argument value, not the code definition.

A callback parameter can be invoked directly, reached through a local alias, or
captured by a nested immediate closure. Reusing the same invocation closure with
different callbacks does not retain the earlier parameter binding. Parameters
assigned anywhere in the concrete body are excluded from this stable-code rule;
unknown callbacks remain unknown and fail if invoked. Named higher-order summary
specialization and general mutable callable flow remain separate work.

Validation from `/home/fin/luna`:

```sh
python3 -B development/check.py --timeout 900 -- test --release -p luna --test text_regions --test text_view_layout --test text_iteration_prototype
python3 -B development/command.py --timeout 120 -- cargo fmt --all -- --check
git diff --check
```

All 29 tests passed (`/tmp/luna-tools-yu9phub9/result.json`, exit 0). New source
coverage exercises named callback inputs, nested captures of a callback parameter,
local callback aliases, repeated invocation with different code identities,
text-capturing callbacks and their reads, unknown callback rejection, and budget
failure followed by recovery. Formatting passed
(`/tmp/luna-tools-34h9sgv3/result.json`, exit 0); the diff check passed.

## Named higher-order summary contexts

The fixed-point key now includes the concrete named function instances supplied
as callback parameters, ordered by parameter index. The same higher-order body
with different callbacks therefore has separate pending state, dependencies and
completed summaries. Forwarded callback parameters retain their named identity;
recursive forwarding revisits the same specialized key instead of losing the
call target. The internal key has an S-expression Display.

Analysis installs callback code under the callee's parameter identities, while
text origins still use the ordinary caller-parameter substitution. Assigned
parameter identities remain excluded. Default inputs accept directly named code
only: a default's local identity belongs to its prepared callee frame and cannot
be resolved from the outer caller's code map.

This stage specializes named function callbacks, including concrete generic
instances. Capturing closures passed across named function boundaries still
require closed code-environment identities; they are not treated as named code
or as empty-effect callbacks. Unknown root callback inputs remain rejected when
invoked. Existing closure-local callback support and runtime resource gates are
unchanged.

Validation from `/home/fin/luna`:

```sh
python3 -B development/check.py --timeout 900 -- test --release -p luna --test text_regions --test text_view_layout --test text_iteration_prototype
python3 -B development/command.py --timeout 120 -- cargo fmt --all -- --check
git diff --check
```

All 30 tests passed (`/tmp/luna-tools-yw0348eb/result.json`, exit 0). New cases
alternate callbacks returning input versus static text within one query, forward
callbacks through another named function, instantiate a generic callback, carry
comparison reads, and recursively swap text arguments while retaining the
callback identity. Unknown callback roots and capturing callbacks crossing a
named boundary remain rejected. Budget failure followed by recovery passes.
Formatting passed (`/tmp/luna-tools-lga54ztw/result.json`, exit 0), as did the
diff check.

## Closed callback code environments

Capturing callbacks can now cross named function boundaries. A query-local
registry interns a closure's concrete code together with its local callable
definitions and captured callable identities. Summary keys reference that closed
code identity. Ordinary captured text remains symbolic input data and is mapped
through the existing capture paths; it is not baked into the code identity.

Closing a callback recursively closes known callable captures, including nested
closures, and retains its body's stable local aliases. Invocation temporarily
installs this code environment, then restores the caller context after success
or failure. The actual closure value still supplies text origins. Forwarding an
already closed callback reuses its identity, allowing recursive named calls to
reach a fixed point. Same-source closures with different captured functions have
distinct contexts and cannot share an incorrect summary.

Registry construction, comparisons and retained code traversals consume query
fuel, with a separate closure-environment depth limit. Assignment exclusions and
unknown callback rejection remain in place. General mutable callable flow,
callable results returned from other functions, and lifetime/cleanup proof are
still separate requirements. Closure defaults requiring a prepared code frame
are not enabled by this stage.

Validation from `/home/fin/luna`:

```sh
python3 -B development/check.py --timeout 900 -- test --release -p luna --test text_regions --test text_view_layout --test text_iteration_prototype
python3 -B development/command.py --timeout 120 -- cargo fmt --all -- --check
git diff --check
```

All 31 tests passed (`/tmp/luna-tools-wbk4l2hx/result.json`, exit 0). New source
cases cover captured text through named forwarding and recursion, character
reads, nested captured closures, and a same-source closure capturing different
named functions in successive calls. Local aliases inside closed callback bodies
retain the captured code identity. Unknown callback roots remain rejected;
budget failure followed by a fresh query succeeds. The earlier named-callback
rejection test now accepts its concrete captured closure while retaining rejection
for a root parameter with no supplied code identity.
Formatting passed (`/tmp/luna-tools-47oxl104/result.json`, exit 0); the diff check
passed. These checks establish dependency transport, not lifetime permission.

## Prepared default callback frames

Default inputs now receive both text origins and known code identities from
already-prepared callee parameters. The default expression's local definitions
are indexed in that frame. A default may alias an earlier callback, construct a
closure over earlier inputs, or invoke an earlier callback; these actions do not
consult the caller's lexical code bindings. Explicit inputs still evaluate in
the caller frame before their values are installed under callee identities.

Each default analysis restores the caller's code, mutation-exclusion and loop
contexts on success or failure. Its return state and postcondition-exit frame are
isolated, and a nonlocal default return remains an error. Unknown code remains
unknown rather than being inferred from a coincidentally matching caller binding.

Text-derived source defaults still require P3 certification. The new regression
uses typed explicit source calls and installs NIR default frames to test this
transfer rule independently; it does not relax source purity or resource gates.

Validation from `/home/fin/luna`:

```sh
python3 -B development/check.py --timeout 900 -- test --release -p luna --test text_regions --test text_view_layout --test text_iteration_prototype
python3 -B development/check.py --timeout 900 -- test --release -p luna --test text_regions default_callbacks_use_prepared_code_and_capture_frames -- --exact
python3 -B development/command.py --timeout 120 -- cargo fmt --all -- --check
git diff --check
```

All 32 tests passed (`/tmp/luna-tools-7_zb46tz/result.json`, exit 0). The exact
new test was rerun after the final nested-default fixture addition and passed
(`/tmp/luna-tools-5a1cc24o/result.json`, exit 0). It covers a default alias of a
named callback, a default capturing earlier text, copying a prepared closure,
invoking an earlier callback to compute a default, and a default closure capturing
an earlier closure parameter. Budget failure and a subsequent fresh query pass.
Formatting passed (`/tmp/luna-tools-z6gkdv20/result.json`, exit 0), as did the
diff check. Text default coverage here is NIR transfer evidence, not source P3
certification or runtime lifetime permission.

## Source and package CLI inspection

`--analyze-text-regions <entry>` now runs this analysis from ordinary source or
an explicit `--packages-v1` request. The entry must be an ordinary nongeneric
runtime function without unresolved type/capability assumptions. Runtime
parameters remain symbolic; the function is not executed. Primary-package
semantic identity disambiguates dependency functions with the same name.

The output is the existing text-region summary S-expression. `--fuel`, atomic
`--output`, and the package `--with-inputs` envelope are supported. Conflicting
actions and Copy/Drop options are rejected; the command does not acquire resource
permissions or certify complete memory safety. Unsupported dependencies, missing
entries and exhausted budgets fail without replacing previous output. The
resident compile-only stream continues to reject analysis actions.

Validation from `/home/fin/luna`:

```sh
python3 -B development/check.py --timeout 900 -- test --release -p luna --test frontend_cli text_region_cli
python3 -B development/check.py --timeout 900 -- test --release -p luna --lib cli
python3 -B development/check.py --timeout 900 -- test --release -p luna --test frontend_cli resource_cli
python3 -B development/command.py --timeout 120 -- cargo fmt --all -- --check
git diff --check
```

Three new black-box CLI tests passed (`/tmp/luna-tools-a1lozou5/result.json`,
exit 0), covering captured callbacks, comparison reads, output shape, entry and
phase rejection, budgets, preserved output, conflicting options, provider calls
and primary-package selection. Nine existing CLI unit tests passed
(`/tmp/luna-tools-t1vakrbs/result.json`, exit 0), as did the three existing resource
CLI tests (`/tmp/luna-tools-b646bl0t/result.json`, exit 0). Formatting passed
(`/tmp/luna-tools-6uqsn06p/result.json`, exit 0); the diff check passed.

## Callable identities in value flow

Internal abstract values now carry callable identities by field path alongside
text origins. Function summaries retain both domains during fixed-point solving;
the public text summary still exports only character reads and text dependencies.
Calls consume the evaluated callee value once, so a named function or captured
closure returned by a factory can be invoked immediately, stored in a local,
forwarded, projected from an aggregate or unpacked from an optional.

Projection, construction, assignment, return joins and error representation
transport preserve code paths together with text paths. An assignment replaces
both domains for its destination subtree. Copies retain the earlier identity,
and closure construction saves callable captures from the evaluated values.
Consequently a deterministic reassignment can be followed by a call without
reusing stale lexical definition metadata.

Callable input leaves start unknown unless specialized code is supplied. Joins
preserve one identity only when all present alternatives agree; conflicting or
unknown identities become unknown and are rejected if invoked. This prevents a
known branch from erasing an unknown aggregate callback. General multi-candidate
invocation remains unimplemented. Runtime ownership, cleanup and lifetime gates
are unchanged by this value-flow extension.

Validation from `/home/fin/luna`:

```sh
python3 -B development/check.py --timeout 900 -- test --release -p luna --test text_regions --test text_view_layout --test text_iteration_prototype
python3 -B development/check.py --timeout 900 -- test --release -p luna --test frontend_cli text_region_cli
python3 -B development/command.py --timeout 120 -- cargo fmt --all -- --check
git diff --check
```

All 34 analysis/layout/prototype tests passed
(`/tmp/luna-tools-ty_blvq3/result.json`, exit 0), including returned factories,
immediate invocation, forwarding, aggregate and optional projection, deterministic
assignments, copies and captures before reassignment, equal and conflicting
branch identities, unknown input callbacks, and budget failure recovery. All
three text-region CLI tests passed (`/tmp/luna-tools-d9tdwdlk/result.json`,
exit 0). Formatting passed (`/tmp/luna-tools-77nvmybs/result.json`, exit 0), as did
the diff check. Callable fields passed inside aggregate parameters still need
path-sensitive specialization across named function boundaries; this stage does
not claim that support or runtime lifetime/cleanup certification.

## Aggregate callback parameter specialization

Named-call summary keys now include the parameter index, logical field path and
known callable identity for every supplied callable leaf. Previously only the
parameter root could specialize a call, losing code carried by record fields or
variant payloads. Callee input values install these identities at their original
paths; unspecified leaves remain unknown. Text origins remain symbolic inputs
and are substituted separately when the summary returns to its caller.

Keys are ordered by parameter and path, and path storage is charged to the query
budget. The S-expression key dump exposes each `(path ...)`. This supports
nested aggregate forwarding, optional payload callbacks and captured closures
inside records without conflating different field arrangements or bypassing
unknown-call rejection. It does not add multi-target invocation, variant
feasibility pruning, runtime text permissions or lifetime/cleanup proofs.

Validation from `/home/fin/luna`:

```sh
python3 -B development/check.py --timeout 900 -- test --release -p luna --test text_regions --test text_view_layout --test text_iteration_prototype
python3 -B development/command.py --timeout 120 -- cargo fmt --all -- --check
git diff --check
```

All 35 tests passed (`/tmp/luna-tools-b9g7d8ci/result.json`, exit 0). The new
source regression checks distinct record fields, nested forwarding, differently
specialized calls in one caller, optional unpacking, captured text closures,
recursive aggregate forwarding, unknown callbacks and budget failure recovery.
Formatting passed (`/tmp/luna-tools-r16cccsz/result.json`, exit 0), as did the
diff check. Optional input variants remain conservatively joined, so the test
expects both the payload dependency and the absent-branch static result.

## Text descriptor storage formation

The frontend now forms typed `ReadPointer` and `WritePointer` operations for a
text descriptor and supported aggregates containing text. This removes the early
representation rejection for expressions such as `pointer.*` where the pointee
is `str`, without treating text as an intrinsic pointer-free value. Descriptor
storage and backing character storage remain separate obligations.

The regression checks direct and projected descriptor reads, whole record and
Tuple reads, and descriptor replacement through a pointer. It also checks that
resource-checked execution rejects the same programs both without a Copy
implementation and with explicit Copy evidence for `str`. An isolated interpreter fixture
with explicit pointer-copy permission establishes descriptor operation behavior;
the default interpreter still rejects these operations. The fixture does not certify the
backing storage lifetime or enable public text resource protocols.

Validation from `/home/fin/luna`:

```sh
python3 -B development/check.py --timeout 900 -- test --release -p luna --test text_view_layout --test text_regions --test text_iteration_prototype
python3 -B development/check.py --timeout 900 -- test --release -p luna --test text_view_layout
python3 -B development/check.py --timeout 900 -- test --release -p luna --test resources borrowed_string_views_do_not_gain_pointer_free_layout_from_copy
python3 -B development/command.py --timeout 120 -- cargo fmt --all -- --check
git diff --check
```

The initial combined run (`/tmp/luna-tools-95u_6nuf/result.json`, exit 101)
passed all 32 dependency tests and both iteration tests, but the new descriptor
fixture hit the interpreter's deliberate non-intrinsic pointer permission check.
The fixture was corrected to assert this rejection, then exercise representation
behavior with explicit test-only pointer-copy permission. Both layout tests then
passed (`/tmp/luna-tools-7oeea0zi/result.json`, exit 0). The existing Copy/provenance
regression passed (`/tmp/luna-tools-jbthh_rh/result.json`, exit 0). Formatting
passed (`/tmp/luna-tools-b0h_t4i4/result.json`, exit 0), as did the diff check.
No interpreter permission or runtime resource gate was relaxed.

## Local descriptor addresses in dependency analysis

Text abstract values now carry local descriptor-storage addresses separately
from backing-character origins. Address formation and field projection identify
a local slot; reading the slot projects its current dependency value. Pointer
writes evaluate the destination before the replacement and replace only that
subtree, preserving sibling fields and earlier copied views. Aliases and loop
joins carry the address alongside text and callable information.

Unknown or nonuniform targets cannot be dereferenced. Lexical scope exit rejects
known addresses into expired local storage; summaries cannot export local
addresses. Passing local addresses to named calls or closure invocations, or
capturing them in closures, still needs coordinated memory-effect transfer and
is rejected. Observational patterns cannot mutate through pointers. These checks
prevent the standalone dependency query from silently omitting caller-memory
updates; they do not replace Copy checking or prove backing-region liveness.

Validation from `/home/fin/luna`:

```sh
python3 -B development/check.py --timeout 900 -- test --release -p luna --test text_regions --test text_view_layout --test text_iteration_prototype
python3 -B development/command.py --timeout 120 -- cargo fmt --all -- --check
git diff --check
```

All 37 tests passed (`/tmp/luna-tools-xzrfd2c1/result.json`, exit 0). New cases
cover descriptor aliases, replacement versus saved copies, field/sibling origins,
branch and loop joins, pointer evaluation before RHS rebinding, character reads,
unknown/ambiguous targets, local address escape, unsupported cross-frame aliases,
and budget failure recovery. Earlier fixture-only failures
(`/tmp/luna-tools-4bvykun_/result.json` and
`/tmp/luna-tools-1cy8zdxi/result.json`, exit 101) were corrected to separate `.*`
from `=` and use `do { ... }` for expression-position statement blocks, following
`docs/grammar/syntax`. No parser rule or semantic expectation was relaxed.
Formatting passed (`/tmp/luna-tools-6a06gke4/result.json`, exit 0), as did the
diff check. Runtime resource permissions remain unchanged.

## Named calls with caller descriptor storage

Calls carrying known local descriptor addresses now analyze the instantiated
callee body against an isolated binding frame sharing the caller's abstract
storage values. Explicit inputs are evaluated once before body analysis. Reads
observe current caller origins, writes update caller slots, and all normal exits
(including explicit returns) join before propagating state back. Entry snapshots
and pre/postcondition observations use the callee frame; caller code bindings,
loop state and return tracking are restored after success or failure.

This preserves alias relationships when two parameters address the same slot,
supports nested named forwarding and callback calls, and allows returned pointers
to caller-owned descriptor slots. Both returned values and writes to caller
storage are checked for callee-local address escapes. Ordinary value-only calls
retain symbolic cached summaries. Recursive pointer-effect calls are rejected
until a relational storage fixed point is available; value-only recursion still
uses its existing fixed point. Closure pointer captures and default expressions
that dereference caller storage remain separate work. No runtime Copy or backing
region lifetime permission is granted by this dependency transfer.

Validation from `/home/fin/luna`:

```sh
python3 -B development/check.py --timeout 900 -- test --release -p luna --test text_regions --test text_view_layout --test text_iteration_prototype
python3 -B development/command.py --timeout 120 -- cargo fmt --all -- --check
git diff --check
```

All 38 tests passed (`/tmp/luna-tools-_h3qj2wg/result.json`, exit 0). New cases
cover nested replacement, saved pre-write views, distinct and identical pointer
arguments, generic pointer returns, field/sibling preservation, callbacks,
argument effects before body execution, early-return joins and character reads.
Recursive storage calls and callee-local addresses written through caller
pointers are rejected; budget exhaustion followed by a fresh query recovers.
Formatting passed (`/tmp/luna-tools-k44b57es/result.json`, exit 0), as did the
diff check. Existing runtime resource rejection tests still pass.

## Descriptor reads during default preparation

A default expression whose earlier prepared parameters contain local addresses
now receives the caller's current abstract storage alongside the prepared callee
bindings. Direct loads and finite named helper calls can therefore resolve the
original descriptor slots, including changes made while evaluating earlier
explicit arguments. The preparation frame cannot export its private addresses
or change caller storage. Pointer writes reached during default evaluation are
rejected, including writes through named helper calls analyzed in that frame;
the previous read-only state is restored on every exit.

This follows the default-expression boundary in `docs/functions/parameters.md`:
per-evaluation pointer observations do not authorize pointer writes or replace P3,
Copy and memory admission. Text-valued defaults are tested at typed NIR because
source text Copy/P3 evidence remains unfinished. An ordinary source-level bool
pointer default additionally checks the admitted path after a text comparison.

Validation from `/home/fin/luna`:

```sh
python3 -B development/check.py --timeout 900 -- test --release -p luna --test text_regions --test text_view_layout --test text_iteration_prototype
python3 -B development/command.py --timeout 120 -- cargo fmt --all -- --check
git diff --check
```

All 40 tests passed (`/tmp/luna-tools-oqu7exha/result.json`, exit 0). The new
NIR regression covers direct and helper descriptor reads, preceding argument
updates, direct/helper pointer-write rejection, unknown origins, restoration of
ordinary writes after default evaluation and budget recovery. The source bool
pointer default also passes and preserves the preceding text comparison's read
dependencies. Formatting passed (`/tmp/luna-tools-bn46y7u3/result.json`, exit 0),
as did the diff check. This does not establish source admission for text defaults.

## Closure descriptor storage frames

Closure invocation now separates lexical capture bindings from the caller's
storage identities. Caller slots are moved into query-local storage frames;
captured and argument pointer roots are remapped to those frames. Captured text
values remain snapshots in the closure's lexical environment. Consequently a
closure can retain the old text value while a captured pointer replaces the
external descriptor, without conflating the two under one source `LocalId`.

Storage frames participate in branch, pattern and loop joins, normal-return
collection, named pointer calls and default preparation. Closure arguments are
evaluated before the caller storage snapshot is installed, while the closure
capture values were already evaluated with the callee. Normal exits restore
caller slots and their original address identities. Return values and indirect
writes cannot retain addresses of closure-local storage. Frame identities are
bounded query-local analysis data and do not alter source or IR binding IDs.

This adds pointer captures, closure pointer parameters, nested closures and
closures passed through named functions to the dependency query. Runtime Copy,
cleanup and backing-character lifetime verification remain separate gates.

Observation-only pointer access is also propagated through named calls. A
regression demonstrated that a computed pattern could call a helper which wrote
the caller descriptor and still receive a successful summary. The query now
carries the read-only observation context into that helper, rejecting the write
instead of allowing candidate-state rollback to hide it. Default preparation
uses the same scoped mechanism with its own diagnostic.

Validation from `/home/fin/luna`:

```sh
python3 -B development/check.py --timeout 900 -- test --release -p luna --test text_regions --test text_view_layout --test text_iteration_prototype
python3 -B development/check.py --timeout 900 -- test --release -p luna --test frontend_cli text_region_cli
python3 -B development/command.py --timeout 120 -- cargo fmt --all -- --check
git diff --check
```

All 42 analysis/layout/prototype tests passed
(`/tmp/luna-tools-hqwd928b/result.json`, exit 0), including capture-versus-storage
separation, pointer arguments and returns, nested/named/repeated closure calls,
argument order, early exits, loop convergence, escaped local addresses, budget
recovery and indirect observation-write rejection. All three CLI tests passed
(`/tmp/luna-tools-mb8t2l7t/result.json`, exit 0). Formatting passed
(`/tmp/luna-tools-6y3ppi0j/result.json`, exit 0), as did the diff check.

The first build (`/tmp/luna-tools-4phtntwq/result.json`, exit 101) exposed two
snapshot insertion borrow errors after the environment refactor; cloning before
insertion fixed them. The next run (`/tmp/luna-tools-9t4cf6em/result.json`, exit
101) passed closure cases but reproduced the indirect observation-write hole.
The final run verifies the scoped read-only fix. Recursive pointer-effect calls
still need relational fixed points; this stage does not certify text liveness.
