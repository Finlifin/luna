# Runtime storage checks for contextual code

CMTT expression quotes, expression generator interfaces and pattern quotes now
share a complete nominal representation snapshot, including newtypes. Looking
through a newtype for storage phase does not equate its identity with its
representation or unfold an opaque normal-form head. Phantom input Types that
are absent from the representation remain legal.

The shared NIR storage inspection now descends through Optional payloads. A
runtime optional cannot hide Type, Integer, Float, Object or comptime function
storage, even when its current value might be absent. These optional domains
remain usable in comptime functions.

The original reproduction was incorrectly accepted before the change:

```flurry
typealias Type = .Type'builtin;
newtype Hidden = Type;
comptime fn identity(code: <|-Hidden>) -> <|-Hidden> { code }
```

No invocation or splice is needed: definitions must expose valid runtime code
interfaces. The failing pre-fix test report is
`/tmp/luna-tools-o1p9miui/result.json`.

The new source suite covers 180 combinations of underlying domain, direct or
nested nominal representation, expression results/contexts, optional and
function results, pattern subjects/outputs, and discarded quote declarations.
Positive tests execute ordinary newtype code and retain phantom Type inputs.
NIR tests independently exercise optional runtime parameters and permitted
comptime parameters.

Validation from `/home/fin/luna`:

```sh
python3 -B development/check.py --timeout 900 -- test --release -p nir --test phases
python3 -B development/check.py --timeout 900 -- test --release -p luna --test cmtt_phase_boundaries --test cmtt --test newtypes
```

The NIR command passed 35 phase tests (`/tmp/luna-tools-p4qb9285/result.json`).
Formatting and diff checks passed.

The source command passed all 527 CMTT tests and the three new boundary tests.
The broad newtypes target passed 199 tests but failed 12, so that command exited
101 (`/tmp/luna-tools-5fyupxi7/result.json`). Ten failures panic at
`program/type_defaults.rs:242` while preparing generic member inputs. The other
failures are `dynamic_chained_members_reuse_outer_inputs_and_self` (Type input
annotation) and `installed_effects_do_not_revive_wrapped_pointer_targets`
(diagnostic expectation). These were follow-up failures, not passing coverage; a before-change
comparison of that full target was not performed.

This change does not transport quoted code through the ordinary VM or complete
source builder default/method publication. It closes a storage-phase hole in the
existing code interface boundary needed by that work.

## Generic projection follow-up

Constant members capture the selected container inputs but do not own a nominal
clause telescope. Their expression preparation now bypasses member parameter
metadata after rejecting explicit member arguments; container formation and
constant initializer contracts still run. This removed the ten metadata lookup
panics in the first rerun (209 passed, two failed;
`/tmp/luna-tools-vl91ft3h/result.json`).

Resolver-identified type and constant members are now prepared before enum-group
namespace probing. Otherwise an alias chain such as
`Alias<value>.Captured.This` tries to complete an invocation-time Type parameter
as a static type annotation. Enum groups retain their namespace-first handling:
moving all projections ahead of that probe broke the existing alias/group test
in `/tmp/luna-tools-i5_q0t4f/result.json`; the final ordering is restricted to
members identified by the resolver.

The wrapped-pointer effect regression now recognizes the explicit diagnostic
that the pointer owner is not fully initialized. It also checks the same two
closure forms without consuming the owner, requiring each to return `true`.
This preserves the rejection requirement and verifies that a live owner is not
rejected indiscriminately.

Final validation from `/home/fin/luna`:

```sh
python3 -B development/check.py --timeout 900 -- test --release -p luna --test newtypes --test enum_groups
python3 -B development/command.py --timeout 120 -- cargo fmt --all -- --check
git diff --check
```

All 211 newtype tests and seven enum-group tests passed (exit 0,
`/tmp/luna-tools-s9e6lnc0/result.json`). Formatting passed (exit 0,
`/tmp/luna-tools-1thfn6qg/result.json`); diff whitespace checks passed.
These close all twelve failures recorded above, without claiming that the
ordinary VM now transports quoted code or publishes source builder defaults.

## Shared contextual code witnesses

`nir::code_values::{ExpressionCode, PatternCode}` now owns the code witness
representation previously private to the elaborator. Both expression and
pattern generators, template consumers and splice expansion use these shared
structures through the existing imports. The S-expression dumps retain their
previous shape, including escaped port/output names and explicit pending-body
markers.

The shared interface validator checks runtime storage domains against the
provided nominal metadata and scope. Expression ports and default source keys
must refer to context identities; a default target may remain an external
lexical identity. Fixed dependencies are retained independently of exposed
ports. Quotation and pattern checking call this validator; body typing,
substitution, resource checking and rejection of pending bodies at emission
remain with their existing consumers. Public Rust construction is not proof
that a body is well typed or ready for publication.

This removes the elaboration-private representation dependency for future VM
and definition-builder consumers. It does not add an ordinary VM Value variant,
a contextual storage domain, Any transport, or source builder code defaults.
Those paths must retain code category and interface evidence rather than using
the quote's runtime result type as the type of the code value itself.

Validation from `/home/fin/luna` (dev/test profile):

```sh
python3 -B development/check.py --timeout 900 -- test -p nir --test code_values --test code_interfaces
python3 -B development/check.py --timeout 900 -- test -p luna --test cmtt --test cmtt_phase_boundaries
python3 -B development/command.py --timeout 120 -- cargo fmt --all
git diff --check
```

All 17 NIR tests passed (`/tmp/luna-tools-uu8kd2lm/result.json`), including
malformed interface rejection/recovery, fixed dependencies and external default
identities, comptime storage rejection, and pending-body preservation. All 527
CMTT tests and three phase-boundary tests passed
(`/tmp/luna-tools-zxvjy0yw/result.json`). Formatting completed successfully
(`/tmp/luna-tools-nkavxnfy/result.json`); diff whitespace checks passed.

## Invocation-owned VM code storage

The VM now carries `Value::Code(CodeValue)` with distinct expression and pattern
variants referencing the shared NIR witnesses. Owned reads, explicit owned
copies and rollback checkpoints retain the same immutable code identity,
context/default mappings and success bindings. They do not evaluate quoted
bodies; a nonterminating runtime body is valid to transport.

Transport charges interface names, domains and body traversal. Missing bodies,
nested unresolved pattern-code markers, malformed expression port/default
sources, exhausted budgets and excessive body nesting are rejected before a
copy is returned. Failure leaves the original owner intact; a later operation
with adequate budget can succeed. Raw Rust construction remains untrusted:
these representation checks do not certify body typing, scope substitution or
runtime resource obligations.

Code has no ordinary Term data domain. Typed immutable input admission and
normal-term/cache-key export reject it, also inside aggregates. Runtime VM
copies reject code with PhaseMismatch. Ordinary constant materialization has an
explicit code rejection rather than treating a quote as its runtime result.
The internal Value comparison uses shared witness identity; it does not provide
a source Eq protocol or claim equivalence of separately constructed programs.

This delivers invocation-owned transport only. Source quotation still uses the
elaborator's code-value path. Ordinary callable contextual domains, typed code
containers, Any transport and source builder default/method publication remain
to be connected; no library API is advertised as supporting those paths yet.

Validation from `/home/fin/luna` (dev/test profile):

```sh
python3 -B development/check.py --timeout 900 -- test -p comptime_interpretor --lib
python3 -B development/check.py --timeout 900 -- test -p luna --test cmtt --test cmtt_phase_boundaries --test comptime_memory
python3 -B development/check.py --timeout 900 -- test -p luna --test execution --test nominal_value_parameters memoized
python3 -B development/command.py --timeout 120 -- cargo fmt --all -- --check
git diff --check
```

All 208 VM tests passed, including six code-transfer tests
(`/tmp/luna-tools-vu_lm4tw/result.json`). The CMTT/phase/memory groups passed 547
tests (`/tmp/luna-tools-59p4z5mr/result.json`); the result-cache groups passed 14
(`/tmp/luna-tools-qc71fxts/result.json`). The earlier dev-stack failure and its
independent freezing repair are recorded in `development/comptime-vm.md`.

## Contextual inputs independent of result category

Expression and pattern signatures now share `CodeFunctionSignature<O>`.
`PatternFunctionSignature` specializes the output to `PatternCodeInterface`;
`DataFunctionSignature` specializes it to an ordinary Term. The input telescope
continues to distinguish Data, Code and Pattern. A Type-valued data result
therefore does not turn a code parameter into Type or into its runtime result.
Shared substitution, budget charging and dependency traversal cover every
input interface and the actual output domain. Existing expression/pattern dump
shapes are unchanged; data results use `data-consumer-signature`.

Both source generator categories now use one argument packet and the VM's
`CodeValue` category tags, including methods, defaults and conditional code
bindings. Definition-side pending witnesses remain permitted in the elaborator;
this does not bypass the VM transport rejection of missing executable bodies.

The next source boundary is a data-returning contextual call. Current
`is_code_generator`/`is_pattern_generator` classification selects functions by
their contextual return annotation. Ordinary signature formation cannot express
contextual inputs, and the construction control-flow evaluator currently has
expression and pattern returns only. Consequently the shared data signature is
not yet a source-callable feature. Connecting it requires:

- Registering code-consuming data functions separately from ordinary callable
  Terms, retaining clause domains, captures, defaults and contracts.
- Reusing code-aware argument routing and generator frames for input admission;
  ordinary runtime argument evaluation must not be introduced for code values.
- Extending construction control flow with data returns, checking all definition
  paths while executing only selected invocation paths. Eagerly elaborating an
  entire data body and then running its NIR would execute code-construction
  effects from unselected branches and is not a valid shortcut.
- Returning the resulting ordinary value through required comptime invocation,
  before exposing this path to library builder defaults or typed code containers.

Validation from `/home/fin/luna`:

```sh
python3 -B development/check.py --timeout 900 -- test -p nir --test code_interfaces --test code_values
python3 -B development/check.py --timeout 900 -- test -p luna --test cmtt --test cmtt_phase_boundaries
python3 -B development/command.py --timeout 120 -- cargo fmt --all
git diff --check
```

All 20 NIR tests passed (`/tmp/luna-tools-jlgkv_3s/result.json`), including mixed
input categories with a Type result, substitution failure/recovery and complete
budget/dependency coverage. All 530 source CMTT and phase-boundary tests passed
after the shared argument change (`/tmp/luna-tools-5z6ptvxj/result.json`).
Formatting completed (`/tmp/luna-tools-3vq5nnji/result.json`); whitespace checks
passed. The earlier source run before the argument unification is not the final
validation for that change.

## Source contextual functions returning data

Explicit comptime functions with expr/pattern code parameters can now return
ordinary data, Type, or void. They share signature formation and substitution
with expression generators (`Generator<O>`), retain contextual input categories,
and use a separate source declaration registry. They do not become ordinary
callable Terms or erase code into its runtime result type.

Direct required calls and construction statement/binding calls route supplied
arguments in source order. Defaults run only for omitted arguments. The existing
construction frame preserves owned caller cells; code parameters stay checked
elaboration witnesses rather than ordinary VM data inputs. The shared control
interpreter now has a data return channel, including bare return and implicit
void completion. Definition checking checks unused bodies and unselected paths;
actual calls execute only selected if/when/match branches, loops and returns.
Preconditions precede the body, and existing entry-snapshot/postcondition handling
is retained for contracts without result bindings.

`development/examples/sysroot-contextual-data.fl` passes expr code through two
explicitly specialized functions, uses the public `meta.StructBuilder` methods,
and returns a generated Type with a usable runtime layout and reflection. This
also required removing replaced declaration binders from required invocation's
abstract-Type permissions. Actual opaque binders in forwarded substitutions and
input graphs are still collected; the builder's rejection of abstract publication
is unchanged. Constructing the example's slot does not execute the supplied
quote or install a code field default.

Current implementation limits (not restrictions added to the language):

- Calls must occur at direct required/construction boundaries. Arbitrary calls
  nested inside ordinary expressions are rejected rather than eagerly run during
  expression elaboration. Explicit statement locals can separate such calls.
- Clause arguments must currently be explicit. Contextual data methods, variadic
  collection, argument packets, and result-bound contracts remain unconnected.
- The construction evaluator still uses native call frames. This path permits
  at most eight nested contextual data invocations, rejecting deeper calls with
  a diagnostic. An explicit invocation stack is required to raise this limit
  without relying on a larger host/test stack. Infinite recursion is tested at
  the ordinary dev/test stack setting.
- Typed code containers, Any transport and definition-builder code defaults are
  still separate unfinished work. These calls do not enter the ordinary result
  memoizer; the pure-Type-only cache policy remains unchanged.

Validation from `/home/fin/luna`, all using dev/test profiles:

```sh
python3 -B development/check.py --timeout 900 -- test -p luna --test cmtt --test builder_invocations contextual_data
python3 -B development/check.py --timeout 900 -- test -p luna --test cmtt --test cmtt_phase_boundaries --test comptime_memory
python3 -B development/command.py --timeout 120 -- cargo fmt --all
git diff --check
```

The first command passed 11 source regressions and the complete-sysroot builder
regression (`/tmp/luna-tools-4paksx16/result.json`). The full CMTT, phase and
comptime-memory groups passed 538 + 3 + 17 tests
(`/tmp/luna-tools-es593tf_/result.json`). Cases cover both code categories,
Type/void results, explicit clauses, defaults, preconditions, recursion limits,
unused definitions, unselected construction, and source-ordered single evaluation
of named arguments with owned-state updates. Formatting completed
(`/tmp/luna-tools-jmrnusxk/result.json`).

Broader builder runs did not pass: existing dev-stack overflows in
`comptime_list_index_early_return_preserves_prior_effects` and
`enum_reflection_observes_live_generated_types_and_preserves_nominal_identity`
aborted the harness (`/tmp/luna-tools-8fe15bxe/result.json` and
`/tmp/luna-tools-cyo8clmn/result.json`). Both were reproduced individually after
replacing only this stage's five edited implementation files with their HEAD
versions at b541c05f, preserving other worktree changes, then restoring the five
files byte-for-byte. Baseline reports are `/tmp/luna-tools-faf09iva/result.json`
and `/tmp/luna-tools-xqhw1rp2/result.json`. No release profile or increased host
stack was used to hide these failures. Commands for individual reproduction:

```sh
python3 -B development/check.py --timeout 900 -- test -p luna --test builder_invocations comptime_list_index_early_return_preserves_prior_effects
python3 -B development/check.py --timeout 900 -- test -p luna --test builder_invocations enum_reflection_observes_live_generated_types_and_preserves_nominal_identity
```

After extending the depth guard to include argument preparation, the final CMTT,
phase and memory run passed 539 + 3 + 17 tests
(`/tmp/luna-tools-atxqk_wh/result.json`). The added regression checks nested
argument admission separately from recursive bodies. Final formatting completed
(`/tmp/luna-tools-mmeh7uou/result.json`).

The example also passed its LLVM native test (one test, no failures) with:

```sh
python3 -B development/check.py --timeout 900 -- run -p luna -- development/examples/sysroot-contextual-data.fl --sysroot library --test-native
```

Report: `/tmp/luna-tools-7ic2gurt/result.json`. This native run preceded only the
argument-depth guard extension; it validates generated runtime layout and field
access, not unlimited construction recursion.

The final complete-sysroot builder regression also passed after the depth guard
extension: `python3 -B development/check.py --timeout 900 -- test -p luna --test
builder_invocations contextual_data_consumer_publishes`
(`/tmp/luna-tools-w7v_28ks/result.json`, one passed).


## Result-bound contracts for contextual data consumers

Contextual functions returning data now accept named results. Definition checking
supplies the result Type to the ordinary postcondition checker and admits its
binding only inside the predicate frame. Invocation installs the completed result
in owned generator storage after the body returns, then checks postconditions
before returning it to the caller. Early returns and tail results use the same
path. Input `old` expressions still use the entry snapshot; a result has no entry
value. Reading the checked result uses metered representation transport and
preserves nested storage identities. A named result without postconditions does
not allocate a postcondition cell.

```flurry
comptime fn inspect(code: <|- bool>, flag: bool) -> result: bool
where ensures result == flag'old
{
    if flag { return true; }
    false
}

fn run() -> bool {
    const result = inspect('(true), true);
    result
}
```

This removes the result-binding limitation listed in the preceding stage. It
does not change predicate admission: Type equality is still rejected by the
program-predicate checker, so Type-returning functions can currently use supported
data predicates but not `ensures result == T`. Another existing limitation is
using a contextual call's unknown Type result as a local type while checking a
consumer definition; completed calls at required boundaries remain supported.
Neither limitation was changed to make these tests pass. Contextual methods,
variadic collection, arbitrary expression scheduling and the invocation-depth
limit are unchanged.

Validation from `/home/fin/luna`:

```sh
python3 -B development/check.py --timeout 900 -- test -p luna --test cmtt --test cmtt_phase_boundaries --test comptime_memory
python3 -B development/command.py --timeout 120 -- cargo fmt --all
git diff --check
```

All passed: 543 CMTT, three phase-boundary and 17 memory tests
(`/tmp/luna-tools-etm46rby/result.json`). Four new source regressions exercise
selected/early returns, false postcondition rejection, entry snapshots, invalid
unused predicates, nominal fields, nested calls, Type results, void completion,
and named results without predicates. Formatting passed at
`/tmp/luna-tools-4xfkqn1z/result.json`; whitespace checks passed.


## Shared input inference for contextual data calls

Contextual data consumers now use the same source-ordered input preparation as
expression-code generators. The shared `generators/inputs.rs` routine is generic
over the result category; data outputs remain Terms, not synthetic quoted code.
The existing code-generator inference, clause-default completion, partial input
contexts, argument adaptation and final interface checks are retained. Internal
inference products do not become Flurry Tuple types or erase code interfaces.

Calls can infer clauses from quoted expr results, established pattern subject and
output interfaces, ordinary data inputs and expected output types. They can also
settle declared Type/value defaults. Insufficient evidence and conflicting explicit
arguments still reject the call; formation requirements are checked after solving.
Closed contextual data results can contribute type hints without treating the
callee as an ordinary callable or executing its arguments/defaults/body during
probing. Generic unresolved results remain unknown hints until actual preparation.
The construction depth guard continues to include nested argument preparation.

The public builder example now calls `SlotFor(code)` and `ForwardSlot('(true))`
without explicit type clauses. Its generated nominal type, reflection and native
field access are unchanged. This supersedes the explicit-clauses-only limit in
the earlier source-consumer stage. Contextual methods, argument packets/variadic
consumer collection, arbitrary nested expression scheduling and abstract Type
results used as local types remain separate unfinished work. Memoization policy
is unchanged.

Validation from `/home/fin/luna`:

```sh
python3 -B development/check.py --timeout 900 -- test -p luna --test cmtt --test cmtt_phase_boundaries --test comptime_memory --test builder_invocations
python3 -B development/check.py --timeout 900 -- test -p luna --test builder_invocations contextual_data_consumer_publishes
python3 -B development/check.py --timeout 900 -- run -p luna -- development/examples/sysroot-contextual-data.fl --sysroot library --test-native
python3 -B development/command.py --timeout 120 -- cargo fmt --all
git diff --check
```

All passed: 549 CMTT, 73 builder, three phase-boundary and 17 memory tests
(`/tmp/luna-tools-dnszp099/result.json`). After editing the builder example, its
complete-sysroot regression passed (`/tmp/luna-tools-asli6i8u/result.json`) and its
native test passed (`/tmp/luna-tools-s7uf3vpv/result.json`, one case). Six new CMTT
regressions cover code/pattern evidence, generic forwarding, numeric target context,
Type/value defaults, conflicting/absent evidence, formation constraints and
source-ordered single evaluation of named inputs. Formatting completed at
`/tmp/luna-tools-w9x04ln4/result.json`; whitespace checks passed.


## Data-consumer variadics and argument expansion

Contextual data consumers now admit ordinary data variadics and reuse the shared
collection path. The declared domain describes the whole collected Tuple, not
one element. Empty and heterogeneous collections retain the registered Tuple
normal-form head; named supply delivers an already formed collection. Duplicate
supply, incorrect element types and incorrect fixed widths remain errors.

Calls with spread arguments use the existing spread planner. Width/type probing
does not execute producers. Nonempty packets are prepared once at their source
position, and empty packets still execute their effects at that position.
Supplied code inputs and data elements retain their interleaved order. Parameter
defaults and result contracts see the completed collection. `()` remains void;
the empty-packet test obtains a real `Tuple<>` through empty collection instead
of treating void as a tuple.

The public `sysroot-contextual-data.fl` example now uses
`PairFor('(true), ...do { (.left, .right) })` to generate a two-field struct via
`meta.StructBuilder`. Its reflection check validates field order and Types; its
native case constructs and reads both fields. The earlier data-variadics and
ordinary packet restrictions are superseded. Quoted-code containers, contextual
data methods, property-call routing and arbitrary expression scheduling are still
separate unfinished work; this change does not make quoted code ordinary data.

Validation from `/home/fin/luna`:

```sh
python3 -B development/check.py --timeout 900 -- test -p luna --test cmtt --test builder_invocations contextual_data
python3 -B development/check.py --timeout 900 -- test -p luna --test cmtt --test cmtt_phase_boundaries --test comptime_memory --test builder_invocations
python3 -B development/check.py --timeout 900 -- run -p luna -- development/examples/sysroot-contextual-data.fl --sysroot library --test-native
python3 -B development/command.py --timeout 120 -- cargo fmt --all
git diff --check
```

All passed: the focused source/builder run (`/tmp/luna-tools-j65zac9j/result.json`),
556 CMTT, 73 builder, three phase-boundary and 17 memory cases
(`/tmp/luna-tools-e000rquv/result.json`), and two LLVM native cases
(`/tmp/luna-tools-n_dlzd3f/result.json`). Seven new source regressions cover empty
and heterogeneous collections, direct named supply, literal/computed packets,
interleaved and empty-packet effects, forwarding, result contracts and rejection
of duplicate/incorrect inputs. Formatting completed at
`/tmp/luna-tools-oq8m9olu/result.json`; whitespace checks passed.


## Property calls for contextual data results

Contextual data calls now recognize extended application at required boundaries,
construction statements and definition checking. Code and data result categories
share the property collector selection in `generators/properties.rs`. This stage
reuses the prepared clause plan when routing the collected record, rather than
preparing explicit clauses twice. Named whole-value supply remains distinct from
colon properties. Calls with only positional extended inputs retain ordinary
routing; record fields may interleave with code arguments and tuple expansions.
Defaults and result contracts continue to run on the completed input frame.

Six source regressions cover inferred anonymous records, equivalent named whole
record supply, interleaved effects, explicit collector selection with tuple
spreads, numeric field context, postcondition rejection, duplicate/ambiguous or
missing collectors, and positional-only extended calls.

Validation from `/home/fin/luna`:

```sh
python3 -B development/check.py --timeout 900 -- test -p luna --test cmtt --test cmtt_phase_boundaries --test comptime_memory --test builder_invocations
python3 -B development/command.py --timeout 120 -- cargo fmt --all
git diff --check
```

All passed: 562 CMTT, 73 builder, three phase-boundary and 17 memory tests
(`/tmp/luna-tools-90ckk1k8/result.json`). Formatting completed at
`/tmp/luna-tools-vjnnpaus/result.json`; whitespace checks passed.

A broader side-effectful explicit Type clause exposed a separate missing path:
`development/repros/contextual-property-live-type-clause.fl` should return true
but fails because required Type evaluation does not receive the generator's live
local frame. It is a valid intended use, not a newly imposed language restriction.
The failed exploratory regression was retained as this independent reproducer;
it is not counted among passing tests. The reproduction command:

```sh
python3 -B development/check.py --timeout 900 -- run -p luna -- development/repros/contextual-property-live-type-clause.fl --interpret run
```

exited 1 at source 534..566 with unavailable comptime input `(local 5 5)`
(`/tmp/luna-tools-m4hvmmfw/result.json`). Repairing Type-input live-frame transport
is the next implementation task. Contextual methods, quoted-code containers and
arbitrary expression scheduling also remain unfinished.


## Required Type computations in live generator frames

The explicit-Type-clause reproducer from the preceding stage is now a passing
regression. Computed Type positions inside a generator data environment reuse
its owned input/update frame, including budget accounting and failure restoration.
Updates are published before the following explicit clauses or argument fields
are evaluated. The property path retains its prepared clause plan; ordinary calls
use the same Type computation path.

Definition checking validates the expression without supplying fake live values.
Already established Type evidence is retained. For an effectful block, a closed
tail Type identity can be used only after checking for earlier control transfers;
the actual block still executes at invocation. Closed required computations retain
their existing required-evaluation behavior, with generator admissibility checked
before invocation. Different input-dependent result identities remain unresolved;
this does not invent a normal-form identity or expand normal-form heads.

The saved source now reads the updated flag in its property field and documents
the expected success. Four source regressions cover ordinary/property calls,
multiple clause updates in source order, unused failing checks versus invoked
failure, invalid unused effects, and rejection of unknown result identity.

Validation from `/home/fin/luna`:

```sh
python3 -B development/check.py --timeout 900 -- test -p luna --test cmtt --test cmtt_phase_boundaries --test comptime_memory --test builder_invocations --test elaboration
python3 -B development/check.py --timeout 900 -- run -p luna -- development/repros/contextual-property-live-type-clause.fl --interpret run
python3 -B development/check.py --timeout 900 -- test -p luna --test cmtt --test cmtt_phase_boundaries --test comptime_memory --test builder_invocations
python3 -B development/command.py --timeout 120 -- cargo fmt --all
git diff --check
```

The broad run passed 566 CMTT, 73 builder, three phase-boundary, 17 memory and 451
elaboration tests (`/tmp/luna-tools-049zlzc3/result.json`). The CLI reproducer
returned `(bool true)` (`/tmp/luna-tools-uia9th8b/result.json`). After moving the
closed-expression admissibility check ahead of execution, the final CMTT/builder/
phase/memory run passed again (`/tmp/luna-tools-aapdrllt/result.json`); the CLI and
elaboration-only evidence preceded that last ordering adjustment. Formatting
completed at `/tmp/luna-tools-gaz7oijr/result.json`; whitespace checks passed.

This repairs live Type-input transport with known result identity. Arbitrary
input-dependent Type results still require a deferred elaboration representation;
contextual data methods, code containers and general expression call scheduling
remain unfinished.


## Contextual data method contracts in NIR

`MethodSignature::DataConsumer` now carries `DataFunctionSignature` without
converting contextual inputs into ordinary callable Terms. It participates in
clause-domain access, type dependency walking and all signature substitution
entrypoints. Its S-expression dump retains the `data-consumer-signature` category,
including quoted expr/pattern interfaces and ordinary result Type.

Trait signature comparison has a data-consumer branch. It checks clause domains,
input categories/context correspondence and output Type through the existing
budgeted contextual comparison, preserving normal-form identity boundaries.
Ordinary program summaries, runtime interpreter dispatch and IIR validation still
reject this signature category. Elaboration's executable-world filter also removes
data-consumer method slots and implementation entries alongside other contextual
members.

This is the representation/validation stage, not source method support. Source
contract formation, method selection, receiver value-reference transport and
invocation dispatch remain to be connected; `ensure_data_consumer` still rejects
methods rather than publishing an incomplete ordinary callable contract.

Validation from `/home/fin/luna`:

```sh
python3 -B development/check.py --timeout 900 -- test -p elaboration --lib program::interfaces::signatures
python3 -B development/check.py --timeout 900 -- test -p nir -p iir -p comptime_interpretor --test code_interfaces --test types --test runtime_function_identities
python3 -B development/check.py --timeout 900 -- test -p luna --test cmtt --test elaboration
python3 -B development/command.py --timeout 120 -- cargo fmt --all
git diff --check
```

All passed: 10 signature comparison tests (`/tmp/luna-tools-331jczy5/result.json`),
36 interpreter identity, nine IIR type, 17 code-interface and 63 NIR type tests
(`/tmp/luna-tools-5xdnsmw6/result.json`), plus 566 CMTT and 451 source elaboration
tests (`/tmp/luna-tools-a1wjfvzz/result.json`). Coverage includes specialization
of the data output, retained code input dependencies, atomic failed transforms,
output/category mismatch, budget failure followed by successful comparison, dump
category preservation and runtime-boundary rejection. Formatting completed at
`/tmp/luna-tools-9wpnnma8/result.json`; whitespace checks passed.

## Source contextual data methods and static inherent invocation

Source trait declarations and implementations now retain contextual data method
signatures, including quoted expression/pattern inputs and data result contracts.
Their early preparation uses the contextual registry rather than demanding an
ordinary callable signature. Declaration-only checking validates defaults and
contracts without inventing or executing a body. Code-producing and data-producing
methods share contract-frame construction; only data results supply an ordinary
result binding for postconditions.

Static inherent calls select the admitted implementation, check access, substitute
container captures into the entire contextual signature, and run the existing
construction interpreter. Named properties, clause inference and nested code
forwarding use the same input preparation as free contextual data functions.
Method classification distinguishes expr, pattern and data outputs, so recognizing
a data method does not misclassify it as a code producer. No ordinary callable
value or invocation-result cache is introduced.

Seven source tests cover compatible and incompatible trait signatures, executed
result/old-input contracts, captured generic container parameters, inferred clauses
and property evaluation, definition-time rejection, receiver/value escape rejection,
and pattern-input category checking. The complete-sysroot example now also builds
`BooleanMethodSlot` via `CodeSchema<bool>.Build`, forwarding its quote to the public
`meta.StructBuilder` construction path and checking the resulting field metadata.

Trait invocation dispatch and receiver value-reference transport remain unfinished.
Both instance syntax and an explicit receiver argument to a static selection are
rejected rather than silently copying the receiver. Arbitrary ordinary expression
scheduling and the existing contextual invocation depth bound are unchanged.

Validation from `/home/fin/luna`:

```sh
python3 -B development/check.py --timeout 900 -- test -p luna --test cmtt --test elaboration --test builder_invocations
python3 -B development/command.py --timeout 120 -- cargo fmt --all
git diff --check
```

All passed: 573 CMTT, 451 source elaboration and 73 builder integration tests
(`/tmp/luna-tools-3fsftcz4/result.json`). The builder group includes the actual
sysroot CLI invocation of the extended example and observes `(bool true)`.
Formatting passed at `/tmp/luna-tools-sg9k2roq/result.json`; whitespace checks passed.

## Contextual data trait invocation

Contextual data calls now use the same trait selection evidence as expr/pattern
code methods. The public interface supplies clause inputs, defaults and contracts;
its completed inputs are mapped by position to the selected implementation's
specialized contextual signature. Shared preparation retains expr/pattern witnesses
without evaluating quoted runtime programs and transports already evaluated data
without replaying argument expressions or implementation defaults.

The public precondition and entry snapshots are established before dispatch. The
selected body runs with its own input frame and contracts; its data result is then
checked against the public output type and public result-bound postconditions.
Definition checking can use symbolic trait evidence without running a body, while
invocation still requires a selected implementation. Default trait bodies and
inherited members follow the existing selection path.

Seven added source regressions cover public defaults versus implementation defaults,
public and implementation postconditions, generic captures and symbolic forwarding,
inherited pattern inputs, missing child evidence, argument evaluation order/count,
value-indexed implementation selection, default bodies and public preconditions.
The sysroot example now constructs `BooleanTraitSlot` through
`CodeSchema<bool>.as(SchemaFactory<bool>).Make` and checks its field names/types.

Validation from `/home/fin/luna`:

```sh
python3 -B development/check.py --timeout 900 -- test -p luna --test cmtt --test elaboration --test builder_invocations
python3 -B development/command.py --timeout 120 -- cargo fmt --all
git diff --check
```

All passed: 580 CMTT, 451 source elaboration and 73 builder integration tests
(`/tmp/luna-tools-k_t_orls/result.json`). The sysroot CLI invocation returned
`(bool true)`. Formatting passed at `/tmp/luna-tools-c3t7om66/result.json` and
whitespace checks passed. Receiver value-reference invocation remains explicitly
unsupported; general ordinary expression scheduling and invocation depth limits
are unchanged. This stage does not introduce code containers or ordinary VM
callables for contextual methods, and does not change invocation result caching.
