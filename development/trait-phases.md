# Trait implementation phases

Semantic authority: [trait comptime phases](../docs/comptime/the-comptime-phases-of-traits.md).
Runtime and comptime implementation contributions belong to the same selected
implementation relationship. Missing phase support must not fall back to a less
specific implementation. Trait member phase and a function's comptime keyword
are distinct decisions; lowering an implementation must not rewrite every member
into a comptime function.

## Current foundation

HIR `ImplDef` retains an explicit `ImplementationPhase` with Runtime and Comptime
variants. Ordinary source implementations and the existing inherent-definition
assembler use Runtime. A `comptime impl` keyword wrapper is consumed during AST
lowering and records Comptime on the implementation rather than leaving the
keyword as an unevaluated generic attribute. HIR S-expression dumps include the
phase. General generated-definition assembly preserves the field supplied by its
caller.

Source `comptime impl Trait for Subject` contributions retain their phase through
interface checking and invocation. Complementary contributions merge
into one implementation descriptor in either declaration order. Inherent
comptime implementations remain unsupported. Shared associated values must
agree before the merged implementation world can be published.

## Member metadata validation

`attrs.comptime_phase` is an ordinary Object constant exported through its
prelude. Attribute inspection evaluates it through the ordinary package and
alias resolution paths. The `attributes::member_phases` consumer accepts merged
metadata and returns source-located `MemberPhase` records. It checks trait or
trait-implementation method/associated-declaration targets, the selected Bool
payload and its original domain. It uses the existing override order, preserves
unrelated keys for other consumers, and charges a shared budget. Failure does
not publish a partial plan or mutate HIR. Records have S-expression Display.

Required attribute evaluation and merge now precede interface construction in
both normal compilation and inspection. Validated member phases populate NIR
method/associated contracts. Normal compilation authorizes only the phase key;
other consumers (including ABI) remain explicitly unsupported. Inspection retains
other keys without authorizing their executable use. Expressions are evaluated
once through ordinary constant/function/name resolution, not recognized by an
attribute function's spelling. Empty metadata avoids an additional world scan.

## Shared relationship index

The implementation index now has `insert_phase` and `select_phase`. Complementary
contributions with identical alpha-bound normalized heads, ordered parameter
domains and interface inputs, and equivalent requirements share one relationship entry. Each
contribution retains its declaration identity. Ordinary `insert` keeps runtime
behavior. Same-phase duplicates still participate in ordinary coherence; open
interface identities cannot be paired. Insertion failure leaves the index and
its revision unchanged; successful additions invalidate older query results.

Phase lookup consumes the common type-resolution result, preserves deferred or
conflicting selection, and reports a missing phase on the selected relationship
without searching less precise candidates. Index and phase-result dumps include
both relationship and contribution identities. Both source phases use this
admission path. Checked contributions merge method tables in trait declaration
order, retaining each body and its original parameter identities. Capability
conditions must have equal alpha-normalized sets; different conditions are
rejected before publishing the merged world. The pairing API compares
applicability conditions by bounded PTC proofs of both
implications, with only the declaration binder scope and no caller assumptions.
This accepts reordered/grouped conditions, redundant truths and equivalent
logical forms without asserting that either condition holds at a call site.
Head identity remains structural: this does not unfold normal-form bodies or
infer equality from their implementations. Exhaustion or an undetermined proof
leaves the index unchanged rather than publishing a separate competing relation.

## Checked IR and VM membership

NIR member contracts now retain a `TraitPhase` independently of callable
execution signatures. An implementation records its available runtime/comptime
contributions with `ImplementationPhases`; dumps include both kinds of metadata.
Source construction uses selected attribute phases and the implementation keyword.
Absent member metadata retains the runtime default for trait declarations.

NIR phase validation rejects an empty contribution set, unsupported inherent
comptime contributions, and methods supplied for an unavailable phase. VM world
admission requires methods only from available phases and rejects supplied
methods from unavailable phases. Missing phases cannot recover a trait default.
Explicit dispatch also checks member phase availability. Comptime-associated
values require the comptime contribution; readonly runtime associated data can
remain shared by a comptime contribution. Function execution phase validation
continues independently. Source implementation completeness checks only the
selected phase, and explicit method projections diagnose a missing phase on the
selected relationship. Single comptime contributions can execute ordinary
comptime methods, including generic owners and available trait defaults.

For complementary methods with renamed implementation inputs, VM admission
checks alpha-equivalence of the input-domain telescopes before transporting
positional arguments. Type and comptime-value domains remain distinct; free
identities and opaque heads are preserved. This bounded check does not replace
head admission, method signature checks or invocation contracts.

## Associated source contributions

A runtime contribution checks runtime associated members and can omit comptime
members even when they have defaults. A comptime contribution checks comptime
members and readonly runtime signature data. Explicit member phase annotations
must agree with the trait contract. Missing required initializers and wrong value
types retain their existing diagnostics.

Selected associated-value lookup checks comptime membership before consulting
completed values or initializers. A missing phase cannot recover a default or a
less precise relationship. Deferred method-signature checks build their Type
world only from available initializers rather than indexing absent phase members.
Complementary associated initializers retain their original owner and a
substitution into the common relationship's binder environment. Required readonly
runtime data can be supplied by the runtime contribution without repeating it in
the comptime contribution. Defaults and explicitly repeated shared values are
checked independently. The second initializer cannot be discarded during merge.

After ordinary associated completion, alternate initializers are checked in the
same environment. Selected recipes and values are restored even on failure;
failed checks never publish a package. Completed scalar results and Type values
are compared structurally with diagnostic locations erased. Generic initializer
programs currently require matching checked structure after substitution; a
structurally different program produces an explicit unestablished-equality
error, even when a future verifier might prove equivalence. This comparison does
not use PTC for program equality or unfold opaque normal-form heads.

Lexical associated references from secondary contributions resolve through the
common relationship and substitute its inputs back into the method's original
environment. Both source declaration orders follow the same path.

## Comptime query keys

The Eq/Hash key selector requires comptime trait membership independently of the
function's comptime execution signature. After ordinary relationship resolution
and capability qualification, it checks the selected implementation's comptime
availability before obtaining a method body or default. A more precise runtime
relationship blocks a broader comptime implementation; no phase-filtered retry
occurs. Existing signature, domain, type requirement and program contract checks
remain in force. Runtime comparison protocol selection is unchanged.

## Generated HIR publication

`DefinitionBatch` publication uses ordinary elaboration on a private candidate.
Generated implementation phases and structured member configuration therefore
follow the same phase admission and complementary merging paths as source
syntax. This is distinct from the VM-facing concrete definition publisher,
which currently publishes struct/enum descriptors, not arbitrary trait bodies.
The HIR regression checks rejection without publication, repair of the retained
draft, merged availability and actual invocation. Membership does not rewrite
function execution signatures.

## Dynamic trait-member packets

NIR's internal `ComptimeFunctionInstance` can name a trait member declaration.
Its packet orders Self, interface inputs, then method inputs. The VM evaluates
and freezes this packet before rebuilding a `TraitMethod`; direct calls then use
the existing interface parameter bindings, selection, phase checks and program
contracts. A method with interface preconditions cannot escape as a plain
callable value. Packet shape, domains, ambiguous member identities and comptime
execution are checked before dispatch; lookup and freezing consume VM budget.
The existing S-expression dump retains the declaration and packet explicitly.

Source elaboration now emits these packets for computed value clauses whose
callable signature is fixed, retaining known Self and interface inputs. Required
invocation expands the member's dependencies, and dispatch preparation selects
the implementation before transporting method inputs to the VM. IIR instance
preparation still needs further integration. For direct VM calls, interface default
expressions are now specialized with the frozen Self/interface/member inputs.
They retain interface-side local bindings, including references to earlier
ordinary arguments; implementation-body binding identities are not substituted
for those locals. Explicit arguments still precede default evaluation.

## Remaining integration

- Interleave metadata queries that themselves depend on interface preparation;
  the current early required-evaluation path supports independent computations.
- Extend shared generic associated-program equality beyond identical checked
  structure using program-level evidence; retain bounded conservative rejection
  when equality cannot be established.
- Extend VM-facing builders beyond their current nominal descriptors and
  finish remaining protocol consumers. Builtin Eq now supplies primitive
  comptime comparisons alongside runtime comparisons. Hash supplies Bool and
  fixed-width integer implementations in both phases; target-sized integers,
  other scalar/aggregate domains and derivation remain unfinished.
- Extend dynamic trait clauses beyond fixed callable signatures and known
  Self/interface inputs. Completed IIR instances currently reject unevaluated
  packets explicitly; source comptime invocation retains origin, specialized
  defaults and program checks.

A runtime entry can already consume the result of a fixed-signature dynamic
trait computation through a bound `Splice` protocol. Required invocation freezes
the comptime wrapper and member calculation before IIR instantiation, leaving
runtime conversion semantics intact. This does not authorize runtime inputs to
the comptime wrapper or permit an unevaluated dynamic packet in IIR.

The existing dynamic inherent-method implementation is not evidence that trait
phase selection or dynamic trait-method clauses are complete.

## Validation

In `/home/fin/luna`,
`python3 -B development/check.py --timeout 900 -- test --release -p luna -p hir
-p elaboration --test trait_phases --test display --test definitions
--test definition_interfaces --test generated_traits` passed 51 tests
(`/tmp/luna-tools-xujgg3q8/result.json`, exit 0). Source tests check both phase dumps,
independent member modifiers, explicit refusal of unfinished phase selection, and
ordinary implementation elaboration. Existing generated traits and definitions
remain accepted. The generated-implementation test was then extended to all four
trait/inherent and runtime/comptime combinations; its suite passed separately
(`/tmp/luna-tools-n4fvchr3/result.json`). Workspace formatting passed
(`/tmp/luna-tools-wk9lspyk/result.json`).

Member metadata validation passed 20 tests with
`python3 -B development/check.py --timeout 900 -- test --release -p luna
-p elaboration --test attribute_phases --test attribute_merge
--test library_attributes --test trait_phases`
(`/tmp/luna-tools-69fwiv54/result.json`, exit 0). These cover selected overrides,
target and Bool-domain rejection, budget failure/retry, unrelated metadata,
real attrs package/prelude resolution, and the existing implementation-phase
boundary. They do not demonstrate executable two-phase selection.

Shared relationship indexing passed 85 tests (73 existing implementation tests
and 12 phase tests) with
`python3 -B development/check.py --timeout 900 -- test --release -p elaboration
--test implementation_phases --test implementation`
(`/tmp/luna-tools-9z5hynz0/result.json`, exit 0). Checks cover complementary
generic heads in both insertion orders, mixed-sort domains, free identities,
opaque heads, missing-phase non-fallback, deferred selection, coherence conflicts,
open interfaces, stale queries and atomic budget retry. The preceding run
(`/tmp/luna-tools-r1hy0go2/result.json`, exit 101) reproduced unbound requirement
capture against the library compiled before the scope fix; the final run includes
that fix and passes its rejection regression. Workspace formatting passed
(`/tmp/luna-tools-l2puwhb9/result.json`); `git diff --check` also passed.

Condition-equivalence integration passed 90 tests (73 implementation and 17 phase
tests) with the same targeted release command above
(`/tmp/luna-tools-48ase5hb/result.json`, exit 0). Added checks distinguish
bidirectional implication, one-way implication with a validated countermodel,
and unvalidated type countermodels. The latter preserve the previous index and
return an undetermined result. The earlier run
(`/tmp/luna-tools-hzisjmlj/result.json`, exit 101) revealed two test expectations
that incorrectly treated unvalidated countermodels as proved inequivalence;
tests now explicitly require conservative rejection and unchanged query state.
Budget exhaustion is checked across insufficient allowances and successful retry.
Workspace format check passed (`/tmp/luna-tools-xrsyzwq0/result.json`).

NIR/VM membership integration passed 250 tests in `/home/fin/luna` with
`python3 -B development/check.py --timeout 900 -- test --release -p nir
-p comptime_interpretor -p elaboration -p iir -p luna
--test runtime_function_identities --test calls --test code_interfaces
--test nominal_parameters --test associated --test capabilities --test types
--test trait_phases`
(`/tmp/luna-tools-h2n031k3/result.json`, exit 0). Tests include actual comptime
method invocation under each membership phase, runtime identity transport,
missing-phase/default rejection, phase-specific completeness, associated value
availability and shared runtime data. Existing source phase rejection remains
tested. The first run (`/tmp/luna-tools-qxodd4dl/result.json`, exit 101) failed
the new NIR membership rejection assertion against the earlier compiled library;
the final run rebuilt the completed validator and passed. Workspace formatting
passed (`/tmp/luna-tools-dh3zh63p/result.json`), as did `git diff --check`.

Source phase application passed 34 tests with
`python3 -B development/check.py --timeout 900 -- test --release -p luna
-p elaboration --test trait_phases --test library_attributes
--test attribute_phases --test attribute_merge --test generated_traits`
(`/tmp/luna-tools-93jnfvdj/result.json`, exit 0). The 14 source cases include
actual invocation, generic owners, available defaults, computed metadata and
override order, opposite-phase rejection, matching inspection behavior, and
non-fallback from a more precise runtime relationship. The initial run
(`/tmp/luna-tools-pwd24e7x/result.json`, exit 101) exposed fixture syntax issues
and inspection's late phase preparation. A subsequent run
(`/tmp/luna-tools-mwr791o9/result.json`, exit 101) caught a raw-inspection
regression: inspecting an Object must not require it to be a valid merge input.
Normal compilation remains strict; raw inspection can retain the original
Objects without applying an invalid phase plan. The final run includes this
repair and the existing failed-output preservation regression. Formatting passed
(`/tmp/luna-tools-ck12hcrm/result.json`); `git diff --check` passed as well.

Complementary source method merging passed 209 tests in `/home/fin/luna` with
`python3 -B development/check.py --timeout 900 -- test --release -p nir
-p comptime_interpretor -p luna -p elaboration --test trait_phases
--test runtime_function_identities --test implementation_phases
--test implementation --test generated_traits --test library_attributes
--test capabilities --test associated`
(`/tmp/luna-tools-8q6b86s5/result.json`, exit 0). Source tests execute both
contribution orders, renamed generic inputs with parameterized trait instances,
Bool value inputs, and matching alpha-renamed capability conditions; differing
capability conditions are rejected. NIR telescope checks cover dependent value
domains, free identities, missing metadata, malformed scope/domain and budget
failure followed by successful retry. Existing method/default availability,
coherence, capabilities and associated-value suites remain passing. Workspace
format check passed (`/tmp/luna-tools-lfr1cwuv/result.json`, exit 0), as did
`git diff --check`. Cross-phase associated source checking remains unsupported.

Phase-specific associated source checking passed 81 tests in `/home/fin/luna`
with `python3 -B development/check.py --timeout 900 -- test --release -p luna
-p elaboration -p comptime_interpretor --test trait_phases --test associated
--test runtime_function_identities --test library_attributes --test generated_traits`
(`/tmp/luna-tools-zc49lzr9/result.json`, exit 0). Added source cases cover
shared readonly Type data with comptime scalar members, renamed generic Type
initializers, omission of absent-phase members/defaults, wrong-phase definitions,
missing required values, conflicting explicit annotations, non-fallback, and
explicit rejection of unfinished complementary associated merging. The first run
(`/tmp/luna-tools-19q5wydh/result.json`, exit 101) found missing `Type` declarations
in three fixtures; the no-sysroot helper now declares the builtin alias.
Formatting passed (`/tmp/luna-tools-guo3xq_8/result.json`, exit 0), as did
`git diff --check`. Shared associated-value merging remains the next integration
step; passing single-contribution tests does not establish cross-phase equality.

Complementary associated merging passed 85 targeted release tests in
`/home/fin/luna` using the preceding phase-specific command
(`/tmp/luna-tools-ln8dvaah/result.json`, exit 0). Source cases cover both
contribution orders, required shared types supplied only by the runtime part,
renamed generic Type and Bool inputs, lexical references from both parts,
computed scalar equality and conflicting Type/scalar results. The initial
compile (`/tmp/luna-tools-sm56x07l/result.json`, exit 101) caught a diagnostic
conversion omission. The next run (`/tmp/luna-tools-vebxed8d/result.json`,
exit 101) exposed premature signature failure when a comptime method depended on
required data supplied only by its runtime complement. Missing local equations
now remain unresolved until the merged Type world checks the deferred signature;
independent completeness checks still reject absent required data. The final
run includes this repair. Formatting passed
(`/tmp/luna-tools-ozbqndtr/result.json`, exit 0), as did `git diff --check`.

The existing source associated-value regressions additionally passed 114 tests
with `python3 -B development/check.py --timeout 900 -- test --release -p luna
-p elaboration -p comptime_interpretor --test execution associated`
(`/tmp/luna-tools-4qoapkof/result.json`, exit 0; 1,247 unrelated cases filtered).
These exercise ordinary defaults, selected overrides, cross-implementation
and lexical dependencies, generic value initialization, callable values and
associated signatures. Combined with the phase suites, 199 tests passed.

Comptime key protocol phase enforcement passed 1,654 tests in `/home/fin/luna`
with `python3 -B development/check.py --timeout 900 -- test --release -p luna
--test execution --test nominal_value_parameters --test comptime_memory
--test equality_protocol --test ordering_protocol`
(`/tmp/luna-tools-g25hwz_6/result.json`, exit 0). All 1,363 execution tests ran
because the cache-key fixture migration affects many invocation paths. The other
291 tests cover nominal value parameters, memory-backed keys and ordinary
runtime comparisons. Added key-selection cases reject runtime membership even
for comptime functions, unavailable phase defaults, and fallback from a more
precise runtime relationship. Positive generic/default selections, memory key
obligations, hashing and memoization continue to pass with explicit comptime
phase declarations. Formatting passed (`/tmp/luna-tools-z81ayc0m/result.json`,
exit 0); `git diff --check` passed. This does not complete builtin Eq/Hash's
library-level two-phase interface design or migrate its runtime declarations.

Generated HIR phase publication passed 18 tests in `/home/fin/luna` with
`python3 -B development/check.py --timeout 900 -- test --release -p elaboration
--test generated_traits --test definitions --test attribute_phases
--test attribute_merge`
(`/tmp/luna-tools-bns70dez/result.json`, exit 0). The generated-trait test now
runs both ordinary and comptime membership, the latter with a runtime complement.
It checks failed signature admission leaves the draft and published metadata
unchanged, repair preserves phase metadata, one merged implementation is
published, and invocation returns the expected value. Function execution phases
remain runtime in both cases, explicitly testing their independence from member
membership. The initial run (`/tmp/luna-tools-3hakswtn/result.json`, exit 101)
and diagnostic rerun (`/tmp/luna-tools-_4uyv2xu/result.json`, exit 101) exposed an
obsolete blanket unsupported-attribute assertion. It now requires the actual
Object-domain mismatch or unsupported named consumer diagnostic while preserving
all transactional assertions. No production behavior was changed in this step.

Dynamic trait-member packet interpretation passed 100 targeted tests in
`/home/fin/luna` with `python3 -B development/check.py --timeout 900 -- test
--release -p nir -p comptime_interpretor --test runtime_function_identities
--test calls --test types`
(`/tmp/luna-tools-g0fd5my0/result.json`, exit 0). Raw checked NIR tests exercise
invocation with multiple method Type inputs, interface pre/postcondition failures,
rejection of contracted callable escape, missing comptime phases, malformed
packet arity, and a failed type requirement followed by a successful invocation.
The initial run (`/tmp/luna-tools-f1k849_z/result.json`, exit 0) passed before the
last two boundary cases were added. Existing function-call and type tests also
pass. This is VM-level evidence, not source dynamic-trait-clause or IIR support.

Dynamic trait default specialization passed 104 tests in `/home/fin/luna` with
`python3 -B development/check.py --timeout 900 -- test --release -p nir
-p comptime_interpretor --test runtime_function_identities --test calls --test types`
(`/tmp/luna-tools-0uxzcarx/result.json`, exit 0). Added cases pass different Type
and Bool clause inputs across repeated calls, reference preceding interface
arguments from later defaults, override a default explicitly, and reject
forward reads or incorrect interface binding maps. The initial 103-test run
(`/tmp/luna-tools-fyv40nge/result.json`, exit 0) preceded the Bool-input case.
These remain checked-NIR VM tests; source and IIR integration are not established.

Source dynamic trait-clause integration now passes the eight cases in
`compiler/luna/tests/dynamic_trait_clauses.rs`: repeated value inputs, explicit
default overrides, rejection of an unproven interface postcondition, named
optional clauses with a parameterized postcondition, required constant
invocation, generic implementation selection, a trait default body, and
exactly-once clause evaluation before explicit arguments. Required dependency
collection expands member declarations, including defaults whose body shares
the member identity. Dispatch preparation keeps symbolic method inputs until
invocation. Contract inheritance now preserves value-input sorts and translates
canonical implementation inputs into each body's own generic environment.

Validation in `/home/fin/luna`:

- `python3 -B development/check.py --timeout 900 -- test --release -p luna
  -p elaboration -p comptime_interpretor --test dynamic_trait_clauses
  --test trait_phases --test runtime_function_identities`: 70 tests passed
  (`/tmp/luna-tools-ugvja28i/result.json`, exit 0), before adding the default-body
  case. Earlier failures exposed a bypass in the iterative materialization
  walker and Type-only substitution in program contracts; both are repaired.
- `python3 -B development/check.py --timeout 900 -- test --release -p luna
  -p elaboration -p comptime_interpretor --test dynamic_trait_clauses`: all eight
  source cases passed (`/tmp/luna-tools-ck589mx5/result.json`, exit 0).
- `python3 -B development/check.py --timeout 900 -- test --release -p luna
  -p elaboration -p comptime_interpretor --test execution`: existing execution
  regressions passed (`/tmp/luna-tools-ehpyro1v/result.json`, exit 0).
- `python3 -B development/command.py --timeout 120 -- cargo fmt --all -- --check`
  passed (`/tmp/luna-tools-dqsokl5z/result.json`, exit 0); `git diff --check` passed.

This completes the fixed-signature source-to-VM path, not value-dependent
callable signatures or completed IIR instances containing unevaluated packets.

Nested dynamic calls exposed a required-dependency classification bug: a second
reference to an already expanded bodyless member entered ordinary function
checking and failed with `required comptime call needs a function body`.
Member identities now remain contract references after expansion; only a
default body sharing that identity is eligible for ordinary body checking.
The regression invokes two implementations of the same member in sequence,
transports both Bool values, and evaluates the result in a required constant.

In `/home/fin/luna`, `python3 -B development/check.py --timeout 900 -- test
--release -p luna -p elaboration -p comptime_interpretor --test
dynamic_trait_clauses --test trait_phases` passed all 38 tests
(`/tmp/luna-tools-w_9grso7/result.json`, exit 0). The regression failed before
the repair (`/tmp/luna-tools-8aa_4c5o/result.json`, exit 101).
`python3 -B development/command.py --timeout 120 -- cargo fmt --all -- --check`
passed (`/tmp/luna-tools-y6vn_drl/result.json`, exit 0).
The related source regressions also passed: `python3 -B development/check.py
--timeout 900 -- test --release -p luna -p elaboration -p comptime_interpretor
--test execution trait`, 94 tests passed, 1,269 unrelated tests filtered
(`/tmp/luna-tools-3x69y3nz/result.json`, exit 0). `git diff --check` passed.

Dynamic packet origin lookup now uses an index built alongside VM method
contracts. Previously every packet scanned all contracts, even when it named
an ordinary function. Initialization records ambiguous cross-interface member
identities explicitly, preserving `InvalidCallPlan` at use. Lookup now consumes
one budget step and performs a hash lookup instead of charging and scanning the
entire member catalog. The index belongs to the immutable interpreter world;
IIR-only interpreters initialize an empty index.

A regression compares invocation work after construction for the same three
calls with zero or 128 unrelated additional members. Both consume equal steps;
this is an operation-budget scaling check, not a wall-clock speedup claim.
An ambiguous-origin regression also passes. In `/home/fin/luna`, validation used
`python3 -B development/check.py --timeout 900 -- test --release -p nir
-p comptime_interpretor --test runtime_function_identities --test instance_budget
--test calls` (`/tmp/luna-tools-71o828n3/result.json`, exit 0).
The initial compile caught the missing IIR constructor field, now initialized;
an initial broader command omitted the `nir` package owning `calls` and did not
run tests. Formatting check passed with `python3 -B development/command.py
--timeout 120 -- cargo fmt --all -- --check`
(`/tmp/luna-tools-yq2mer5a/result.json`, exit 0); `git diff --check` passed.

The runtime handoff is now covered by a source-to-IIR regression with an explicit
ordinary `Splice.splice(comptime data:Self)` implementation. It checks the IIR
contains only runtime functions and no `choose`/`use_it` comptime bodies, executes
the same entry with both Bool runtime inputs, and rejects forwarding a runtime
input into the comptime wrapper. This path already worked; no production change
was needed. All ten dynamic-trait source tests passed in `/home/fin/luna` with
`python3 -B development/check.py --timeout 900 -- test --release -p luna
-p elaboration -p comptime_interpretor --test dynamic_trait_clauses`
(`/tmp/luna-tools-inirtfel/result.json`, exit 0). This is evidence for completed
invocation before IIR, not for retaining dynamic packets inside IIR.

Builtin comparison now declares `comptime_eq` and its `comptime_ne` default
separately from runtime `eq`/`ne`. All existing primitive Eq subjects supply the
comptime contribution as well. Hash declares `comptime_hash` alongside `hash`;
concrete Hash implementations remain future library work. Key selection prefers
the explicit comptime name and selects its implementation body by the selected
member name. Compact comptime-only protocols using `eq`/`hash` remain supported
when no explicit comptime name is declared. An explicit name is authoritative:
invalid phase/signature or duplicates do not trigger another-member fallback.

Validation in `/home/fin/luna`, all commands use
`python3 -B development/check.py --timeout 900 -- test --release -p luna
-p elaboration -p comptime_interpretor` followed by:

- `--test equality_protocol --test ordering_protocol`: 9 passed
  (`/tmp/luna-tools-costpxwq/result.json`, exit 0).
- `--test library_comptime_comparison`: 3 passed, including Bool key comparison,
  distinct comptime hashes, duplicate/phase rejection, and all twelve integer
  domains on both 32-bit and 64-bit targets
  (`/tmp/luna-tools-s2rqlftz/result.json`, exit 0). The initial run rejected
  target-sized literals without explicit target data; the fixture now supplies
  the target rather than relying on the host.
- `--test execution comptime_key`: 9 passed, 1,354 unrelated tests filtered
  (`/tmp/luna-tools-zrwxyb5k/result.json`, exit 0).
- `--test frontend_cli shipped_clone_protocol_preserves_noncopy_sources`:
  passed with the shipped traits/comparison files
  (`/tmp/luna-tools-_d6ut_n_/result.json`, exit 0).

`python3 -B development/command.py --timeout 120 -- cargo fmt --all -- --check`
passed (`/tmp/luna-tools-y0o__o5n/result.json`, exit 0); `git diff --check` passed.

Builtin Hash now implements both phases for Bool and the ten fixed-width integer
types. Bool maps to 0/1; integer representations up to 64 bits widen with zero
extension, including the representation bits of signed values. A 128-bit value
xors its two 64-bit halves using BitVec operations. These are deterministic
non-cryptographic scalar hashes; different values may collide and still require
Eq. The implementation does not introduce implicit numeric casts, depend on
LLVM-only math intrinsics during comptime execution, or truncate away the high
half of a wide integer.

In `/home/fin/luna`, `python3 -B development/check.py --timeout 900 -- test
--release -p luna -p elaboration -p comptime_interpretor --test library_hash
--test library_comptime_comparison` passed six tests
(`/tmp/luna-tools-2vg4ouxw/result.json`, exit 0). Hash cases execute fourteen
scalar boundary inputs in both phases, including signed minima and 128-bit high
halves. A deliberate collision is rejected by Eq, and LLVM O0/O3 native tests
verify representative hashes. The shipped-file CLI regression also passed with
`--test frontend_cli shipped_clone_protocol_preserves_noncopy_sources`
(`/tmp/luna-tools-nh4x4v8j/result.json`, exit 0). Initial tests exposed that Bool
has no supported bit view (now mapped explicitly) and assumes cannot contain
ordinary calls (the test binds results before its predicates).

Formatting check passed via `python3 -B development/command.py --timeout 120 --
cargo fmt --all -- --check` (`/tmp/luna-tools-ex7mg0ab/result.json`, exit 0).
`git diff --check` passed. Target-sized integers, characters, symbols, compound
keys and derivation are not covered by these new Hash implementations.
