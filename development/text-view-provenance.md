# Text iteration prerequisite: borrowed storage provenance

`development/library-prototypes/builtin/text_iteration.fl` composes the existing
checked byte query with inline item delivery. It is intentionally outside the
sysroot and has no public export. The algorithm works in unchecked reference
execution; this does not prove a valid lifetime for the text view.

## Actual blocking paths

- `program/places.rs::readable_storage_input` does not form a runtime `str`
  pointer load. A proposed `Clone` implementation using `self.*` therefore fails
  during elaboration. No `str` resource implementation was added to the library.
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
