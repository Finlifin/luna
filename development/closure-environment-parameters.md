# Open closure environment implementation checkpoint

Contract: `docs/type/qualified-types/closure-type.md` and
`docs/functions/higher-order-functions.md`. `%c F` must preserve the actual
environment; `c: Type.vars.Closure` may instantiate to an empty context. It must
not erase captures, allocate an implicit box, or identify ordinary captured
values with their types. Ordinary generic callback signatures now work from
source through IIR execution, including returned callbacks. Clause-bound context
type expressions also complete during comptime invocation. General computed
context expressions and dependent capture sources remain incomplete.

## Observed implementation boundary

- NIR now has `ClosureContextType`, `ClosureContextValue`, and
  `ContextualClosure`. `Term::contextual_closure` completes a concrete context
  to the existing `Closure` representation or eliminates an empty context.
  Binder substitution, runtime identity traversal, scope checking, callable
  helpers, type-copy budgets and S-expression Display cover these nodes.
  Unresolved contextual qualifiers cannot inhabit executable runtime storage.
  `compiler/nir/tests/closure_contexts.rs` covers these representation contracts.
  Source domain recognition follows the resolved Type identity, including aliases.
- Candidate matching now extracts empty or captured contexts from formed
  callable types, forwards caller context binders without scope escape, and
  checks shared environment/phase obligations against the original types.
  `CallableContext.open_environment` distinguishes an unknown extent from an
  ordinary environment-free function. Partial fixed capture fields survive
  result-context propagation. `compiler/elaboration/tests/calls.rs` covers
  inference, higher-order forwarding, rejection and candidate recovery.
- `nir::Term::Closure` stores a fixed `Vec<Term>` of capture types. Its scope
  checking and rewriting traverse those fields, not an environment variable.
- Type formation accepts clause parameters declared in `Type.vars.Closure`.
  Expression formation preserves clause-bound qualifiers as symbolic Type values
  until specialization. Dynamic context construction still requires further IR
  and interpreter work; this is not implicit runtime storage for Type values.
- `calls.rs` retains fixed optional environment-field types when available;
  otherwise its explicit open-environment marker keeps capture inference pending.
- Generic storage checks retain scoped context binders and verify rigid binder
  domains against declaration metadata. Concrete substitution exposes capture
  storage to the existing phase checks; arbitrary unregistered contexts fail.
- `compiler/luna/tests/closure_environment_parameters.rs` exercises the existing
  generic fixed-field environment path, plus open empty/captured contexts,
  generic forwarding through IIR, Type aliases and wrong-domain rejection.
- Interpreter declaration input admission now validates context values against
  their own domain. Capture fields participate in bounded recursive type
  inspection and selected associated-type normalization.
- Resource rejection reachability substitutes mixed-sort declaration inputs
  using package metadata instead of assuming every input is a Type. This fixes
  rejection-summary formation for callbacks with owned captures. The summary
  cache keeps concrete environment inputs in its instance identity; failed
  input validation does not publish a result.
- NIR callable-result propagation, callback-memory import and named-callable
  validation also use declaration-derived binder sorts. Source tests check the
  generic NIR directly in addition to concrete IIR execution, including pointer
  mutation and rejection of a returned closure pointing into an expired scope.

## Implementation direction

Use a distinct context value and a symbolic contextual type constructor, with
context parameters in the comptime-value domain. Do not encode a context as a
tuple Type, effect row, or arbitrary list of Types in source semantics. An
ordered context must retain binding dependencies and external identities.

1. Add the context-value/domain representation and symbolic qualifier to NIR.
   Implement scope validation, capture-avoiding abstraction/substitution, all
   structural traversals, and stable S-expression Display together. A completed
   context lowers to the existing closed closure representation; an empty
   context eliminates the qualifier. Structural completion must never unfold
   a normal-form function head.
2. Extend candidate-local inference: comparing `%c F` with a formed closure
   supplies its context as evidence for c; comparing it with an ordinary
   callable supplies the empty context. Fixed contexts compare in dependency
   order. Do not infer an environment by scanning source variable names or
   inventing captures before checking the closure.
3. Register `Type.vars.Closure` as the context domain and admit its clause
   binders in both ordinary and open signatures. Keep domain validation
   separate from generic `Sort::ComptimeValue`: arbitrary comptime values are
   not closure contexts. Preserve lexical resolution and Type identity.
4. Thread partial environment information through callback contexts, type
   aliases, result contexts and staged descriptions. Completed runtime
   materialization must contain no unresolved context variable. Continue to
   deliver the actual capture storage through the existing closure ABI.
5. Verify source-to-IIR calls with empty, scalar, pointer and owning captures;
   recursive forwarding and returned closures; independent instantiations;
   phase mismatches; cleanup and repeated-consumption rejection. Run focused
   NIR/inference tests before the affected closure/resource integration groups.

The context representation and inference are prerequisites, not evidence that
the source feature works. Fixed-field tests cannot certify arbitrary `%c` or
dependent environment sources.

## Source integration verification (2026-10-21)

Working directory: `/home/fin/luna`. All commands use
`python3 -B development/check.py --timeout 900 --`.

- `test -p luna --test closure_environment_parameters --test contextual_spreads`:
  22 passed, exit 0; `/tmp/luna-tools-qgdbh9gb/result.json`.
- `test -p nir --test closure_contexts --test nominal_parameters --test phases`:
  42 passed, exit 0; `/tmp/luna-tools-swllss_u/result.json`.
- `test -p elaboration --test calls --test associated`:
  53 passed, exit 0; `/tmp/luna-tools-hvjjwo60/result.json`.
- Workspace formatting completed through `development/command.py`;
  `git diff --check` passed. Changes remain uncommitted on `dev`.

Follow-up: clause-context Type expressions and returned callbacks (same wrappers
and working directory):

- `test -p luna --test closure_environment_parameters --test resources`:
  354 passed, exit 0; `/tmp/luna-tools-_0oaz8qq/result.json`.
- `test -p nir --test closure_contexts --test nominal_parameters`:
  27 passed, exit 0; `/tmp/luna-tools-6r3pb2j2/result.json`.
- `test -p comptime_interpretor --lib`: 66 passed, exit 0;
  `/tmp/luna-tools-tikcq_cm/result.json`.
- Workspace formatting and `git diff --check` passed; no commit created.

Effect-memory follow-up: operation-label substitution now uses the enclosing
function's mixed-sort declaration binders, preserving exact heads rather than
memory layout views. Generic requests and generic handler installers carrying
`echo<Flag<flag>>` now analyze directly from NIR and execute for both Boolean
indices. A mismatched handler label remains rejected. This does not add direct
value parameters to effect declarations themselves (subsequently added below).

Verification in `/home/fin/luna` through `development/check.py --timeout 900`:
`test -p luna --test nominal_value_parameters --test effect_spreads` passed 262
tests (`/tmp/luna-tools-51ue3p5c/result.json`); `test -p luna --test execution effect`
passed 72 (`/tmp/luna-tools-657u3f2y/result.json`), both exit 0. Formatting and
`git diff --check` passed; changes remain uncommitted.

Resource follow-up (same wrappers and working directory):

- `test -p luna --test closure_environment_parameters --test match_cleanup`:
  23 passed, exit 0; `/tmp/luna-tools-0et21i0s/result.json`. Includes pointer
  identity/mutation and resource-checked single versus repeated consumption.
- `test -p elaboration --lib verification::rejection`: 12 passed, exit 0;
  `/tmp/luna-tools-irudalo_/result.json`. Includes mixed-sort contexts, failed
  substitution recovery, cache separation, recursion and budget exhaustion.
- Formatting and `git diff --check` passed; changes remain uncommitted.

Generic NIR resource follow-up (same wrappers and working directory):

- `test -p luna --test closure_environment_parameters --test match_cleanup`:
  24 passed, exit 0; `/tmp/luna-tools-dfiz6prv/result.json`.
- `test -p elaboration --lib verification`: 48 passed, exit 0;
  `/tmp/luna-tools-rz5x00ct/result.json`.
- Formatting and `git diff --check` passed; no commit created.

Mixed-sort resource audit also repaired `Drop` selection and access: method
signatures, type requirements, program contracts and capabilities use declared
binder sorts; nominal field projection uses the nominal declaration's binders.
`Flag<tag: bool>` destructors now retain separate true/false behavior.

Remaining specification boundary: `verification/drop/plan.rs` rejects nonempty
Drop pre/postconditions with `cleanup program specification obligations are not
discharged`. `value_indexed_drop_methods_preserve_selected_arguments` checks
successful cleanup without such conditions and preservation of this rejection
when a postcondition is present. Pointer-write postconditions also currently
need explicit checked premises during frontend validation; this audit does not
claim their inference or cleanup discharge is implemented.

Verification: `/home/fin/luna`,
`python3 -B development/check.py --timeout 900 -- test -p luna --test nominal_value_parameters --test drop`:
1009 passed, exit 0; `/tmp/luna-tools-w4dziobf/result.json`.
Workspace formatting and `git diff --check` passed; no commit created.

The mixed-sort audit also covered comptime key protocols and memoization.
`Package::declaration_binders` recovers bounded declaration metadata without
inferring sorts from supplied values. Eq/Hash method signatures and obligations,
memoized signatures and requirements, and staged invocation templates now use
these binders. Source coverage includes value-indexed nominal keys, deliberate
hash collisions, generic protocol dispatch, ordinary/staged invocation, result
reuse and rejection of a key with different nominal arguments without publishing
a new cache entry.

Verification in `/home/fin/luna` through `development/check.py --timeout 900`:

- `test -p luna --test nominal_value_parameters`: 243 passed, exit 0;
  `/tmp/luna-tools-15uq3uua/result.json`.
- `test -p luna --test execution -- memoized comptime_key`: 17 passed, exit 0;
  `/tmp/luna-tools-xh3cuzj5/result.json`.
- Workspace formatting and `git diff --check` passed; no commit created.


## Direct effect value parameters

Effect declarations now retain parameter domains in their S-expression dump.
Instance formation checks sorts, substituted dependent domains and scope with
bounded work; symbolic values require independent declaration-domain evidence.
Standalone source operations accept scalar value arguments, preserve them through
generic requests/installers, and retain distinct handler identities in IIR.
Comptime immutable-value admission checks the domains even when the label input
is unused in the operation signature. Documentation includes a Boolean example.

At that checkpoint, effect-group value arguments and nominal-object inputs were
still incomplete. Nominal-object layout evidence is now connected as described
below; effect-group value arguments remain outstanding.

Verification, `/home/fin/luna`, using `development/check.py --timeout 900`:

- `test -p elaboration --test effects --test effect_requests`: 19 passed,
  `/tmp/luna-tools-p9j9g_s2/result.json`.
- `test -p luna --test nominal_value_parameters --test effect_spreads`: 265 passed,
  `/tmp/luna-tools-2ay677g6/result.json`.
- `test -p luna --test execution effect`: 72 passed,
  `/tmp/luna-tools-qbcorv2s/result.json`.
- `test -p iir`: 63 passed, `/tmp/luna-tools-q6l48c5e/result.json`.
- `test -p nir --test effects --test nominal_parameters`: 18 passed,
  `/tmp/luna-tools-leegu8ua/result.json`.

All exited 0; workspace formatting and `git diff --check` passed. Uncommitted on dev.


## Nominal effect input layouts

Effect formation now accepts explicit layout evidence from checked declarations.
The Checker borrows its struct/enum/newtype metadata through a bounded lookup;
NIR shares the same recursive admission and field-instantiation algorithm with
package-backed callers. No package clone or body unfolding supplies field types.
Lookup results must match the requested declaration identity. Nested nominal
payloads and their parameter domains share the caller's budget.

Source tests cover direct struct keys plus generic forwarding of nested structs,
enum variants and newtypes, actual execution, memory inference and mismatched
handler rejection. Effect-group value inputs were subsequently connected below.

Verification in `/home/fin/luna`, using `development/check.py --timeout 900`:

- `test -p nir --test nominal_parameters --test normal_inputs --test newtypes`:
  46 passed, `/tmp/luna-tools-jw3sq6xw/result.json`.
- `test -p luna --test nominal_value_parameters --test effect_spreads`:
  267 passed, `/tmp/luna-tools-gr3wttd4/result.json`.
- `test -p elaboration --test effects --test effect_requests`:
  19 passed, `/tmp/luna-tools-6nigvfi_/result.json`.

All exited 0. Formatting and `git diff --check` passed; no commit created.

### Effect-group domains and omitted request inputs

Effect groups now register declared value domains, share ordinary clause input
routing and defaults, and preserve those inputs when expanding whole-group rows
and selecting operations. Request inference abstracts declared mixed-sort binders
and settles dependent defaults before reconstructing the complete operation key.
Computed spread packets use the same declared sorts and initial default plan;
their final prepared request reuses normal operation inference.

Source execution covers dependent domains, nominal keys, generic installers,
optional and dependent defaults, value inference through a nominal parameter,
and computed packets. Rejection cases cover wrong domains, different keys and
missing handlers. This does not establish general closure-context completion or
drop-contract discharge.

Validation in `/home/fin/luna`, all exit 0:

- `development/check.py --timeout 900 -- test -p luna --test nominal_value_parameters --test effect_spreads`:
  269 passed, `/tmp/luna-tools-fjx5d_nj/result.json`.
- After packet integration, `development/check.py --timeout 900 -- test -p luna --test nominal_value_parameters --test effect_spreads --test execution effect`:
  91 passed, `/tmp/luna-tools-lbn3pz1z/result.json`.
- Workspace formatting and `git diff --check` passed. Changes remain uncommitted.

### Declaration-time effect defaults

Unused standalone operations and effect groups now undergo default checking.
Wrong domains, self references and forward references are rejected even without
a request. A dependent value default (`T, key:T, .copy:T=key`) exposed duplicate
inherited parameter identities in domain substitution; the substitution now
includes each declaration identity once. Both standalone and grouped operations
execute with prior concrete/symbolic inputs preserved and reject mismatched
defaulted handler keys. Additional packet tests verify dependent defaults supply
callback context, including callbacks inside computed packets.

Validation: `/home/fin/luna`,
`python3 -B development/check.py --timeout 900 -- test -p luna --test nominal_value_parameters --test effect_spreads --test contextual_spreads`,
exit 0, `/tmp/luna-tools-rc1wawz7/result.json`. Workspace formatting and
`git diff --check` passed; no commit created.

### Effect-delivered closure resource evidence

New source conformance tests distinguish ordinary delivery from closed inline
delivery. Both preserve boolean captures through generic forwarding/installers.
Inline delivery also preserves pointer mutation/lifetime checks and owned-capture
consumption: one invocation succeeds, repeated consumption and dangling captures
are rejected. These are verification of existing implementation, not a claim
that ordinary effect delivery now transports complete closure resource evidence.

Two desired-behavior tests remain explicitly ignored in the default suite and
were run separately to establish the actual missing paths. Ordinary pointer
delivery fails with `effect memory premise requires data-only direct delivery`;
ordinary owned closure delivery loses callable evidence and fails with
`closure environment state transfer requires a known live local closure`.
`moves/effects.rs::request` clears operand callable evidence and restores only
precomputed handler completion summaries. A complete fix needs request-specific
operand provenance/ownership in handler analysis and its completion summaries,
including memory summaries; removing the current checks is not sufficient.

Evidence in `/home/fin/luna`:
- `development/check.py --timeout 900 -- test -p luna --test closure_environment_parameters`:
  14 passed, 2 ignored, exit 0; `/tmp/luna-tools-invf4cxd/result.json`.
- The same command with `ordinary_effect_delivered -- --ignored`:
  both known missing cases failed, exit 101; `/tmp/luna-tools-fznwcn7r/result.json`.
- Workspace formatting completed; changes remain uncommitted.

### Owned operand transfer through direct handlers

The ownership reproducer above is now enabled and passes. Checked single-arm,
unguarded direct handlers returning a plain input binding publish its parameter
index in `EffectResultInput` (including S-expression Display). Each request
retains bounded snapshots of actual operand callable/aggregate/tag evidence and
uses the selected operand for completion. No evidence is inferred merely from
equal types; guarded, fallback and computed bodies retain the existing path.
Pointer provenance remains unsupported and its reproducer stays ignored.

Schedule installation also used a Type-only work validator for operation
arguments. It now uses the existing completed instance-argument validator,
accepting checked scalar, nominal and closure-context value identities.

Validation in `/home/fin/luna`, exit 0:
- `development/check.py --timeout 900 -- test -p luna --test closure_environment_parameters --test nominal_value_parameters --test execution effect`:
  93 passed, one known pointer-provenance case ignored;
  `/tmp/luna-tools-2yp_b6pg/result.json`.
- `development/check.py --timeout 900 -- test -p luna --test drop effect`:
  86 passed, `/tmp/luna-tools-nk4xdo6i/result.json`.
- New selected-second-operand execution checks exact destructor ordering and
  one cleanup per capture, plus serialized transfer evidence through a callee:
  `test -p luna --test drop scheduled_effect_forwards_selected`, one passed,
  `/tmp/luna-tools-1en_er3c/result.json`.

### Local aliases and explicit resume in operand transfer

Transfer evidence now follows straight local alias chains, nested blocks, and
explicit `resume` delivery. The analysis consumes the caller's shared fuel and
has a depth bound; calls, assignments and computed patterns cannot supply this
identity-transfer evidence. Existing ownership checking still validates the
complete handler before the evidence is published. A source regression confirms
that consuming a capture before transfer fails during resource preparation,
rather than being revived by the returned callable provenance.

Validation in `/home/fin/luna`: `development/check.py --timeout 900 -- test -p luna --test closure_environment_parameters --test drop effect`,
93 passed, one known pointer-provenance case ignored, exit 0;
`/tmp/luna-tools-bwp47ui4/result.json`. Source cases cover one/multiple aliases,
nested blocks, direct resume and alias-then-resume. Workspace formatting and
`git diff --check` passed. No commit created.

### Common transfer origin across candidates and fallback

Unguarded handler branches now share operand-transfer evidence only when every
branch identifies the same input slot. Literal and unit patterns may filter
other inputs. Partial handlers additionally require matching outer operation,
ABI, signature, slot and forwarded parameter; a divergent or absent fallback
origin does not acquire transfer evidence. Calls and computed/guarded patterns
remain outside this identity-transfer analysis.

Source cases execute both boolean candidates and both matched/forwarded routes,
delivering an owned callback that consumes its capture. A focused proof test
checks differing return origins, guard exclusion, exhausted budget and subsequent
successful analysis. Validation in `/home/fin/luna`, exit 0:
- `development/check.py --timeout 900 -- test -p elaboration --lib branch_transfer`:
  one passed, `/tmp/luna-tools-mjvywhsw/result.json`.
- `development/check.py --timeout 900 -- test -p luna --test closure_environment_parameters --test drop effect`:
  94 passed, one known pointer-provenance case ignored,
  `/tmp/luna-tools-7kmzm0mj/result.json`.
Formatting and diff checks passed; no commit created.

### Shared transfer proof and remaining memory integration

The bounded identity-transfer analysis now lives in
`verification/effect_transfer.rs`, available to both verification layers. Moves
uses its fallible interface: exhausted proof fuel is reported explicitly rather
than silently falling back to an unknown completion. This establishes no pointer
permission by itself. Memory integration remains unfinished: `EffectInput` only
represents data-only delivery, `effect_completion` rejects callable/pointer
results, and request origins must be retained through completion in moves. Those
guards remain in place until result provenance is represented and transported.

Validation in `/home/fin/luna`, exit 0:
- `development/check.py --timeout 900 -- test -p elaboration --lib branch_transfer`:
  one passed, `/tmp/luna-tools-bowdong4/result.json`.
- `development/check.py --timeout 900 -- test -p luna --test closure_environment_parameters --test drop effect`:
  94 passed, one known pointer-provenance case ignored,
  `/tmp/luna-tools-nphdqbxl/result.json`.
Workspace formatting and diff checks passed; no commit created.

### Conditional pointer completion and effectful borrowed helpers

`memory::EffectInput` now records an optional forwarded parameter, including in
its equality/hash and S-expression representation. Validation requires a direct
operation, a valid slot and matching input/result domains; unrelated inputs
remain data-only. Conditional pointer requests snapshot the selected operand's
targets at its evaluation point, and computation/run/installation boundaries
retain completion targets. Function and closure result collection inspect the
completion type instead of losing pointers behind a computation wrapper.
Source memory installation derives transfer information from the shared proof
and joins it with fallback evidence; resource-produced pointer delivery remains
restricted pending actual request-origin transport in moves.

Effectful functions with direct pointer parameters/results need contextual
cleanup publication, even if each operation returns only ordinary data. The
resource and interpreter context classifiers now agree on this case. Contextual
borrowed-helper footprints retain the installer's checked effect inputs through
the callable-memory bridge, and resumed resource analysis restores these memory
premises. The executable regression returns its input pointer after `poll`,
mutates the original object through it and rejects a dangling local variant.

Conditional-memory tests additionally cover pointer result identity through a
callee, absent/out-of-range transfer premises, budget recovery, absence of a
pointee read/write footprint during forwarding, and rejection of local escape.
This does not yet enable general ordinary-handler pointer/closure delivery.

Validation in `/home/fin/luna`, all exit 0:
- `development/check.py --timeout 900 -- check -p elaboration`:
  `/tmp/luna-tools-y93b5na9/result.json` (initial integration).
- Final `development/check.py --timeout 900 -- test -p luna --test effect_memory_transfers --test resources --test closure_environment_parameters --test drop -- effect pointer`:
  238 passed, one known closure-pointer delivery case ignored;
  `/tmp/luna-tools-dca3qzy1/result.json`.
Workspace formatting and `git diff --check` passed. No commit created.

### Ordinary direct pointer delivery

Moves now returns the selected operand's pointer origins from an effect request,
using the sealed identity-transfer premise for the exact operation, occurrence,
ABI and signature. Origins are captured when that operand is evaluated, before
later operands can rebind its source slot. Memory installation derives the same
transfer proof, checks fallback agreement, and permits only the selected direct
pointer input/result; unrelated inputs retain their data-only restriction.
Local aliases and explicit resume preserve this proof. No pointer lifetime is
extended, and handler pointee stability checks remain active.

Both resource analysis and interpreter cleanup classify pointer-result operations
as contextual even when their enclosing function has scalar parameters/results.
This covers a helper which creates a local object, relays its pointer, modifies
the object and returns only a boolean.

Execution regressions cover direct installation, nested calls, aliases, resume,
dangling local rejection, and later-operand rebinding without changing the
already-evaluated pointer. General computed transfers, pointer aggregates and
pointer-capturing closure delivery remain unsupported by this path.

Validation in `/home/fin/luna`:
- `python3 -B development/check.py --timeout 900 -- test -p luna --test effect_memory_transfers --test resources --test closure_environment_parameters --test drop -- effect pointer`:
  exit 0, 242 passed, one known closure-pointer delivery case ignored;
  `/tmp/luna-tools-48hu0ai5/result.json`.
- Workspace formatting: exit 0, `/tmp/luna-tools-cq_lew0_/result.json`.
- `git diff --check`: exit 0. Changes remain uncommitted on dev.

### Identity delivery of pointer-capturing callbacks

The shared effect identity-transfer layout now admits callable values as well as
direct pointers. Only a proven, matching input/result transfer bypasses the
data-only delivery restriction. Moves retains the selected operand's complete
pointer-origin map and existing callable evidence at operand evaluation time.
Memory inference transports capture pointer fields, prepared local code and
named callable identities across request/computation/run/installation nodes.
The caches are populated before later operands execute.

Computation result lattices now describe the completion's callable fields, and
return summaries retain its pointer fields. Generic body types are instantiated
before looking up their completion's operation contracts. Conditional borrowed
field premises now admit Copy scalar pointees under the same restrictions as
direct borrowed parameters; actual calls still check target validity and the
callee footprint. This does not authorize computed handlers, arbitrary pointer
aggregates, or mutable non-scalar captured storage.

The previously ignored ordinary closure-pointer regression is enabled. Added
coverage checks generic effectful forwarders, alias and resume arms, independent
captured targets for identical closure contexts, dangling captures, and both
standalone memory inference and scheduled execution. Existing owned-capture
consumption tests continue to reject repeated consumption.

Focused validation in `/home/fin/luna`, exit 0:
`python3 -B development/check.py --timeout 900 -- test -p luna --test closure_environment_parameters --test effect_memory_transfers`:
20 closure-environment and 5 effect-memory tests passed, none ignored;
`/tmp/luna-tools-ydplroyz/result.json`.

Final related regression in `/home/fin/luna`, exit 0:
`python3 -B development/check.py --timeout 900 -- test -p luna --test effect_memory_transfers --test resources --test closure_environment_parameters --test drop -- effect pointer`:
244 passed, none ignored; `/tmp/luna-tools-jz0c_c6c/result.json`.
Workspace formatting passed (`/tmp/luna-tools-bw_0wqh6/result.json`), as did
`git diff --check`. Changes remain uncommitted on dev.

### Aggregate identity delivery through ordinary effects

Identity-transfer admission now validates the full bounded pointer-input layout
for tuple, record, nominal/newtype and enum values, in addition to direct pointer
and callable values. Request snapshots already transport complete field maps,
callable identities and variant evidence; this change admits those layouts
without treating the transfer as a reconstruction. Newtype pointer results use
the representation-aware pointer target cache across all effect boundaries.
Unrelated operation inputs still require data-only delivery and transfer proof
still requires the same input/result domain and matching branch/fallback origin.

Resource effect-context selection and interpreter cleanup now traverse nested
pointer fields consistently, including operation results inside scalar-returning
functions. This shares the existing bounded callable-layout traversals while
preserving their original callable-only behavior for non-effect callers.

New executable cases check nested record mutation, tuples, wrapped pointers,
variant-qualified origins and absent variants, generic records containing both
code and pointers, and once-only consumption of a captured owner after aggregate
delivery. Lifetime rejection is checked for each pointer representation. Memory
inference is checked independently alongside scheduled execution.

Focused validation in `/home/fin/luna`, exit 0:
`python3 -B development/check.py --timeout 900 -- test -p luna --test effect_memory_transfers`:
10 passed; `/tmp/luna-tools-tfjtknsm/result.json`.

Related regression initially exposed over-classification of pointer-free callable
completion as requiring every operation's context. Pointer-only traversal is now
separate from callable-identity traversal: operation results need either kind,
whereas the added parameter/completion rule only detects nested pointers.
The existing `effect_completion_ownership_retains_callable_provenance` test and
the five aggregate `ordinary_effect` tests passed after repair;
`/tmp/luna-tools-8yuh9vra/result.json` (exit 0).

Final related regression in `/home/fin/luna`, exit 0:
`python3 -B development/check.py --timeout 900 -- test -p luna --test effect_memory_transfers --test resources --test closure_environment_parameters --test drop -- effect pointer`:
249 passed, none ignored; `/tmp/luna-tools-3q3dpsor/result.json`.
Workspace formatting passed (`/tmp/luna-tools-6eqk27qt/result.json`), as did
`git diff --check`. Changes remain uncommitted on dev.

### Conditional identity-transfer proofs

The shared proof now joins if/match branches only when each delivers the same
input position. Each branch receives a separately budgeted alias environment;
branch-local aliases do not supply evidence to sibling branches. Pure scalar
selectors can be introduced by local bindings without being mistaken for
delivered pointer/callable provenance. Literal/wildcard observation patterns and
scalar guards are supported, including guards on outer handler patterns.

The observation recognizer only accepts direct scalar reads/projections,
primitive logical/arithmetic operations and transparent blocks. Calls, stores,
pointer reads and computed patterns remain outside this proof. The installer
still checks ordinary ownership, memory footprints and fallback agreement.
Insufficient budget to copy a branch environment is an explicit exhaustion
error, rather than an unsupported-result fallback.

Tests check agreeing and disagreeing branch origins, opaque call guards, budget
failure/recovery, both boolean outcomes of if/match/guard/fallback handlers,
pointer writes and dangling rejection. The owned-callback branch/fallback test
also exercises the new paths with a consuming capture.

Focused validation in `/home/fin/luna`, exit 0:
- `python3 -B development/check.py --timeout 900 -- test -p elaboration --lib branch_transfer`:
  one passed; `/tmp/luna-tools-9uq7bath/result.json`.
- `python3 -B development/check.py --timeout 900 -- test -p luna --test effect_memory_transfers ordinary_effect_transfer_tracks_conditional_bodies_and_guards`:
  one passed (five handler forms, both boolean outcomes and lifetime rejection);
  `/tmp/luna-tools-psbmyl8a/result.json`.

Final related regression in `/home/fin/luna`, exit 0:
`python3 -B development/check.py --timeout 900 -- test -p luna --test effect_memory_transfers --test closure_environment_parameters --test resources --test drop effect`:
116 passed, none ignored; `/tmp/luna-tools-orvgvcky/result.json`.
Workspace formatting passed (`/tmp/luna-tools-jbgqmcgi/result.json`), as did
`git diff --check`. Changes remain uncommitted on dev.

### Alternative operand origins through ordinary effects

Transfer proofs and conditional memory/result premises now carry a canonical
`BTreeSet` of possible complete input positions. Branches and compatible
fallbacks union these sets; an unproved path keeps the result unproved. Public
memory premises reject empty sets, out-of-range slots and input/result domain
mismatches. Set traversal/copies consume the shared proof budget, and dumps use
`(forwarded-parameters ...)` in deterministic order.

Requests snapshot every candidate at its own evaluation point. Moves merges
pointer origins and restores candidate callable/variant evidence; memory merges
pointer targets, field origins and prepared/named code identities. This denotes
one selected runtime operand, not duplicated ownership. Unrelated operation
inputs retain the existing data-only restriction. All possible returned pointer
origins remain obligations at the use site.

New tests cover different if/arm/fallback sources, borrowed callback candidates,
owned callback consumption, variant-qualified fields and absent variants.
Conditional summaries verify both input origins survive later-operand rebinding,
without inventing a pointee read/write footprint, and check malformed premises
plus budget recovery. Scheduled cleanup records distinguish both captured
owners: the unselected owner drops first and the selected owner drops after
invocation, exactly once on either branch.

Focused validation in `/home/fin/luna`, exit 0:
- `python3 -B development/check.py --timeout 900 -- test -p elaboration --lib branch_transfer`:
  one passed; `/tmp/luna-tools-3xeuzd54/result.json`.
- `python3 -B development/check.py --timeout 900 -- test -p luna --test effect_memory_transfers`:
  16 passed; `/tmp/luna-tools-3r30j3_i/result.json`.
- `python3 -B development/check.py --timeout 900 -- test -p luna --test drop scheduled_effect_alternative_callbacks_drop_each_owner_once`:
  one passed; `/tmp/luna-tools-y8va2gkj/result.json`.

Final related regression in `/home/fin/luna`, exit 0:
`python3 -B development/check.py --timeout 900 -- test -p luna --test effect_memory_transfers --test closure_environment_parameters --test resources --test drop -- effect pointer`:
256 passed, none ignored; `/tmp/luna-tools-i260bqir/result.json`.
Workspace formatting passed (`/tmp/luna-tools-vtkttil3/result.json`), as did
`git diff --check`. Changes remain uncommitted on dev.

### Checked Copy inputs outside the returned-origin set

Conditional memory premises now record `copy_parameters`, displayed as
`(copy-inputs ...)`. These are full representation/cleanup-compatible Copy
obligations, not ownership permissions inferred from a pointer layout. Resource
installations obtain them through the checked delivery callback and seal them on
result premises. Contextual memory bridges preserve and charge this metadata.
A proven transfer can ignore alias-bearing inputs only under those obligations;
missing or out-of-range premises are rejected. Ordinary owning cleanup remains
separate, and a Copy marker does not exempt a custom destructor.

Multiple applicable contracts for one operation are conservatively joined by
memory lookup, including returned origins and Copy obligations. Previously the
first matching entry could select one scoped provider's origins arbitrarily.
Runtime/resource occurrence routing remains distinct; this conservative memory
join may retain extra lifetime obligations until memory keys encode occurrences.

Operation pointer inputs now trigger contextual cleanup even with scalar
function/completion results. Aggregate call-result dispatch also uses the
completion type behind the computation wrapper; direct use of an effectful
aggregate result previously lost field origins despite a correct memory summary.

Focused validation in `/home/fin/luna`, exit 0:
- `python3 -B development/check.py --timeout 900 -- test -p luna --test effect_memory_transfers`:
  20 passed before the additional contract-order regression;
  `/tmp/luna-tools-k9de65mb/result.json`.
- `python3 -B development/check.py --timeout 900 -- test -p luna --test effect_memory_transfers conditional_memory_joins_same_operation_contracts_without_order_dependence`:
  one passed; `/tmp/luna-tools-2197nk12/result.json`.
Cases cover direct pointers and Copy records, nested distinct providers, scalar
completion, malformed/missing Copy premises, order-independent candidate sets
and rejection of conflicting custom-cleanup responsibility.

Final related regression in `/home/fin/luna`, exit 0:
`python3 -B development/check.py --timeout 900 -- test -p luna --test effect_memory_transfers --test closure_environment_parameters --test resources --test drop -- effect pointer`:
260 passed, none ignored; `/tmp/luna-tools-9iz73j8y/result.json`.
The separate contract-order test above is excluded by these name filters.
Workspace formatting passed (`/tmp/luna-tools-llchuef3/result.json`), as did
`git diff --check`. Changes remain uncommitted on dev.

## Branch bindings and early resume transfer flow

Ordinary direct handler identity proofs now distinguish normal completion from
resumed completion. Sequential blocks accumulate resumed origins while only the
remaining paths continue. Conditional and scalar-match arms can end in resume,
or resume on some paths and deliver a normal tail on others. Discard removes
normal result identity without removing an already resumed result; a discarded
operand alone cannot count as delivery. Unsupported calls/stores still invalidate
this bounded proof, and ownership/memory checks remain independently required.

Scalar pattern bindings inherit identity only from a proven operand alias.
Computed selectors receive no identity, nested matches preserve legitimate
aliases, and branch/block environments are isolated. All flow joins and copied
origin maps are fuel charged.

Validation in `/home/fin/luna`, exit 0:
- `python3 -B development/check.py --timeout 900 -- test -p elaboration --lib branch_transfer`:
  one test, including unknown/computed origins, sibling isolation, partial
  resume, discarded tail rejection, fuel failure and recovery;
  `/tmp/luna-tools-mn6mrtxa/result.json`.
- `python3 -B development/check.py --timeout 900 -- test -p luna --test effect_memory_transfers --test drop -- effect`:
  88 drop tests and 17 effect-memory tests passed;
  `/tmp/luna-tools-24z3j4r6/result.json`. The name filter excludes four other
  effect-memory tests and unrelated drop tests. Expanded source cases cover
  nested bindings, guards, conditional/match resume, pointer mutation and
  dangling rejection, alternative origins, and exact owning-closure drop order.
- Formatting: `/tmp/luna-tools-kjrkin8o/result.json`; `git diff --check` passed.

An initial source regression exposed the frontend's Discard wrapper on a
statement-position conditional; the final implementation explicitly preserves
resume flow through it. Changes remain uncommitted on dev.

Parameter patterns now reuse observation-pattern identity tracking, including
whole-input `as` bindings, `or` candidates and simultaneous candidate binding
remaps. This does not admit field projection as whole-input transfer. Expanded
source validation checks both bool values for literal/as/or patterns, and four
owning callback delivery bodies (including resume during a let initializer),
with one successful invocation and rejection of a second affine invocation.
`test -p luna --test effect_memory_transfers` via development/check.py passed all
21 tests (`/tmp/luna-tools-jex6qni8/result.json`). After moving remap copy charging
before allocation, `test -p elaboration --lib branch_transfer` also passed
(`/tmp/luna-tools-dycxjfy9/result.json`). Formatting passed before that final
ordering-only adjustment (`/tmp/luna-tools-ht2asynt/result.json`), and
`git diff --check` passed afterward. All commands ran in `/home/fin/luna`.

Pure scalar selectors now recurse through local let bindings, if/match, guards,
and discarded observations. Ordinary calls remain unsupported even when hidden
inside those constructs; no interprocedural purity is inferred. Source coverage
adds four accepted guard/selector forms (each with both flag values and dangling
pointer rejection) and four opaque-call rejection forms. The complete 22-test
`effect_memory_transfers` suite passed via development/check.py in
`/home/fin/luna` (`/tmp/luna-tools-o2lvr91s/result.json`, exit 0). Formatting
passed (`/tmp/luna-tools-3x7q6ajz/result.json`); `git diff --check` passed.

Whole-aggregate pattern aliases now retain operand identity across record,
tuple and enum shape observations. Field bindings intentionally receive no
whole-operand identity. OR candidates intersect the identities of canonical
bindings rather than letting the last alternative overwrite earlier evidence;
remapping projected bindings does not invent identity. Computed patterns remain
outside this structural proof.

Validation in `/home/fin/luna` through development/check.py:
- The 22 existing effect-memory tests passed in `/tmp/luna-tools-vog82fql/result.json`;
  that run initially failed the new aggregate case due to an incorrect named
  struct pattern in the fixture. Corrected to the language's `{field:pattern}`
  syntax; focused aggregate regression passed in `/tmp/luna-tools-3ro179zw/result.json`.
  It covers record/tuple/enum/OR aliases plus dangling rejection for each.
- Both effect-transfer unit tests passed in `/tmp/luna-tools-7g0kw4dk/result.json`;
  projected-vs-whole alternatives are checked in both orders, including remaps.
- Owning callback record-pattern test passed in `/tmp/luna-tools-a805lurh/result.json`:
  both selector values, ordinary/resume delivery, one invocation accepted and
  second invocation rejected as unavailable.
- Formatting passed in `/tmp/luna-tools-5rkp1p7j/result.json`; diff whitespace
  check passed. Changes remain uncommitted on dev.

Handler bodies now admit structural match subjects at local/direct-field places,
not only scalar subjects. Whole-alias evidence survives nested tuple/record/enum
matches and owning callback records; direct fields do not acquire whole-input
identity. Scalar selectors may also inspect an aggregate's structure. Calls and
pointer reads are not admitted as structural subject observations.
Validation via development/check.py in `/home/fin/luna`, exit 0:
- Complete then-current 24-test effect_memory_transfers suite:
  `/tmp/luna-tools-x18l3rr8/result.json`.
- Added rejection cases for aggregate producer calls and pointee observations:
  `/tmp/luna-tools-2bojyf8p/result.json` (one test). The initial pointer-to-record
  fixture was rejected earlier by frontend Copy admission; the final fixture
  uses a supported bool pointee read to exercise the intended memory boundary.
- Both effect-transfer unit tests: `/tmp/luna-tools-omv_wn3r/result.json`.
Formatting passed (`/tmp/luna-tools-45jeg835/result.json`) before that fixture's
literal-only correction; `git diff --check` passed afterward. Uncommitted on dev.

Local pattern-let transfer now reuses structural pattern identity analysis.
Let-else failures analyze the pre-match environment and must leave the path;
resume origins from failure join normal/successful delivery. Pure guards remain
observation checked. An owning initializer is still consumed according to the
resource checker: reusing its original variable in else was correctly rejected;
matching a Copy scalar field instead leaves the owner available for delivery.
Source cases cover whole-record local aliases, let-else and guarded let-else,
distinct left/right pointer alternatives, dangling rejection and owning callback
single-use preservation. Initial malformed fixture delimiters were corrected;
the final relevant suite passes.

Integration validation in `/home/fin/luna`, exit 0:
`python3 -B development/check.py --timeout 900 -- test -p luna --test effect_memory_transfers --test closure_environment_parameters --test resources --test drop -- effect pointer`
passed 264 tests (10 closure environment, 144 drop, 24 effect memory, 86 resources),
none ignored; `/tmp/luna-tools-n3zg21of/result.json`. One unrelated-name contract
ordering test is filtered out. The shared wrapper cleaned 1.7 GiB of rebuildable
elaboration/luna output after completion. Formatting passed in
`/tmp/luna-tools-79ogf0fz/result.json`; `git diff --check` passed. Uncommitted on dev.

Match subjects in transfer bodies now use TransferFlow directly. Conditional
selection and local alias preparation can supply normal subject origins, while
subject-side resume paths bypass matching and join the final delivery set.
Direct field observation is admitted without promoting a field to whole-input
identity. Ordinary calls and pointer reads remain unsupported by this proof.
Source tests extend alternative pointer and owning callback cases to selected
subjects, early resume during subject preparation, and block-prepared owning
records; both runtime choices and affine/dangling rejections are checked.
Validation in `/home/fin/luna`, exit 0, through development/check.py:
- All 25 effect_memory_transfers tests: `/tmp/luna-tools-tytivjt0/result.json`.
- Both verification::effect_transfer unit tests: `/tmp/luna-tools-an4czpls/result.json`.
Formatting: `/tmp/luna-tools-ndh5ij2u/result.json`; diff whitespace check passed.

Independent data results can now ignore checked Copy alias-bearing inputs
without a forwarded-input origin proof. Memory premise validation, resource
installation and request admission consistently enforce Copy/layout obligations;
non-data results still require origin evidence. This permission is solely to
ignore the input, not to dereference it or retain its aliases.

The remaining operation-argument read/write gap is explicit: installed arm
parameters currently have no parameter-relative access footprint/provenance;
actual reads/writes cannot be authorized merely by relaxing identity-transfer
checks. Handler captures and operation arguments remain distinct contracts.

Validation in `/home/fin/luna` via development/check.py:
- Full then-current 26-test effect_memory_transfers suite passed:
  `/tmp/luna-tools-v955hs_a/result.json`.
- New conditional-premise/negative-boundary test passed:
  `/tmp/luna-tools-8t814i1i/result.json`. It checks empty access/result footprints,
  rejection without a Copy premise, rejected pointee reads/writes, and a Copy
  marker that conflicts with custom cleanup. A first negative fixture assumed
  raw pointers needed an explicit marker; intrinsic pointer delivery correctly
  accepted it, so the final fixture tests an actual cleanup incompatibility.
Formatting passed (`/tmp/luna-tools-r1zokgyw/result.json`) before the literal-only
fixture correction; diff whitespace check passed. Uncommitted on dev.

## Conditional operation-parameter access footprints

`memory::EffectInput` now includes a deterministic/hashable set of
`EffectParameterAccess { parameter, input_fields, read_value, write }`.
Display emits `(parameter-accesses (parameter-access ...))`; worklist keys and
lookup merges retain the complete set, and copying/traversal charges field paths.
Validation checks parameter and pointer layout paths, requires a read or write,
and currently restricts targets to unguarded scalar storage. Guarded enum leaves
and pointer-slot traversal need further contracts rather than guessed validity.

Request memory inference snapshots each accessed pointer at its argument's
preparation point and records the footprint after operands are prepared. Direct
pointer inputs and pointer fields of owned records are mapped to caller origins.
Later pointer-variable reassignment does not retarget a prior operand. Equivalent
operation premises merge access sets conservatively, independent of input order.

This is conditional-summary support, NOT completed source-handler access support.
Resource-sealed EffectResultInput does not yet carry accesses; existing installer
and resource-to-memory producers explicitly construct empty access sets. Handler
parameter bodies still lack formal borrowed origins/access derivation, so normal
source handlers reading/writing operation parameters remain rejected. Next work
must infer/check those arm footprints, seal them, preserve them across both
resource-to-memory bridges, and validate actual request targets. Do not claim
those steps are complete or use empty fields to drop a future nonempty contract.

Validation in `/home/fin/luna` through development/check.py, exit 0:
- All 29 effect_memory_transfers tests: `/tmp/luna-tools-kr5po_f6/result.json`.
  New tests cover read/write/both, direct and aggregate input projection,
  evaluation-time pointer snapshots, malformed accesses, budget recovery,
  deterministic dump and conservative order-independent premise merging.
- `test -p luna --test resources -- effect`: 12 passed;
  `/tmp/luna-tools-tv5vnv1p/result.json`.
- Formatting: `/tmp/luna-tools-xgh97n34/result.json` (before a source-location
  correction in one error map); final diff whitespace check passed.
All changes remain uncommitted on dev.

### Sealed access transport

EffectResultInput now stores private parameter_accesses, includes path work in
budget charging, and emits accesses in its S-expression. A single memory_input
conversion preserves the complete contract for both resource analysis and
callable-memory inference. Installed memory premises obtain access sets from the
matching checked result slot/operation/ABI/signature; fallback completion evidence
also retains the outer provider's accesses. This supersedes the earlier note
that resource-to-memory bridges always construct empty access sets.

Actual local-arm access derivation and borrowed origins are still pending. New
local arms therefore currently contribute no accesses; only established evidence
can supply nonempty sets. Do not advertise source parameter dereference support.
The next implementation needs independent arm memory inference plus resource
borrowed parameter seeding, checked actual targets and cleanup publication.
Existing helpers include memory::infer_closure_with_effect_inputs (synthetic
closure input may allow reusing argument-relative inference) and
Analyzer::declare_closure_borrow (conditional pointee seeding); neither may be
used to bypass actual target checks or cleanup-root mapping.

Validation in `/home/fin/luna`, exit 0:
- `development/check.py --timeout 900 -- test -p elaboration --lib sealed_effect_parameter_access`:
  `/tmp/luna-tools-lwq3y1k9/result.json`. Unit-level checked evidence with a nonempty
  footprint is passed through infer_callable_memory_with_effect_context into a
  real NIR effect request; its parameter read/write appears in the summary.
  Empty footprint and exhausted budget are separately checked. This tests the
  bridge, not source-installer derivation.
- All 29 effect_memory_transfers tests: `/tmp/luna-tools-1c69wh0z/result.json`.
Formatting passed in `/tmp/luna-tools-0ofmasd8/result.json` before the small fallback
transport addition; final diff whitespace check passed. Uncommitted on dev.

### Closed-arm memory derivation

Added public memory::infer_handler_arm_memory and HandlerParameterSummary
(memory ArgumentSummary plus EffectParameterAccess set, with S-expression
Display). Direct arms with distinct complete parameter bindings and no lexical
captures use a private closure analysis view. The normal memory engine supplies
parameter-relative accesses and return origins, including record pointer fields
and named helper calls. Existing overwrite inference conservatively records
read+write, so tests preserve that established footprint.

The private view rewrites operation resume into its own return boundary.
Source lexical Return, nested Closure/EffectHandle, captures and decomposing
parameter patterns are explicitly rejected, rather than silently reinterpreting
their scope or environment. Copies/traversal/type work are bounded; parameter
binding uniqueness uses a set. The source IR is never mutated.

This helper is not yet wired into handler resource installation or the source
walk_handler path. Actual parameter borrowed origins and request target checks
are still required before enabling runtime parameter access. Consumers must
also retain may_run_cleanup/may_reject and returned-origin obligations; the
access set alone is not the complete arm contract.

Validation in `/home/fin/luna`, exit 0:
`python3 -B development/check.py --timeout 900 -- test -p luna --test effect_memory_transfers`
passed all 31 tests (`/tmp/luna-tools-f9e8phza/result.json`). New cases cover direct
read/write, scalar and pointer resume, mixed resume/normal completion, helper
calls, record fields, budget failure/recovery, Display, captured environment and
lexical return rejection. Formatting passed in `/tmp/luna-tools-avaf9oix/result.json`
before the small uniqueness-set improvement; final diff whitespace check passed.

## Executable direct scalar-pointer handler parameters

Closed direct arms with complete parameter bindings, data-only results, no
extra cleanup footprint and explicit direct scalar-pointer accesses now pass a
shared `direct_handler_accesses` admission helper. Source memory inference uses
that arm footprint at requests rather than walking formal pointer bindings with
unknown origins. Resource installation checks the arm against conditional
borrowed scalar roots, records BorrowedRoot::Parameter, excludes these roots from
ownership cleanup, and seals the derived accesses only after arm checking.

Requests snapshot actual accessed targets as each operand is prepared and,
after all operands complete, validate live pointer-free Copy scalar ownership
and the precise pointee type. Effect-call bridges retain the footprint. Existing
identity-transfer support and other arm forms use their original paths when the
new admission is unavailable. Direct-passing ABI remains separate.

Source execution is now supported for direct pointer reads/writes, ordinary
scalar helpers and explicit resume, including calls through an effectful callee.
Guard reads and outer fallback writes compose. Reads through dead local targets
are rejected. Write footprints invalidate pre-call observations used to prove
postconditions. Aggregate parameter fields remain inference-only: runtime access
admission currently requires a direct pointer slot; captures, decomposed patterns
and additional cleanup responsibilities still need their own contracts.

Validation in `/home/fin/luna`, exit 0:
- New end-to-end `ordinary_handlers_access_checked_scalar_pointer_parameters`:
  `/tmp/luna-tools-vvj0r099/result.json` (read/write/helper/resume and dangling
  rejection for each).
- Full suite initially passed 33 cases and hit a malformed guard token in the
  new fallback fixture (`pointer.*=>`); corrected token separation. Focused
  snapshot/fallback regression passed `/tmp/luna-tools-mhn9af0q/result.json`.
  The postcondition observation-invalidation case passed in the full run.
- Integration command via development/check.py:
  `test -p luna --test effect_memory_transfers --test closure_environment_parameters --test resources --test drop -- effect pointer`
  passed 270 tests (10 + 144 + 30 + 86), none ignored;
  `/tmp/luna-tools-tgzlt6jy/result.json`. Four effect-memory names are filtered
  out; they were covered separately or by the preceding full run.
- Formatting `/tmp/luna-tools-0u3hhzb8/result.json` preceded the guard-token-only
  fixture correction; final diff whitespace check passed. Cache wrapper cleaned
  rebuildable outputs normally. Changes remain uncommitted on dev.

### Executable aggregate-pointer handler parameters

Extended the closed direct-arm admission from raw pointer parameters to scalar
pointer leaves in Copy aggregates. Installation resolves each symbolic input path
through `pointer_input_layout`, seeds a distinct borrowed root at the physical
storage path, and records the symbolic parameter mapping. These roots do not
enter owned cleanup. Request checking snapshots the same physical paths before
later arguments run, then validates actual target liveness and scalar Copy
storage. Distinct formal roots do not assert actual non-aliasing.

This supersedes the aggregate inference-only limitation immediately above.
Nested records, tuples and newtype pointer wrappers now execute through ordinary
handlers. Captures, decomposed handler parameters, guarded variant leaves,
non-Copy input/target ownership transitions and extra cleanup remain outside this
admission; errors and non-direct ABIs retain their existing paths.

Validation in `/home/fin/luna`:
- `python3 -B development/check.py --timeout 900 -- test -p luna --test effect_memory_transfers`
  passed all 38 tests, exit 0; `/tmp/luna-tools-h8j6o1f3/result.json`.
  New cases cover four aggregate layouts, dangling targets, shared target write
  order, prepared argument snapshots, absent Copy and conflicting custom Drop.
- The initial run passed 35 tests and rejected the tuple fixture because its
  Duplicate implementation was missing; added the required fixture declaration,
  preserving the whole-input Copy requirement.
- Related integration: `python3 -B development/check.py --timeout 900 -- test -p luna --test closure_environment_parameters --test resources --test drop -- effect pointer`
  passed 240 tests (10 closure + 144 drop + 86 resources), exit 0;
  `/tmp/luna-tools-dy0lv6wl/result.json`. Together with the full effect-memory
  suite, this turn verifies 278 tests. No full-workspace test run was requested.
- Final workspace formatting: `python3 -B development/command.py --timeout 120 -- cargo fmt --all`,
  `/tmp/luna-tools-0g42esya/result.json`; final whitespace check also passed.
  Changes remain in the dev worktree, uncommitted.

### Destructured operation parameters in direct handler access

The closed-arm memory adapter now receives whole operation inputs under fresh
private binding owners and applies ordinary LetPattern analysis before guards
and body. Pattern owners, expression-local uses and local declarations are
scanned under the analysis budget before selecting the private owner. Source
bindings and operation-relative input paths remain distinct. Pattern code and
nested lexical environments retain their rejection paths.

Resource installation now seeds borrowed origins on the complete parameter
Tuple before calling the ordinary pattern binder, rather than injecting them
into a single whole-input binding afterward. This enables record and tuple
fields, nested records and whole-pattern aliases without a second decomposition
algorithm. Borrowed targets still do not become owned cleanup roots. Request
validation and actual aliasing behavior are unchanged.

Validation in `/home/fin/luna`:
- `python3 -B development/check.py --timeout 900 -- test -p luna --test effect_memory_transfers`
  passed 40 tests, exit 0; `/tmp/luna-tools-6toh6y84/result.json`.
  New execution cases cover nested/tuple/record entry decomposition, whole
  aliases, dangling rejection, wildcard companions, guarded fallback and resume.
  The obsolete inference-only rejection for wildcard inputs was removed; the
  fallback test now exercises ignored scalar inputs with actual pointer access.
- Added an OR-pattern remap case afterward, included in integration below.
- Integration: `python3 -B development/check.py --timeout 900 -- test -p luna --test effect_memory_transfers --test closure_environment_parameters --test resources --test drop -- effect pointer handler`
  passed 361 tests (10 closure + 223 drop + 39 effect-memory + 89 resources),
  exit 0; `/tmp/luna-tools-w3k6zv2r/result.json`. The one filtered effect-memory
  test passed in the preceding full run. The added OR-pattern case passed here.
- Formatting: `python3 -B development/command.py --timeout 120 -- cargo fmt --all`,
  exit 0; `/tmp/luna-tools-rh49bz3n/result.json`. Final `git diff --check` passed.
  Documentation updated; changes remain uncommitted on dev.

### Parameter accesses combined with checked lexical captures

The handler analysis adapter now accepts the installer's explicit capture types.
Copy data captures use the closure environment; stable direct scalar-pointer
captures use the existing borrowed-capture analysis and virtual input slots.
`HandlerParameterSummary` retains capture accesses with their lexical LocalId,
separately from operation parameter accesses, and displays both in its
S-expression dump. Source memory inference maps captured accesses through the
installer's actual pointer origins; only operation inputs enter request access
contracts. Public closed-arm inference still requires an explicit environment
before admitting free bindings.

The resource installer retains its existing Copy/cleanup, lexical availability,
address-exposure, and pointer-slot stability checks. It now admits the combined
parameter summary after those checks. Newly seeded operation borrowed roots skip
roots already occupied by captured storage, including installer-parameter roots;
formal storage does not assert actual non-aliasing. Runtime captures remain shared
installer state, not copied closure cells: repeated requests observe updates.

Validation in `/home/fin/luna`:
- Full effect-memory suite after data captures: 42 passed, exit 0;
  `/tmp/luna-tools-24tp67wj/result.json`.
- Full suite after pointer captures and root allocation: 44 passed, exit 0;
  `/tmp/luna-tools-_14s7xln/result.json` via
  `python3 -B development/check.py --timeout 900 -- test -p luna --test effect_memory_transfers`.
  Tests cover repeated mutation of a captured scalar, destructured inputs,
  captured guards with fallback/resume, distinct and aliased capture/input
  targets, and both source-summary write footprints. Rejection cases cover
  dangling captures, pointer-slot rebinding and address-exposed scalar captures.
- Added a bool input/u32 captured-target regression to ensure virtual roots of
  different storage types cannot collide; included in integration below.
- Integration first run: `python3 -B development/check.py --timeout 900 -- test -p luna --test effect_memory_transfers --test closure_environment_parameters --test resources --test drop -- effect pointer handler`,
  `/tmp/luna-tools-eavjnx3v/result.json`: closure 10 and drop 223 passed;
  effect-memory passed 43 and failed the new root-collision fixture at name
  resolution because the minimal test environment does not predeclare `u32`.
  Added `typealias u32=.u32'builtin;` to the fixture. Resources had not run yet.
  The cache wrapper cleaned 1.7 GiB of rebuildable outputs after this run.
- After the fixture correction, reran `python3 -B development/check.py --timeout 900 -- test -p luna --test effect_memory_transfers --test resources -- effect pointer handler`:
  exit 0; `/tmp/luna-tools-auxi09sl/result.json`. Both the mixed-storage-type
  borrow regression and the remaining resource group pass (44 effect-memory,
  89 resources). With the earlier 10 closure and 223 drop cases, 366 related
  integration tests passed. One filtered effect-memory case passed in the full
  suite before this literal fixture correction.
- Workspace formatting passed before the type-alias-only fixture correction:
  `/tmp/luna-tools-lu258r5q/result.json`; final `git diff --check` passed.
  Documentation includes a repeated-request capture example and separates
  parameter/capture provenance. Changes remain uncommitted on dev.

### Stable pointer-bearing aggregate captures

Handler capture validation now uses pointer input layouts, covering records,
nested records, tuples and newtype pointer wrappers. Source summaries resolve
symbolic input paths through lexical capture bindings; resource installation
reads physical storage paths from the same layout. Captured field targets are
borrowed, while the aggregate remains owned by the installer. Virtual capture
slots retain capture ordering rather than assuming every capture is a raw pointer.

Admission also handles capture-only accesses: an operation no longer needs an
unrelated pointer parameter to use this checked path. Stable whole-capture roots
are inherited by nested resource frames, and classification of scalar captures
uses a budgeted root set instead of repeatedly scanning all pointer fields.
Every captured pointer field requires known provenance and live Copy scalar
storage. Whole-object/field assignment and address exposure of pointer-bearing
captures remain conservatively rejected. Variant-qualified fields and unknown
captured callable identities still need separate contracts.

Validation in `/home/fin/luna`:
- `python3 -B development/check.py --timeout 900 -- test -p luna --test effect_memory_transfers`
  passed 47 tests, exit 0; `/tmp/luna-tools-hn_sue5q/result.json`.
  New matrix covers four layouts with and without pointer operation parameters,
  source-summary captured writes, dangling capture targets, whole/field
  rebinding, and address exposure.
- Added alias-sharing across two captured fields and a request parameter, plus
  rejection of conflicting aggregate cleanup, for the integration run below.
- Removed the obsolete pointer-record row from the old blanket aggregate-capture
  rejection test in drop.rs. Its valid-provenance behavior is now covered by the
  new execution matrix, and actual provenance failures by dedicated negatives;
  affine and unknown-callable capture cases remain in that rejection test.
- Integration: `python3 -B development/check.py --timeout 900 -- test -p luna --test effect_memory_transfers --test resources --test drop -- effect pointer handler`
  passed 358 tests (223 drop + 46 effect-memory + 89 resources), exit 0;
  `/tmp/luna-tools-0i8_tysm/result.json`. Two effect-memory names were filtered:
  one was covered by the preceding full suite; the newly added alias case is
  run separately below. Cache cleanup removed 1.6 GiB of rebuildable artifacts.
- Final formatting passed: `python3 -B development/command.py --timeout 120 -- cargo fmt --all`,
  `/tmp/luna-tools-k8_wwj4j/result.json`; `git diff --check` also passed.
- Targeted alias/cleanup regression passed, exit 0:
  `python3 -B development/check.py --timeout 900 -- test -p luna --test effect_memory_transfers aggregate_capture_aliases_share_actual_storage_with_request_inputs`,
  `/tmp/luna-tools-7qdujctv/result.json`. This brings the related integration
  coverage to 359 passing tests, plus the earlier full-suite-only case.
  Documentation updated; changes remain uncommitted on dev.

### Disjoint data updates in pointer-bearing handler captures

Capture stability now retains complete mutation paths rather than a root-only
set. Assign/bit-write paths are compared with captured pointer storage paths in
both directions: replacing a pointer, its parent or the entire aggregate still
requires a writeback contract, while disjoint data fields may change. Address
exposure conservatively records the whole root and keeps the existing restriction.
Path copies and overlap comparisons are budgeted. Cached enclosing-scope scans
store the same Place paths, so closure/handler frames without a function exit
contract use the same rule as complete function scans.

This supersedes the blanket field-assignment restriction in the preceding
checkpoint. Pointer-bearing captures can now hold both stable pointers and mutable
Copy data, including nested data-only subaggregate replacement. Whole-scope
checking and inherited pointer-slot stability remain in place.

Validation in `/home/fin/luna`:
- Full effect-memory suite: `python3 -B development/check.py --timeout 900 -- test -p luna --test effect_memory_transfers`,
  50 passed, exit 0; `/tmp/luna-tools-empcrq51/result.json`. Repeated requests
  observe boolean/nested-field updates and data-subaggregate replacement;
  pointer-field/ancestor writes and field address exposure are rejected.
- Added nested-handler cached-scope regression, positive plus pointer-slot
  self-assignment rejection: `python3 -B development/check.py --timeout 900 -- test -p luna --test effect_memory_transfers nested_handlers_preserve_pointer_field_stability_with_mutable_data`,
  exit 0; `/tmp/luna-tools-wp5dhhpd/result.json`.
- Formatting passed `/tmp/luna-tools-qu10mtsh/result.json` before a literal-only
  change strengthening the nested negative to self-assignment; whitespace check
  passed afterward. Documentation includes the mixed pointer/data state example.
- Related handler integration: `python3 -B development/check.py --timeout 900 -- test -p luna --test drop --test resources -- handler`,
  108 drop + 4 resources passed, exit 0;
  `/tmp/luna-tools-qw8dbt4e/result.json`. Together with the 50 full-suite and
  one added nested effect-memory test, 163 distinct tests passed this turn.
  Final whitespace check passed. Changes remain uncommitted on dev.

### Variant-qualified operation parameter accesses

Conditional operation accesses now retain enum-qualified input paths. Request
preparation maps physical origins through the established tag domains with
`memory_input_targets`; an empty symbolic set means an excluded payload, while
missing origins remain an error. Tags and targets are captured before later
arguments evaluate. Formal handler origins union candidates at shared physical
slots instead of overwriting the preceding variant's borrowed target.

Generic function field-borrow premises also share physical payload storage.
`guarded_request_targets` recovers mutually exclusive candidates only when the
request argument is an unchanged original parameter or its field projection:
borrowed-root metadata identifies the original symbolic input path, and guarded
paths are compared at the corresponding source position. A complete lexical
mutation scan is required before applying this correlation. Missing correlations
retain conservative candidates; no mismatched pointee is silently accepted.
Capture variant guards remain outside the stable-capture contract, and a single
handler branch merging different pointee types can still need finer candidate
state. EffectParameterAccess Display now renders explicit `(variant N)` nodes.

Validation in `/home/fin/luna`:
- `.none` / `.some` access, present dangling rejection:
  `/tmp/luna-tools-5f66cftd/result.json`, exit 0.
- Initial full run `/tmp/luna-tools-ezhhclfk/result.json` passed 54 and exposed
  the physical-slot union of bool/u32 borrowed fields. Implemented the guarded
  source-parameter correlation above; focused mixed-type run passed
  `/tmp/luna-tools-swcfgns3/result.json`.
- Full `python3 -B development/check.py --timeout 900 -- test -p luna --test effect_memory_transfers`
  then passed 55 tests, exit 0; `/tmp/luna-tools-5t2oicby/result.json`. Matrix
  includes struct-field projections of mixed payloads, prepared tag/origin
  snapshots, and whole-value matches with same-pointee variant candidates.
- Added dump/path checks afterward for the qualified access metadata.
  Formatting passed `/tmp/luna-tools-pjg351hv/result.json`; whitespace check passed.
- Integration: `python3 -B development/check.py --timeout 900 -- test -p luna --test effect_memory_transfers --test drop --test resources -- effect pointer handler`,
  365 passed (223 drop + 53 effect-memory + 89 resources), exit 0;
  `/tmp/luna-tools-g0uxflkl/result.json`. Both filtered effect-memory cases
  passed in the preceding full run. Cache cleanup removed 1.6 GiB.
- Strengthened the dump assertion to inspect EffectParameterAccess itself,
  rather than the enclosing summary (which also prints the memory footprint).
  Final formatting passed `/tmp/luna-tools-tnow87gz/result.json`, and whitespace
  check passed. Focused final verification is recorded below.
- Final focused `effect_variant_parameter_access_checks_only_present_payloads`
  passed via development/check.py, exit 0; `/tmp/luna-tools-jicu_4c9/result.json`.
  Documentation describes conditional payload access and the remaining origin
  correlation limits. Changes remain uncommitted on dev.

### Stable variant-qualified pointer captures

Capture layout validation now accepts guarded scalar pointer leaves. Installation
uses its established enum domains to exclude impossible fields, validates the
remaining targets, and unions physical-slot candidates. Conditional roots of
unchanged original parameters reuse guarded source correlation. Before entering
the handler frame, only domain paths appearing as guards of pointer input leaves
are copied; unrelated mutable data tags are not frozen. Captured tags are
transported only after complete-scope pointer/tag stability checks succeed.
Nested guard paths retain their qualified prefixes. Capture objects and pointees
remain in their original ownership scopes.

The capture-domain snapshot and all retained paths are budgeted. Constructing
the capture analysis expression occurs once per capture, rather than cloning a
large aggregate type per pointer leaf. Generic unknown heterogeneous payloads
can still require finer correlation inside one handler body; known tags separate
those payloads. This supersedes the prior blanket variant-capture restriction.

Validation in `/home/fin/luna`:
- Basic `.none`/`.some` capture and dangling rejection passed
  `/tmp/luna-tools-2kngjhrp/result.json`, exit 0.
- Full effect-memory suite initially passed 59 tests,
  `/tmp/luna-tools-r5auwsbr/result.json`, exit 0.
- Extended mutable-data-tag case to both absent and present pointer payloads;
  focused run passed `/tmp/luna-tools-o0x33oxe/result.json`.
- Final full `python3 -B development/check.py --timeout 900 -- test -p luna --test effect_memory_transfers`
  passed 60 tests, exit 0; `/tmp/luna-tools-_xb01jhm/result.json`. New coverage
  includes nested record/enum captures, known bool/u32 variant payloads,
  unrelated mutable enum state across requests, tag rebinding/address rejection,
  and present dangling payloads.
- Formatting passed `/tmp/luna-tools-0h4ktqle/result.json`; whitespace check passed.
  Documentation now explains stable guard facts separately from captured values.
- Related integration: `python3 -B development/check.py --timeout 900 -- test -p luna --test drop --test resources -- handler`,
  108 drop + 4 resource tests passed, exit 0;
  `/tmp/luna-tools-ws5yp3kz/result.json`. Combined with the final full
  effect-memory suite, 172 distinct tests passed. Final whitespace check passed.
  Changes remain uncommitted on dev.

### Conditional pointer origins across patterns and local snapshots

Resource flow now stores necessary variant conditions per physical pointer slot
and candidate target. Generic borrowed input fields and handler operation inputs
seed those conditions; handler captures import existing evidence. Pattern binding
uses selected variant paths to eliminate contradictory candidates, independently
of whether the pattern copies or transfers its fields. Conditions are projected
onto bound values, including whole-value aliases. Ordinary local bindings snapshot
existing evidence before evaluation; later changes to the source do not alter the
copy. Overwritten slots, restored capture contents, loaded pointer contents, and
retired bindings invalidate old metadata. Control-flow joins retain only identical
conditions present in both incoming states. Missing metadata never permits target
exclusion. Request validation consumes available conditions before its older
unchanged-parameter fallback. This remains program verification, not PTC.

The implementation does not yet preserve this precision through arbitrary value
reconstruction or call results. It may conservatively reject heterogeneous unions
when the necessary conditions were lost. Target liveness, exact pointee type and
cleanup compatibility remain independent obligations.

Validation in `/home/fin/luna`:
- Full effect-memory suite passed 63 tests before the final invalidation/budget
  audit, `/tmp/luna-tools-mhdotvs0/result.json`, exit 0.
- Added direct local and `as`-pattern aliases to the heterogeneous typed-request
  matrix; existing cases cover snapshots surviving source replacement, replacement
  of the snapshot itself, and rejection of dangling replacement targets.
- Full workspace formatting passed `/tmp/luna-tools-qs31y9c8/result.json`.
- Broader integration command: `python3 -B development/check.py --timeout 900 -- test -p luna --test effect_memory_transfers --test match_cleanup --test resources --test drop`;
  result recorded below after completion. Documentation describes the supported
  propagation and remaining conservative boundaries. Changes remain on dev,
  uncommitted.
- Integration `/tmp/luna-tools-cc5np7yv/result.json` exited 101: 769 drop,
  63 effect-memory, 13 match-cleanup and 346 resource tests passed. One resource
  test, `recursive_memory_fixed_point_uses_a_worklist_for_long_call_chains`,
  exhausted its 3,000,000-unit memory-analysis budget. No baseline claim is made;
  this run exposed the issue in the current worktree.

### Dependency-driven memory summary convergence

The memory summary driver previously reevaluated every discovered body in each
round, consuming substantial fuel propagating a leaf footprint back through a
150-call chain. It now maintains a deduplicated pending queue and reverse caller
dependencies. Every consumed callee instance records its caller, including reads
of provisional bottom; changed summaries reschedule only dependent callers.
Self-recursion and mutually recursive calls use the same mechanism. Old dependency
edges remain conservative when callable candidates change. Instance identity still
includes effect contracts and prepared code inputs. All discovered bodies must be
checked and the pending queue must drain before returning a root summary; errors
and fuel exhaustion still fail the entire inference. Dependency registration and
rescheduling consume the shared budget. The original long-chain test budget and
low-budget rejection assertion are unchanged.

Validation in `/home/fin/luna`:
- Formatting passed `/tmp/luna-tools-pacetek5/result.json`, exit 0.
- `python3 -B development/check.py --timeout 900 -- test -p luna --test resources --test effect_memory_transfers --test match_cleanup --test drop`
  passed, `/tmp/luna-tools-8ir6yl6a/result.json`, exit 0. All 1,192 tests
  passed (769 drop, 63 effect-memory, 13 match-cleanup, 347 resources), including
  the unchanged long-call-chain test, recursive callback/aggregate summaries,
  bounded expanding type instances and failure recovery cases.
- Final whitespace check passed. No commits or pushes; changes remain on dev.

### Conditional pointer origins through assignment

Assignments now snapshot the RHS value's existing pointer conditions before
replacing storage, then install the snapshot after origin replacement invalidates
the old qualifiers. Both pointer-slot paths and enum-condition paths are rebased
under the destination field path; pointee addresses are unchanged. This preserves
ordinary assignment, self-assignment and field assignment without carrying stale
conditions from the previous value. Missing RHS evidence remains conservative.

The effect alias matrix now exercises whole-value assignment, self-assignment,
field assignment followed by projection, dynamic branch assignments with equal
conditions, and replacement of a field with a dangling target. Both bool and u32
payloads are checked by actual execution and typed effect access.

Validation in `/home/fin/luna`:
- Workspace formatting passed `/tmp/luna-tools-164y7fw6/result.json`.
- First targeted run `/tmp/luna-tools-gc_nbh0l/result.json` exited 101 due to a
  new fixture spelling `other.*==0`; the lexer requires a boundary between
  dereference and equality. Fixed it to `other.* == 0`; no implementation or
  semantic expectation was weakened. Final execution evidence follows below.
- Final `python3 -B development/check.py --timeout 900 -- test -p luna --test effect_memory_transfers --test resources -- assignment effect_aliases pointer_slots`
  passed, `/tmp/luna-tools-jm0polum/result.json`, exit 0: 1 expanded effect
  matrix plus 11 resource tests. The matrix executes 16 successful scenarios
  (8 setups × 2 payload types) and rejects two dangling replacement cases.
  Final whitespace check passed. Documentation updated; uncommitted on dev.

### Evaluated conditional origins for aggregate construction

Expression resource analysis now carries pointer conditions with the value being
delivered. Place reads snapshot established conditions; aggregate constructors
collect each field's evaluated conditions before evaluating subsequent fields,
prefixing physical pointer and tag paths. Tuple, record, anonymous-record and enum
construction share this path. Newtype wrap/unwrap remaps both classes of path.
Snapshots and result-bearing local/preparation blocks can transport this evidence.
Unsupported result forms explicitly clear nested operand evidence rather than
accidentally treating an operand's qualifiers as the enclosing result's. Divergent
paths clear the result evidence as well. Arbitrary call results and conditional
expression result joins remain conservative.

Pattern subjects, ordinary local initialization, assignment and operation request
checks consume evaluated qualifiers. Handler capture installation still queries
the captured place directly, not unrelated last-expression evidence. Existing
liveness and origin checks remain authoritative for actual targets.

The effect matrix additionally covers record wrapping/projection, tuple patterns,
newtype round trips, nested enum payload extraction, result-bearing blocks, and a
later aggregate field which rebinds the original variable after the first field
has been prepared. Existing assignment, self-assignment and dangling-replacement
cases remain in the same matrix.

Validation in `/home/fin/luna`:
- Full workspace formatting passed `/tmp/luna-tools-b95mzx6z/result.json`.
- `python3 -B development/check.py --timeout 900 -- test -p luna --test effect_memory_transfers --test resources`
  passed, `/tmp/luna-tools-q_yq15s3/result.json`, exit 0: 63 effect-memory and
  347 resource tests (410 total). The expanded alias matrix executes 28 successful
  scenarios and rejects two dangling replacements. Resources cover recursive
  summaries, indirect reads/writes, lifetime checks, patterns and cleanup.
- Final whitespace check passed. Documentation updated; uncommitted on dev.

### Conditional pointer evidence at expression joins

Normal results of `if` and `match` now join pointer conditions alongside origins.
For a target present on both sides, the result retains only common necessary
conditions. When a known opposite-side origin set excludes the target, its own
side's conditions remain valid. Missing origin slots are unknown and cannot prove
absence. The join considers only live result branches; return/divergence contributes
no delivered value. Match arms fold the same operation over successful exits.

Enum construction now gives each actual payload target its known outer tag in
addition to nested conditions. This allows a parameter value and a freshly
constructed variant to join without confusing heterogeneous payload storage.
The expression wrapper permits these explicitly joined results while continuing
to clear unadapted operand evidence. Lifetime, target type and ownership checks
remain separate from these tag conditions.

Expanded the effect alias matrix with dynamic if/match results, an early-return
branch, and parameter-versus-constructor alternatives in both orders. Existing
aggregate, assignment, self-assignment, evaluation-order and dangling-target cases
remain enabled. The matrix now has 40 successful executions and 4 rejection cases.

Validation in `/home/fin/luna`:
- Workspace formatting passed `/tmp/luna-tools-kqa0v097/result.json`, exit 0.
- `python3 -B development/check.py --timeout 900 -- test -p luna --test effect_memory_transfers --test resources --test match_cleanup`
  passed, `/tmp/luna-tools-_hfrq5sh/result.json`, exit 0: 63 effect-memory,
  347 resource and 13 match-cleanup tests (423 total).
- Final whitespace check passed. Documentation updated; uncommitted on dev.

### Pointer conditions through temporary field projection

Temporary projection now selects pointer qualifiers under its complete physical
field path and rebases both slot and condition paths to the selected value.
Conditions outside the selected subtree are conservatively discarded. The
expression wrapper permits the evidence explicitly produced by field projection;
place projections retain their existing snapshot path. Base evaluation, remaining
owner state, pending cleanup and lifetime checks use the existing projection code.

The alias matrix adds direct constructed fields, nested fields, newtype wrapping
of the aggregate, if/match aggregate results, a later sibling rebinding the source,
and a dangling-target replacement through a temporary field. It now covers 54
successful scenarios and 5 rejection cases. Relevant existing temporary-projection
tests check evaluation counts, sibling cleanup and non-extension of lifetimes.

Validation in `/home/fin/luna`:
- Formatting passed `/tmp/luna-tools-0300y_4i/result.json`, exit 0.
- `python3 -B development/check.py --timeout 900 -- test -p luna --test effect_memory_transfers --test resources -- effect_aliases temporary_projection`
  passed, `/tmp/luna-tools-kxtsq49w/result.json`, exit 0. The expanded effect
  matrix and selected temporary-projection resource regressions all passed.
- Final whitespace check passed. Documentation updated; uncommitted on dev.

### Conditional origins through named function summaries

Ordinary explicit call arguments now refine symbolic input-field targets using
the evaluated argument's established tag conditions before subsequent arguments
execute. Memory-summary results retain both the symbolic return leaf and its
physical storage path; mapping the leaf back to actual caller targets also builds
necessary return-tag conditions. If a single target occurs under several result
paths sharing a slot, only common conditions survive. Unknown targets remain
subject to ordinary rejection checks.

The call analysis returns qualifiers explicitly with origins. Named aggregate
calls publish that result after recording the call. Resolved single named locals
can forward the result; unadapted closure/candidate-set result paths remain
conservative. Pointer-only and generic expression fallbacks clear operand evidence
rather than accidentally publishing the last argument's qualifiers. Defaults keep
the existing conservative summary mapping.

The alias matrix adds identity forwarding, record wrapping/projection, newtype
forwarding, recursion, mixed parameter/constructed returns, and a later argument
rebinding the original source after an earlier argument was prepared.
Initial compilation `/tmp/luna-tools-9xhvre0q/result.json` failed on stale tuple
unpacking and a local name mismatch while changing the internal result shape;
those implementation errors were corrected before rerunning validation.

Validation in `/home/fin/luna`:
- Formatting passed `/tmp/luna-tools-nxkr3je1/result.json`, exit 0.
- `python3 -B development/check.py --timeout 900 -- test -p luna --test effect_memory_transfers --test resources`
  passed, `/tmp/luna-tools-2izrh3a5/result.json`, exit 0: 63 effect-memory and
  347 resource tests (410 total). The expanded alias matrix has 68 successful
  executions and 6 dangling replacement rejections. Recursive summaries, callable
  factories, prepared argument snapshots and ordinary memory calls also passed.
- Documentation updated and final whitespace check passed. Uncommitted on dev.

### Qualified aggregate returns from named candidate sets

Named candidate calls now return the qualifiers already computed from their
merged memory summary, instead of dropping that fourth result component. Local
function variables and temporary callee expressions transport this explicit
result. Temporary named callees retain their existing one-time argument preparation
and receiver cleanup; closure result evidence remains unadapted and conservative.
No candidate is selected statically just because it appears first.

The effect matrix adds keep-versus-replace candidate sets in both orders, both
stored and immediately invoked conditional functions, and later-argument mutation
with two forwarding candidates. It now exercises 78 successful scenarios and 8
rejections. Related resource tests check candidate completeness, evaluation order,
and expired-target rejection.

Validation in `/home/fin/luna`:
- Formatting passed `/tmp/luna-tools-ys65psi_/result.json`, exit 0.
- `python3 -B development/check.py --timeout 900 -- test -p luna --test effect_memory_transfers --test resources -- effect_aliases named_candidate temporary_named_aggregate`
  passed, `/tmp/luna-tools-uvhua1wa/result.json`, exit 0. The expanded effect
  matrix and all selected candidate/temporary aggregate resource tests passed.
- Final whitespace check passed. Documentation updated; uncommitted on dev.

### Qualified closure arguments, captures and aggregate returns

Closure argument preparation now refines symbolic input leaves from evaluated
conditions. Closure construction snapshots capture qualifiers into physical
environment slots; local storage and temporary receivers retain those qualifiers.
Capture input binding filters contradictory tag candidates against that saved
environment, not the subsequently modified lexical variable.

Closure result mapping shares the necessary-condition accumulator used by named
calls. Conditions survive only if valid for every occurrence of a target at a
result slot, including all closure candidates. Returned qualifiers are transported
explicitly through local and temporary closure calls. Capture writeback still
invalidates old qualifiers: repeated calls after writeback can remain conservative
until exit summaries transport fresh conditional evidence. No stale association
is reinstated merely because the target type matches.

Validation in `/home/fin/luna`:
- Formatting passed `/tmp/luna-tools-17vo710s/result.json`.
- `python3 -B development/check.py --timeout 900 -- test -p luna --test effect_memory_transfers --test resources`
  passed, `/tmp/luna-tools-x3a19jlk/result.json`, exit 0: 63 effect-memory and
  347 resource tests (410 total). New matrix cases cover closure parameters,
  generic captures, immediate invocation, capture snapshots after lexical
  reassignment, heterogeneous closure candidates, and dangling capture rejection.
  The matrix now executes 94 successful scenarios and 10 rejection cases.
- Existing closure cleanup, captured-state transitions, recursion and aggregate
  return tests also passed. Documentation updated; whitespace check passed.
  Changes remain uncommitted on dev.

### Conditional capture evidence on all invocation exits

Resource reports now snapshot capture pointer conditions alongside pointer targets
for normal, predicate-failure and effect-escape exits. Report append and snapshot
alignment checks include all three channels. Display prints deterministic
S-expressions with explicit slot, target, field-path and variant-condition nodes.

Capture restoration maps each exit's symbolic targets to caller storage and
rebases its condition paths into the receiver environment. Multiple occurrences
of the same actual target keep only common necessary conditions. Old environment
qualifiers are cleared before fresh evidence is installed. If an exit is missing
pointer evidence, the synthesized unknown remains unconditional; it cannot inherit
a tag condition attached to another occurrence of the unknown sentinel.

The shared necessary-condition accumulator now serves named returns, closure
returns and capture writeback. Added repeated invocation, captured-enum replacement,
and early-return capture cases. The alias matrix contains 100 successful scenarios
and 11 rejection cases. A separate report test checks structured guard dumps.

Initial selected regression `/tmp/luna-tools-z6fzmd6q/result.json` passed 216 drop
and 28 effect tests, but its dump fixture lacked caller pointer premises at its
analysis entry and was rejected before producing a report. Replaced that fixture's
entry with a concrete local target; generic captured inputs remain covered by the
execution matrix. The unknown-sentinel hardening was added before the final run.

Validation in `/home/fin/luna`:
- Formatting passed `/tmp/luna-tools-sg1il82i/result.json`, exit 0.
- `python3 -B development/check.py --timeout 900 -- test -p luna --test effect_memory_transfers --test resources --test drop -- effect_aliases capture_guard closure captured handler`
  passed, `/tmp/luna-tools-5au9xdep/result.json`, exit 0. Selected drop,
  effect-memory and resource regressions all passed, including the dump test.
- Documentation synchronized and final whitespace check passed. Uncommitted on dev.

### Conditional pointer evidence across effect delivery

Operation preparation now snapshots conditions together with every forwarded
parameter's origins. Possible forwarded values join by actual target using the
same necessary-condition join as expression branches. Result qualifiers are
published only on a live continuation after completion evidence is restored.
Effect request, installation, computation and run boundaries explicitly transport
the delivered value's evidence; they do not reuse the last argument's state.

Added direct forwarding, explicit resume, alternative input selection in both
orders, later-argument source mutation, and nested fallback to the alias matrix
(112 successful scenarios and 12 rejection cases).

Validation in `/home/fin/luna`:
- Initial `/tmp/luna-tools-uwtsoapm/result.json` exposed a missing transport path:
  computation/run wrappers cleared request-produced qualifiers. Added those
  existing transparent boundaries to the result-evidence propagation cases.
- `python3 -B development/check.py --timeout 900 -- test -p luna --test effect_memory_transfers --test drop -- effect handler`
  passed, `/tmp/luna-tools-mulnr0ko/result.json`, exit 0. All selected effect
  and handler cleanup regressions passed. Documentation synchronized.
- Final run passed 178 drop + 59 effect-memory tests (237 total). Formatting
  check passed `/tmp/luna-tools-yy0zzt_v/result.json`, exit 0; whitespace check
  passed. Changes remain uncommitted on dev.

### Default arguments preserve qualified prepared inputs

Verified the existing default-argument summary path with heterogeneous enum
payloads. It already retains symbolic fields while preparing defaults, including
chained record defaults and a checked pure supplier. No compiler change was
needed. The enlarged effect fixture now supplies Copy/Drop protocol identities at
compile time, because nominal copying in P3 defaults must be certified before
execution. Initial attempts without those identities were correctly rejected
(`/tmp/luna-tools-mgkfivrj/result.json`, `/tmp/luna-tools-bhu8l02c/result.json`).
A protocol-aware focused run passed `/tmp/luna-tools-5c5rjvco/result.json`.

Added explicit default override and expired-target cases, plus a separate test
requiring nominal Copy evidence during default validation. The alias matrix now
has 124 successful scenarios and 14 rejection cases. Coverage includes explicit
arguments changing original variables before default evaluation, dependent defaults,
and pure supplier calls. Updated the parameter documentation with an enum example
and the distinction between Copy permission and pointee lifetime checks.

Validation in `/home/fin/luna`:
- Formatting passed `/tmp/luna-tools-lcb5dr5f/result.json`, exit 0.
- `python3 -B development/check.py --timeout 900 -- test -p luna --test effect_memory_transfers --test resources -- effect_aliases default`
  passed, `/tmp/luna-tools-gs87aw40/result.json`, exit 0. The effect alias matrix,
  explicit nominal-Copy rejection test and selected default resource tests passed.
- Final whitespace check passed. Changes remain uncommitted on dev.

### Copy loads of pointer-bearing local aggregates

Connected runtime aggregate-load formation, local memory-summary field snapshots,
and resource transfer of pointer origins, tag conditions, enum domains and callable
metadata. All candidate storage roots must be present and have the expected type;
the delivered aggregate requires Copy evidence. Unknown field origins remain
unknown. Existing pointer-store restrictions are unchanged.

Extended the heterogeneous enum alias matrix with direct loads, nested field
addresses, snapshots before/after reassignment, and conditional storage addresses.
A separate test checks live Copy storage, expired aggregate storage and missing
Copy permission. Handler installation still excludes exposed pointer-bearing
owners: test-local addresses leave scope before the copied result crosses the
handler boundary. This is not an expansion of the handler borrowing contract.

Initial focused run `/tmp/luna-tools-a6pu74h7/result.json` exposed the old frontend
formation gate; fixed both dereference and dereferenced-field formation. The next
run `/tmp/luna-tools-malc_cov/result.json` passed 65 effect tests and exposed the
independent handler installation restriction; scoped temporary addresses without
relaxing that restriction. Neither failed run is counted as completed validation.

To reduce repeated compiler rebuilds, normal cache pressure now first removes
luna outputs, then elaboration only if needed, and finally dependencies if pressure
persists. Actual quota failures retain full cleanup and a single retry. The cache
policy tests passed (11 tests), `/tmp/luna-tools-1l_7ua8n/result.json`, exit 0.

Final validation in `/home/fin/luna`:
- `python3 -B development/check.py --timeout 900 -- test -p luna --test effect_memory_transfers --test resources`
  passed 413 tests (66 effects, 347 resources), exit 0,
  `/tmp/luna-tools-jyg1d7bj/result.json`. The alias matrix exercises 136 successful
  executions and 16 expired-target rejections within its one test.
- Actual post-build cache maintenance removed only luna outputs (26 files,
  1.4 GiB); elaboration was retained. No rebuild-time speedup is claimed yet.
- `python3 -B development/command.py --timeout 120 -- cargo fmt --all`
  passed `/tmp/luna-tools-snm3aya_/result.json`, exit 0. Final `git diff --check`
  passed. Changes remain uncommitted on dev.
- External/unknown aggregate storage and aggregate pointer stores retain their
  existing conservative boundaries; this is local Copy-load support.

### Immediate Copy loads from closure captures

The previous turn made concrete implementation and validation progress. Continued
from the current dev worktree by exercising aggregate-load results across named
returns and closure calls. Named return and mixed pointer/callable-field loads
already worked; direct captured `value.ref.*` was rejected by the blanket capture
address validator (`/tmp/luna-tools-jdq36bc6/result.json`).

Permit only a syntactic AddressOf consumed directly by ReadPointer, with matching
result type and supported readable layout. Continue traversing the addressed value
and keep ordinary resource Copy/storage checks. Saved or escaping capture addresses
retain the existing environment-contract requirement. No arbitrary captured
pointer writes are authorized.

Added four alias-matrix routes (named loader, direct capture loader, nested loader,
replacement target), mixed callable-field reads with multiple named candidates,
and explicit captured-load Copy/lifetime rejection cases. Updated memory docs.

Validation in `/home/fin/luna`:
- `python3 -B development/check.py --timeout 900 -- test -p luna --test effect_memory_transfers --test resources -- effect_aliases aggregate_pointer_load capture closure`
  passed 97 tests (19 effects, 78 resources), exit 0,
  `/tmp/luna-tools-_r4ngy6g/result.json`. This includes the existing capture storage
  address rejection test. The expanded alias matrix covers 144 valid executions
  and 17 expired replacement-target rejections.
- `python3 -B development/check.py --timeout 900 -- test -p luna --test effect_memory_transfers -- captured_aggregate_load`
  passed the added test's live Copy, missing Copy and expired pointee cases,
  exit 0, `/tmp/luna-tools-yypg5m9i/result.json`.
- Formatting passed `/tmp/luna-tools-9j43t377/result.json`; whitespace check passed.
  Changes remain uncommitted on dev. Broader captured-address contracts and the
  complete language implementation remain unfinished.

### Callable identity snapshots for pointer loads

Previous turn made implementation and validation progress. Re-examined current
aggregate loads at the source-summary boundary: resource-checked execution passed,
but an explicit summary assertion showed returned function fields were Unknown
(`/tmp/luna-tools-p2n9_2ty/result.json`, expected regression failure).

ReadPointer now snapshots local prepared-code candidates and named callable
contracts from the addressed subtree, rebases field paths and joins every possible
storage candidate. Missing candidates do not inherit another path's known code.
Scalar function-slot loads also publish callable_result metadata. Only layouts
containing callable leaves scan callable storage maps; ordinary scalar loads avoid
that work. All snapshots and joins remain budgeted; external/indirect unknown
storage stays conservative.

Tests assert returned candidate identities and transitive read footprints, not
only interpreter results. Coverage includes mixed pointer/callable records, two
known named functions, multiple storage addresses, an unknown callable input and
a scalar function-slot result. Existing resource checks and stores are unchanged.
Updated memory-effects documentation.

Validation in `/home/fin/luna`:
- Targeted callback/callable/capture tests passed 46 cases,
  `/tmp/luna-tools-7k7mj_vd/result.json`, exit 0.
- `python3 -B development/check.py --timeout 900 -- test -p luna --test effect_memory_transfers --test resources`
  passed 417 tests (70 effects, 347 resources), exit 0,
  `/tmp/luna-tools-4thkyy2p/result.json`.
- Formatting passed `/tmp/luna-tools-4kaismo6/result.json`, exit 0.
  Final `git diff --check` passed. Changes remain uncommitted on dev.

### Loaded callable snapshot timing and lifetime regressions

Previous turn completed callable-load summary changes. Tested the current dev
implementation rather than assuming candidate retention implies correct timing.
Added six cases covering whole-record replacement before/after loading, callback
field replacement before/after loading, and nested-field storage. Each asserts the
exact returned function declaration and pointer input index, then executes through
a second function and checks the boolean result. Six paired cases substitute an
expired pointer for the selected pointee and require resource rejection.

All cases passed without compiler changes. Added a concrete documented callback
snapshot example explaining pre-load vs post-load replacement.

Validation in `/home/fin/luna`:
- `python3 -B development/check.py --timeout 900 -- test -p luna --test effect_memory_transfers -- loaded_callable_snapshots`
  passed the matrix (6 positive executions, 6 lifetime rejections), exit 0,
  `/tmp/luna-tools-ftk28vhr/result.json`.
- Formatting passed `/tmp/luna-tools-mn7hduz_/result.json`, exit 0.
  Final whitespace check passed. Changes remain uncommitted on dev.

### Higher-order callback writes through loaded aggregate results

Previous turn added verified snapshot/lifetime regressions. Continued with current
source by passing callback parameters into pointer-bearing aggregates, loading and
returning them, forwarding through another ordinary function, then invoking the
returned callback field. Direct loads, saved loaded values and closure-based loads
all preserve write effects. Each was tested with named, anonymous and conditional
callbacks. Explicit source summaries must include a write to input 0; execution
must observe the mutation, and expired input storage must be rejected.

The existing compiler passed this expanded matrix without changes. Added a full
higher-order example and contract explanation to memory-effects documentation.

Validation in `/home/fin/luna`:
- Initial named-callback cases passed `/tmp/luna-tools-gg60o9dv/result.json`.
- `python3 -B development/check.py --timeout 900 -- test -p luna --test effect_memory_transfers -- loaded_callback_parameters`
  passed the expanded matrix (9 executions plus 9 expired-target rejections),
  exit 0, `/tmp/luna-tools-qapwvchh/result.json`.
- Formatting passed `/tmp/luna-tools-h85f_3pv/result.json`, exit 0.
  Final whitespace check passed. Changes remain uncommitted on dev.

### Parameter-relative aggregate pointer loads

Previous turn added verified higher-order write-contract tests. Advanced beyond
local storage: a `load(p:*Link)->Link {p.*}` regression failed because result field
origins were unknown (`/tmp/luna-tools-phvkhkan/result.json`).

Aggregate reads now export an indirect ParameterPlace for unconditionally present
pointer fields reached through parameter storage. Build each projection using its
actual field owner and instantiated owned-field type, record a typed pointer-slot
read, then append the load to the source path. Existing resource mapping resolves
that path at the call site and checks both aggregate storage and loaded pointees.
This applies to structs, nested structs, newtypes and Tuple without unfolding
normal-form identity. Variant-conditioned paths remain unknown until a separate
guarded-load contract exists; external callable identities are not fabricated.
Raw aggregate pointer stores remain unchanged.

Validation in `/home/fin/luna`:
- The original direct-load execution passed `/tmp/luna-tools-pvfs7zyk/result.json`.
- Expanded regression checks exact parameter-relative indirect return paths,
  ordinary forwarding, four valid layouts, four expired aggregate-storage cases
  and four expired pointee cases.
- `python3 -B development/check.py --timeout 900 -- test -p luna --test effect_memory_transfers --test resources`
  passed 420 tests (73 effects, 347 resources), exit 0,
  `/tmp/luna-tools-zofqaib5/result.json`.
- Formatting passed `/tmp/luna-tools-uea81lyd/result.json`, exit 0.
  Final whitespace check passed. Updated documentation and example. Changes
  remain uncommitted on dev; full Flurry implementation remains unfinished.

### Multi-level aggregate-load storage obligations

Previous turn implemented parameter-relative aggregate loads. Rechecked the
current dev source with two and three pointer layers and a two-address selection.
Asserted exact indirect path depth and candidate cardinality in return summaries;
execution checks that replacing the original aggregate after the call leaves its
returned copy pointing at the original target. Added four rejected programs with
expired aggregate/intermediate slots, asserting unavailable-storage diagnostics.
No compiler change was necessary; documented per-layer obligations.

Validation in `/home/fin/luna`:
- Initial three positive scenarios passed `/tmp/luna-tools-gc7rnwbi/result.json`.
- `python3 -B development/check.py --timeout 900 -- test -p luna --test effect_memory_transfers -- indirect_aggregate_load`
  passed three test functions (including the earlier four-layout matrix), exit 0,
  `/tmp/luna-tools-bd3tw1v9/result.json`. This turn adds three positive executions
  and four expired-slot rejections.
- Formatting passed `/tmp/luna-tools-9dzot2n4/result.json`, exit 0.
  Final whitespace check passed. Changes remain uncommitted on dev.

### Bounded callable subtree queries; guarded-load boundary audit

Previous turn added verified multilevel-load regressions. Audited why parameter
loads of enum payloads remain unsupported: ParameterPlace through/field segments
carry layout owners and indices but no variant condition. Physical payload slots
can overlap across tags with different types. Correct support must transport tag
conditions with loaded result origins, preserve absent payloads, and enforce
conditional initialization when mapping caller storage. Removing the current
variant-path rejection would be unsound; it remains unchanged. This work is not
claimed as implemented enum support.

Found repeated full-environment scans in callable subtree extraction, including
aggregate pointer reads. Added a common lexicographic BTreeMap subtree iterator;
local prepared-code projection, named-code projection and both load snapshots now
start at the selected root/path and stop at its first sibling. Unrelated roots and
fields no longer consume the extraction budget. Relevant field copies and joins
remain charged. No runtime speedup measurement is claimed.

Added direct/proxied subtree reads with 32 unrelated callback-bearing roots,
following roots and distinct sibling callback/pointer identities. Assert exact
returned declaration and pointer input, plus execution results.

Validation in `/home/fin/luna`:
- `python3 -B development/check.py --timeout 900 -- test -p luna --test effect_memory_transfers --test resources -- callable callback loaded indirect_aggregate`
  passed 54 tests (12 effects, 42 resources), exit 0,
  `/tmp/luna-tools-q4ohmbmc/result.json`.
- Formatting passed `/tmp/luna-tools-3upwcphm/result.json`, exit 0.
  Final whitespace check passed. Changes remain uncommitted on dev.

### Typed conditional storage projections (enum-load foundation)

Previous turn implemented bounded subtree lookup. Audited all indirect-memory
consumers: source path composition, caller mapping, flow facts and Drop access
validation must preserve tag conditions together. An unguarded physical payload
index cannot express this contract.

Added StorageProjection and project_owned_storage: resolve symbolic owned paths
into typed physical fields plus ordered enum-container/tag guards. Nested variant
paths validate each declaration, variant and field before selecting its type.
The result has a stable S-expression Display. Integrated the resolver into source
aggregate-load projection, replacing the ad hoc ordinary-field conversion. Only
guard-free projections are currently published as indirect loads; guarded paths
remain unknown until call-site/cleanup consumers can discharge their conditions.
This is an implemented representation step, not completed enum indirect loading.

Tests demonstrate identical physical slots with different guard conjunctions and
pointee types; reject missing markers, missing payload fields, bad tags/indices
and exhausted fuel. Anonymous layouts work without package metadata, nominal
layouts require it. Nested dump structure is checked.

Validation in `/home/fin/luna`:
- `python3 -B development/check.py --timeout 900 -- test -p luna --test effect_memory_transfers --test resources -- conditional_storage indirect_aggregate callable callback loaded`
  passed 55 tests (13 effects, 42 resources), exit 0,
  `/tmp/luna-tools-umgn913s/result.json`.
- After adding package-optional layout cases, the focused conditional projection
  test passed `/tmp/luna-tools-4oaefnsf/result.json`, exit 0.
- Formatting passed `/tmp/luna-tools-dewyn2cp/result.json`; whitespace check passed.
  Changes remain uncommitted on dev.

Next: carry these conditions through ParameterPlace composition and all consumers.
Conditions in a suffix's first load are relative to the base's current final
storage, so both field and guard paths need prefixing; later loads use their own
pointee roots. Inactive payloads must contribute no access or pointer target, and
unknown tags must preserve conditional target/type correlation rather than select
one variant. Call-site mapping, inferred borrow initialization, flow observations
and Drop footprint validation all require corresponding handling before enabling
production of guarded indirect results.

### Composing conditional projections within a storage root

Previous turn implemented typed conditional projection resolution. Added
StorageProjection::then and a shared checked extension routine. Continuing a
projection preserves existing guards and roots new enum guards at the accumulated
physical prefix. Single-step resolution uses the same walker. The combined
symbolic depth (ordinary fields plus two extra marker/tag components per enum
selection) remains bounded across calls, so splitting a path cannot evade limits.
The operation clones only after budget charges and returns a new projection;
failed traversal never modifies the original, while spent fuel is not restored.
No pointer dereference is implicit in this operation.

Tests compare every valid split of two nested payload paths with whole-path
resolution and exact dumps, check a nonzero anonymous-record prefix, reject
continuation through a pointer without a load, and verify failure/recovery for
malformed suffixes, zero budget and combined-depth exhaustion.

Validation in `/home/fin/luna`:
- `python3 -B development/check.py --timeout 900 -- test -p luna --test effect_memory_transfers --test resources -- conditional_storage indirect_aggregate loaded`
  passed 7 tests, exit 0, `/tmp/luna-tools-784h31q9/result.json`.
- Expanded nonzero-prefix and pointer-boundary cases passed the focused projection
  test, `/tmp/luna-tools-kkz28i9w/result.json`, exit 0.
- Formatting passed `/tmp/luna-tools-xgvpvv0v/result.json`; whitespace check passed.
  Changes remain uncommitted on dev. Conditions across pointer-load boundaries
  and their call-site/cleanup consumers remain the next required implementation;
  enum indirect loading is not yet enabled.

### Typed traversals across pointer-load boundaries

Previous turn implemented same-root composition. Added StorageTraversal with
ordered load projections and a separate final target. Each load retains its
slot's typed fields/guards while starting the next projection at a fresh pointee
root. Explicit then/load operations check one aggregate depth limit across all
stages and mutate the traversal only after successful validation. They do not
prove tag conditions or ownership. Display emits nested S-expression load and
target nodes.

Integrated the new traversal into cleanup footprint type validation, retaining
field-owner and pointer-load type checks. Existing unguarded cleanup contracts
now exercise this code; guarded paths are still not emitted by source summaries
or authorized at runtime. This remains infrastructure toward enum indirect loads.

Added nested-enum pointer traversals, cross-root guard separation, segmented vs
combined projection, failed-load recovery, non-pointer load rejection, structured
dumps and recursive pointer-depth exhaustion (31 valid hops followed by rejection).

Validation in `/home/fin/luna`:
- `python3 -B development/check.py --timeout 900 -- test -p luna --test effect_memory_transfers --test resources --test drop -- conditional_storage indirect memory`
  passed 234 tests (70 drop, 7 effects, 157 resources), exit 0,
  `/tmp/luna-tools-6xwx43x_/result.json`.
- Added explicit segmented-traversal equality checks then reran the focused test;
  `/tmp/luna-tools-_dnwr_d7/result.json`, exit 0.
- Formatting passed `/tmp/luna-tools-hboecynh/result.json`; whitespace check passed.
  Changes remain uncommitted on dev. Next work still includes transporting guards
  in published parameter paths and discharging them against actual caller states;
  the full language goal and enum indirect-load support remain incomplete.

### Guard activity and absent-payload joins

Previous turn implemented typed cross-load traversal. Added GuardActivity
(active/inactive/conditional), stable Display, storage_guard_activity and
StorageProjection::activity. Nested domain queries include every enclosing
variant selection instead of consulting an ambiguous physical slot. Inactive
outer conditions short-circuit; unknown conditions remain conditional. Ordered
ancestor paths and budgets are checked. Aggregate Copy reads now use this common
condition evaluator to exclude inactive leaves.

New nested Envelope reads exposed a real join failure: the absent variant's
missing physical origin slot erased the present variant's inner pointer/type
conditions (`/tmp/luna-tools-gvpjuopf/result.json`, 16 passed, 1 failed).
Fixed join_result_pointer_guards to accept optional typed enum-domain evidence.
If a slot is missing on the other branch, retain conditions only when the complete
pointer layout proves EVERY leaf sharing that physical slot inactive. Unknown or
incomplete evidence keeps the previous conservative behavior. If, match and
multi-address load joins supply this evidence. No empty slots are installed into
live storage merely to represent absence.

Tests cover nested guards, unknown outer domains, short-circuiting, malformed
ancestor chains, budgets, reversed branches, match joins and multi-address loads.
The alias matrix now has 154 successful executions and 17 expired-target cases.
Updated memory-effects documentation.

Validation in `/home/fin/luna`:
- `python3 -B development/check.py --timeout 900 -- test -p luna --test effect_memory_transfers --test resources --test match_cleanup`
  passed 439 tests (79 effects, 347 resources, 13 match cleanup), exit 0,
  `/tmp/luna-tools-sq9vbeyv/result.json`.
- After adding reverse/match/address alternatives, focused alias and guard tests
  passed `/tmp/luna-tools-5jkkwvmw/result.json`, exit 0.
- Formatting passed `/tmp/luna-tools-gkpre_d9/result.json`; whitespace check passed.
  Changes remain uncommitted on dev.

Remaining: published parameter paths still lack conditional-load transport, so
full enum indirect loading remains disabled. Also, effect forwarded-result joins
currently pass None for enum-domain evidence; transporting per-argument domain
snapshots there is a concrete next integration point for absent-payload results.

### Per-input enum domains for forwarded effect results

Previous turn implemented guard activity and absent-payload joins. Reproduced the
remaining effect join failure with choose_envelope returning either a nested
pointer payload or an absent variant (`/tmp/luna-tools-lthuq862/result.json`).

Effect argument preparation now snapshots enum domains for forwarded candidates
alongside origins and qualifiers. Guard joins receive those snapshots; origin
joins use the existing typed/domain-aware merger instead of unconditionally
unioning known subsets. This preserves true absence and retains unknown targets
where a live pointer slot lacks provenance. Later argument mutations do not alter
already captured evidence. Runtime result restoration and ownership transfer
remain separate; no pointer slots are synthesized for absent-only results.

Added four alias routes: present/absent in both orders, a later argument mutating
the original enum, and a replacement pointer source with an expired-target case.
Added an absent-only effect result test. The alias matrix now covers 162 valid
executions and 18 expired-target rejections. Updated effect documentation.

Validation in `/home/fin/luna`:
- `python3 -B development/check.py --timeout 900 -- test -p luna --test effect_memory_transfers --test drop -- effect_ handler_`
  passed 203 tests (149 drop, 54 effect), exit 0,
  `/tmp/luna-tools-ole6n8w1/result.json`.
- Formatting passed `/tmp/luna-tools-7m1dr3zk/result.json`; whitespace check passed.
  Changes remain uncommitted on dev. Guarded parameter-path transport and complete
  enum indirect loading remain outstanding; full Flurry is not complete.

### Validation of transported conditional storage paths

Added StorageTraversal::then_guarded to reconstruct typed projections from
physical fields and suffix-relative payload guards. It validates declaration
owners, tag and payload bounds, ordered ancestor prefixes, and traversal budgets
before publishing the result. Pointer loads retain conditions on the previous
root. Invalid paths leave traversal state intact. Existing cleanup footprint
validation now uses this same entry point with no guards.

Regression covers both differently typed variants sharing physical slots,
nonzero-prefix composition, load boundaries, forged owners, absent/invalid/
duplicate/reversed/trailing guards, exhausted fuel and recovery.

Validation in /home/fin/luna:
- python3 -B development/check.py --timeout 900 -- test -p luna --test effect_memory_transfers --test resources --test drop -- conditional_storage indirect memory
  exit 0; /tmp/luna-tools-w7wfpyy2/result.json.
- Full workspace formatting exit 0; /tmp/luna-tools-ura00i10/result.json.
- git diff --check passed. Changes remain uncommitted on dev.

This validates transported projections; ParameterPlace still lacks guarded path
transport and caller-side conditional provenance consumption. Full enum indirect
loading is not enabled by this change.

### Conditional enum payloads across memory calls

Reproduced a two-hop enum Copy read losing pointer provenance (MissingPlace,
/tmp/luna-tools-jxcwwj_d/result.json). ParameterPlace and internal pointer paths
now carry conditions indexed by load stage. Composition rebases only the first
stage under the current field prefix; subsequent stages start at their loaded
root. Identity, budgets, S-expression dumps, local-slot resolution and acquired
cleanup serialization retain those conditions.

The resource consumer reconstructs typed guarded paths, filters inactive or
incompatible candidates, and checks the complete enum owner because ownership
keeps enum payloads opaque until pattern decomposition. Borrowed analysis uses
separate conditional pointee premises for each tag-qualified slot, without
installing unconditional physical payload slots. Return-field origin qualifiers
also preserve proven payload absence through subsequent by-value calls.

The flow optimizer returns unknown for conditional paths where tag-aware range
observations or selective write invalidation have not been implemented. Drop
footprint consumption still rejects conditional paths without tag evidence;
serialization alone does not establish that evidence.

Initial source matrix passed /tmp/luna-tools-4qz2a4pg/result.json: 20 valid
executions across Choice, **Choice, Holder and Envelope, 8 expired-target
rejections, 2 expired-owner rejections and a post-read mutation snapshot.
Additional direct-callee consumption coverage and broad related regression are
being validated below. The interrupted check /tmp/luna-tools-o5wblbzj/result.json
has no final result and is not passing evidence.

Broad regression initially aborted in the existing fixed-2-MiB
optional_inline_call_chains_preserve_defaults_contracts_and_owned_returns test
(/tmp/luna-tools-50cl79bb/result.json). An initial read-arm extraction did not
suffice (/tmp/luna-tools-ge_3b2kz/result.json). Optional GDB was unavailable due
to ptrace restrictions (/tmp/luna-tools-al830am3/result.json); no escalation was
needed. Inspection of the actual test binary's prologue showed memory::Infer::walk
reserving 0x13770 bytes per recursive frame. Split lexical blocks/conditionals
and pointer-read snapshots from the operation dispatcher, preserving depth/fuel
checks and the test's original stack limit. Verification follows.

Final validation in /home/fin/luna:
- Fixed-stack inline cleanup regression plus enum loading tests passed
  /tmp/luna-tools-k8f2nhub/result.json (4 tests, exit 0).
- python3 -B development/check.py --timeout 900 -- test -p luna --test effect_memory_transfers --test resources --test drop --test match_cleanup
  passed 1,214 tests: 769 Drop, 85 effect-memory, 13 match-cleanup, 347 resources;
  /tmp/luna-tools-2a8x9lli/result.json, exit 0.
- python3 -B development/check.py --timeout 900 -- test -p elaboration --lib -- pointer_root_tests
  passed all 3 tests; /tmp/luna-tools-8nnatsz1/result.json, exit 0.
- Full workspace formatting passed /tmp/luna-tools-ouxa8xc8/result.json;
  git diff --check passed.

The four new source tests cover 28 successful executions and 11 expired-storage
rejections. This includes reading the copied enum inside the callee, absent
payloads, differently typed payloads sharing slots, nested wrappers, independent
conditions across two loaded roots, expired intermediate enum storage, and
snapshot preservation after replacing the source enum. Added a composition unit
test for stage rebasing, condition-sensitive identity and failure atomicity.
Documentation now includes a runnable-style Copy enum forwarding example and
states remaining Drop/optimizer limitations. Changes remain uncommitted on dev;
Flurry as a whole remains incomplete.

### Tag-qualified Drop access and retained-value snapshots

Drop's pointer snapshot now includes enum domains and per-target conditions.
Its access checker reconstructs guarded typed projections at each load stage,
filters excluded payloads and incompatible targets, and validates whole enum
owners without inventing unconditional payload nodes. Existing target type,
availability and prior-retirement checks remain in force.

Active temporaries and prepared values now retain their own enum domains and
pointer qualifiers. Projection keeps the original aggregate snapshot; retained
callee environments are captured before their temporary root is removed;
pattern residuals preserve the subject's evidence. Cleanup imports those facts
under its canonical temporary root alongside surrounding pointer storage.

New source coverage includes direct and indirect enum payload reads in Drop,
different pointer types sharing a slot, unknown tags, plain temporaries, retained
projection siblings after subsequent source replacement, and expired owners or
targets. The prepared-argument test injects a runtime contract rejection after
source verification, installs checked Copy permission in the reference VM, and
checks that the retained owner is dropped exactly once. A cleanup-order case
rejects a target invalidated by an earlier destructor. Targeted conditional Drop
checks pass /tmp/luna-tools-sq_3nyrr/result.json. Broad validation follows.

Final validation in /home/fin/luna:
- python3 -B development/check.py --timeout 900 -- test -p luna --test drop --test resources --test effect_memory_transfers --test match_cleanup
  passed 1,218 tests (773 Drop, 85 effect-memory, 13 match-cleanup, 347 resources),
  /tmp/luna-tools-fl320vgq/result.json, exit 0. This includes the original fixed
  2-MiB inline-contract-cleanup regression.
- Full workspace formatting passed /tmp/luna-tools-5q6rksv1/result.json;
  git diff --check passed.

The new Drop tests cover 20 successful source executions, 2 runtime rejection
executions with exactly one retained-owner destructor, and 5 rejected expired or
previously retired storage cases. Updated memory-effects and drop-elaboration
documentation. Changes remain uncommitted on dev. Conditional range-observation
and loop-write precision are still conservative. An additional useful audit is
custom destructors nested inside outer variant cleanup branches: planner branch
premises must stay distinct from physical field layout and surrounding tag facts.

### Enclosing variant premises in nested Drop

Reproduced a valid nested destructor failing with "Drop pointer target type
disagrees with slot pointee" (/tmp/luna-tools-4q2u7nf8/result.json): two outer
variants share physical slots but store BoolWatch and CountWatch, whose inner
payload pointers have different types. The planner carried selected field types
but lost the corresponding outer tag at the access boundary.

Scoped variant layouts now retain both index and fields. Access projection
inherits enclosing variant conditions even when its own field path has no enum
step. Conditional domain queries qualify the base path with enclosing tags;
candidate filtering combines these premises with the local payload guards.
The planner's existing save/restore keeps branch evidence scoped, and ordinary
unguarded accesses with no active variant context retain their direct path.

New source tests exercise 26 valid executions: named owners, temporaries, two
nested enum levels, absent inner payloads, and ordinary pointer-field Drop bodies.
Two rejection cases select an expired bool/u32 target. Initial targeted checks
passed /tmp/luna-tools-co0sibay/result.json; final full Drop validation follows.

Final validation in /home/fin/luna:
- python3 -B development/check.py --timeout 900 -- test -p luna --test drop
  passed all 775 tests, /tmp/luna-tools-q9utkx7y/result.json, exit 0, including
  the fixed-2-MiB inline-contract cleanup test and all prior conditional Drop cases.
- Full workspace formatting passed /tmp/luna-tools-va963p4s/result.json;
  git diff --check passed.

Changes remain uncommitted on dev. Conditional memory range-observation and
loop-write precision are still conservative; the overall language goal remains
unfinished.


### Conditional return origins in range analysis

The program-verification flow walker now resolves tag-qualified return origins of readonly calls from current local aggregate snapshots. Each load stage checks its own root's tags; inactive alternatives are omitted, while missing/unknown tags, stale pointer frames, external conditional storage, writes, and possible cleanup remain conservative. Unconditional loads cannot borrow a conditional access's pointer type. This is program-state analysis, not PTC.

Validation in `/home/fin/luna`: `python3 -B development/check.py --timeout 900 -- test -p elaboration --lib conditional_origin_tests` passed (one test exercising active/inactive/unknown tags, two loaded roots, expired frames, missing snapshots, writes/cleanup, malformed stages and exhausted budget), report `/tmp/luna-tools-je67lvkg/result.json`. `python3 -B development/check.py --timeout 900 -- test -p luna --test resources conditional_pointer_result_ranges` passed (seven source cases through a two-function forwarding chain, including rejection of stale-value proofs), report `/tmp/luna-tools-cnsg68yy/result.json`. Whole-workspace formatting passed, report `/tmp/luna-tools-zdvne7is/result.json`; `git diff --check` passed.

The focused source-test build took 155 seconds and its post-build cache maintenance removed 1.4 GiB of luna artifacts after reaching the 2 GiB cap. Prefer one affected test-target invocation over a focused run immediately followed by the same whole target; do not assume the executable survives post-build cache cleanup.

The complete affected resource test target passed: `python3 -B development/check.py --timeout 900 -- test -p luna --test resources`, **348 passed**, report `/tmp/luna-tools-pvnwnhss/result.json`. Changes remain uncommitted on dev.


### Saved conditional addresses and nested source paths

Added 17 source-level cases in resources.rs: nine cases distinguish saved addresses from later retagging and direct/aliased/callee writes, while eight cases cover both expected and wrong values through nested enum payloads, two pointer-load stages, and record-contained enums. They query the check's input proof, so an `assumes` cannot make its own assertion appear already proven. No additional compiler semantics were needed for these paths. The memory-effects documentation now states the saved-address versus current-value distinction.

Validation in `/home/fin/luna`: `python3 -B development/check.py --timeout 900 -- test -p luna --test resources` passed **350 tests**, report `/tmp/luna-tools-mi88t1ex/result.json`. The earlier run `/tmp/luna-tools-7qet0_rt/result.json` had 349 passes and one test-source parse failure caused by `.*=23`; corrected to `.* = 23` before the successful run. Formatting passed `/tmp/luna-tools-7vj05fd5/result.json`; `git diff --check` passed after the correction. Changes remain in the dev worktree without commit.


### Invariant conditional return paths in loop write analysis

`flow::call_memory_plan` now resolves conditional return paths of readonly pointer factories against loop-stable local aggregate snapshots. `invariant_conditional_targets` checks every tag container and loaded pointer root against the complete possible-write set, observes each loaded root's own tag, and preserves a final destination even when that destination is written. Unknown tags, external origins, changed containers/slots, malformed stages, stale frames and exhausted budgets provide no precise result. Conditional accesses in writing callees remain conservative: pointer-storage preservation alone does not establish tag timing. Existing physical owner checks are shared with ordinary conditional snapshot projection.

Added source regressions for direct and forwarded factories in loops, and tag reassignment before and after use; added internal assertions distinguishing destination writes from intermediate-slot writes and rejecting exhausted budgets. Validation in `/home/fin/luna`: `python3 -B development/check.py --timeout 900 -- test -p elaboration --lib conditional_origin_tests` passed, report `/tmp/luna-tools-7lkxbizo/result.json`; `python3 -B development/check.py --timeout 900 -- test -p luna --test resources` passed **351 tests**, report `/tmp/luna-tools-ygosrknd/result.json`. Formatting passed `/tmp/luna-tools-oeu8ny4k/result.json`; `git diff --check` passed. Documentation updated; changes remain uncommitted on dev.


### Conditional scalar writers and composed loop factories

Extended conditional return-origin and loop-footprint mapping to callees whose complete write summary is limited to Bool/fixed-integer storage. The existing preservation check excludes both pointer-cell and enum-tag replacement; it does not preserve scalar value facts. `invariant_scalar_write_plan` now resolves guarded paths using the same loop-stability checks as return paths. Default computations still require their existing P3/resource evidence. Unknown calls, possible cleanup, aggregate replacement and pointer-slot writes retain conservative handling.

Ten source cases cover direct conditional scalar writes, a writing pointer factory, aggregate-wrapped results, readonly default factories, two loaded roots, and retagging/slot changes. Each executes at most two iterations: five valid cases run successfully, and five counterexamples fail with ProgramContractRejected; static input proofs agree. Earlier loop regressions now also use two terminating iterations so retagging after the first use has an executable second-iteration counterexample. The test frontend helper accepts explicit Copy/Drop selectors; ordinary tests retain empty selectors. The initial fixture lacked the resource boundary required for nominal reads in P3 defaults (reports `/tmp/luna-tools-9ul2mn1e/result.json` and `/tmp/luna-tools-cz7qaork/result.json`); the final fixture binds both protocols and supplies nominal Copy implementations, without relaxing purity checks.

Validation in `/home/fin/luna`: `python3 -B development/check.py --timeout 900 -- test -p elaboration --lib conditional_origin_tests` passed, report `/tmp/luna-tools-q_r2357f/result.json`; `python3 -B development/check.py --timeout 900 -- test -p luna --test resources` passed **352 tests**, report `/tmp/luna-tools-fm33v_lh/result.json`. Formatting passed `/tmp/luna-tools-jxf8vvfq/result.json`; `git diff --check` passed. Formal memory-effects documentation updated. Work remains uncommitted on dev.


### Field-specific tag and slot stability

Conditional loop path resolution now checks each actual tag-container path and pointer-slot path instead of requiring the whole loaded root to remain unchanged. Tag checks reject container/ancestor replacement but permit descendant payload writes; slot checks reject overlapping writes in either direction. Ordinary invariant slot loading shares the overlap helper. This preserves independent sibling fields without using stale tags or pointer contents.

Twelve source cases cover owned enum fields and enum-pointer fields, direct/callee sibling updates, field replacement before/after the first use, and whole-parent replacement. Six preserve the intended proof and execute successfully; six lack the proof and fail the runtime contract during the terminating two-iteration execution. Source resource suite: `python3 -B development/check.py --timeout 900 -- test -p luna --test resources`, **353 passed**, report `/tmp/luna-tools-j_u8bkjb/result.json`. Internal conditional tests initially passed `/tmp/luna-tools-k8rrcds4/result.json`; final internal rerun passed `/tmp/luna-tools-vx3799ts/result.json` after removing an incidental Tuple-as-anonymous-record owner extension. That extension needs coordinated tuple pointer read/write support: flow/nominal.rs currently restricts anonymous place fields/read projection to Record, so changing snapshot lookup alone is insufficient. The new source paths use nominal Holder/Choice fields and do not depend on that removed branch.

Working directory `/home/fin/luna`; formatting passed `/tmp/luna-tools-bt0l78b7/result.json`, final `git diff --check` passed. Memory-effects documentation updated. Changes remain uncommitted on dev.


### Typed tuple storage paths in program flow

Tuple range values now retain their declared type through unknown values, evaluated tuples and prepared tuple inputs. `place_fields` and anonymous physical-field reads obtain tuple views through the existing metadata/PTC structural-view boundary. Writes derive semantic component paths from the declared type, so nested tuple field updates invalidate the correct tuple observations and preserve sibling values. Tuple replacement supports both existing and unknown fields. Ordinary invariant-slot traversal admits tuple fields while retaining its prior rejection of unguarded variant payloads; guarded traversal continues to require its conditions. This does not unfold a Tuple normal-form head or grant a tuple view to an arbitrary opaque head.

Internal tests exercise nested pointer reads/writes, scalar snapshots before and after mutation, invariant slot targets, out-of-bounds writes, preservation of the original normal-form type, and rejection of an unregistered opaque head. Source tests cover plain/nested tuples, direct tuple storage/record-contained tuples, two-function forwarding, replacement, old/new pointer targets and subsequent writes. Eight source scenarios include four successful executions and four runtime contract rejections, agreeing with static input proofs. Tuple Copy declarations are explicit; initial runtime validation correctly rejected the fixture without them (`/tmp/luna-tools-osrk1gk3/result.json`), and the final fixture supplies the required implementations.

Validation in `/home/fin/luna`: `python3 -B development/check.py --timeout 900 -- test -p elaboration --lib verification::flow` passed **4 tests**, report `/tmp/luna-tools-7upwfka3/result.json`; `python3 -B development/check.py --timeout 900 -- test -p luna --test resources` passed **354 tests**, report `/tmp/luna-tools-z028hmmh/result.json`. Formatting passed `/tmp/luna-tools-70mi6bhf/result.json`; final `git diff --check` passed. Memory-effects documentation includes the tuple-slot example and Copy requirement. Changes remain uncommitted on dev. External observation-path generalization beyond the exercised source paths still needs its own coverage; this work does not claim complete tuple-pattern-view elaboration.


### Completed aggregate program facts

`flow::binding_expression` now handles completed TupleValue, RecordValue and NominalValue terms instead of dropping their aggregate shape and known scalar fields. Field extraction follows declared tuple views, ordered record labels or package nominal layouts, with arity/domain/depth/budget checks. Nested scalar literals reuse the existing literal/captured-value fact path. Tuple normal-form identity is retained; opaque or unresolved layouts remain unknown. Type/type-list fields supply no scalar program facts and retain staging/type-layer formation responsibilities. This does not authorize runtime use of a comptime aggregate without Splice.

Eight source scenarios exercise tuple, nested tuple, struct and enum constants in explicit comptime functions. Four expected predicates are proven and invoke successfully; their four negations lack proof and fail with ProgramContractRejected. Extracted scalar snapshots survive local aggregate mutation. The first fixture attempted direct runtime use and was correctly rejected for missing Splice (`/tmp/luna-tools-q2vzf91n/result.json`); it was corrected to an explicit comptime entry, without relaxing phase checks. Internal cases also cover malformed arity, scalar kind/width, record labels/order, mixed Type fields, depth limits, and preservation of a registered Tuple head.

Validation in `/home/fin/luna`: `python3 -B development/check.py --timeout 900 -- test -p luna --test resources` passed **355 tests**, report `/tmp/luna-tools-o4ht7x3q/result.json`. Final internal run after adding record-validation cases: `python3 -B development/check.py --timeout 900 -- test -p elaboration --lib verification::flow`, **4 tests passed**, report `/tmp/luna-tools-vdxd91pv/result.json`. Final format report `/tmp/luna-tools-_bwoajn2/result.json`; `git diff --check` passed. Documentation includes the explicit comptime example and preserves the Splice boundary. Work remains uncommitted on dev.


### Staged aggregate contracts through runtime materialization

Symbolic comptime aggregate parameters now receive stable declaration-scoped program roots, extending the existing scalar capture identity to tuples and records without unfolding normal-form heads. Field observations transfer across call frames. Runtime predicate admission now accepts data-only record/variant reconstruction as well as tuples, while recursively rejecting calls and memory writes. This is required because input staging restores completed aggregate bindings independently inside each contract scope; folding a field rooted at ComptimeValue alone does not address these local reconstruction bindings.

The dedicated `staged_aggregate_contracts` test target covers nominal, nested and structural records, direct/forwarded contracts, equality boundaries, and explicit Splice conversion postconditions. Twelve valid bounded calls and one valid Splice conversion execute; six unjustified strict-comparison implementations and one wrong-field Splice implementation are rejected. The small target avoids rebuilding the large resource test binary for each edit.

Validation in `/home/fin/luna`: `python3 -B development/check.py --timeout 900 -- test -p luna --test staged_aggregate_contracts` passed both tests, report `/tmp/luna-tools-ijzpez23/result.json`; `test -p nir --test predicate_reads` through the same wrapper passed 9 tests, including data reconstruction and effectful-field rejection, report `/tmp/luna-tools-sjcggg96/result.json`; `test -p elaboration --lib verification::flow` passed, report `/tmp/luna-tools-lbkyqxvy/result.json`. Full workspace formatting passed `/tmp/luna-tools-hyhk98_y/result.json`; `git diff --check` passed. No full suite rerun; changes remain uncommitted on dev.


### Staged field stability across opaque callbacks

Runtime declaration checking now preserves staged parameter roots when a full function traversal finds neither direct writes nor address exposure. Symbolic declaration-scoped comptime captures are also marked frozen. The former concerns locally reconstructed inputs; the latter concerns immutable declaration values. Neither freezes reachable runtime memory. Existing path invalidation already respects frozen roots, and explicit writes still invalidate their own paths. This closes the verified failure where an opaque callback erased `config.other <= config.limit` despite receiving no access to the configuration (initial failing report `/tmp/luna-tools-mxx4h6nl/result.json`). Freezing only declaration captures was insufficient because ordinary comptime parameters remain local binders during definition checking.

Source regressions execute callbacks with flat and nested staged configuration. A writing callback updates a runtime Byte from 7 to 23 while the independent configuration contract remains proven. Counterexamples that assert the old pointee value after the callback are rejected for an unproven postcondition. Existing cross-call and Splice tests remain included. Validation in `/home/fin/luna`: `python3 -B development/check.py --timeout 900 -- test -p luna --test staged_aggregate_contracts` passed 4 tests, report `/tmp/luna-tools-fhaa26qw/result.json`; `test -p elaboration --lib verification::flow` through the same wrapper passed 4 tests, report `/tmp/luna-tools-l1hdsl0i/result.json`. Workspace formatting passed `/tmp/luna-tools-6oegj66p/result.json`; no full suite rerun. Changes remain uncommitted on dev.


### Immutable staged captures through closure invocation checking

Closure entry analysis now carries the enclosing state's frozen-input evidence into implicit captures. Opaque-call conservatism no longer marks those captures mutable merely because they have a nominal layout. Explicit written paths remain recorded even with an inconsistent immutable marker; captured runtime copies remain subject to the existing invalidation rules. Frozen evidence propagates into nested closure frames, while pointee observations remain separate.

Source tests validate ordered defaults from staged fields, ordinary/nested closure bodies calling a contracted helper, and an opaque callback before that helper. Reversing the captured inequality is rejected, as is overwriting a runtime copy before using its old field relation. The initial closure test demanded an inferred result postcondition from an arbitrary closure call; that is a different, unsupported contract mechanism. The final tests instead check the actual inner call obligation and execute its result, without granting arbitrary closure-result inference. The inner-call failure was reproduced in `/tmp/luna-tools-0kb9ai6r/result.json` before the fix.

Validation in `/home/fin/luna`: `python3 -B development/check.py --timeout 900 -- test -p luna --test staged_aggregate_contracts` passed 7 tests, report `/tmp/luna-tools-e5oqto65/result.json`; `test -p luna --test closure_environment_parameters` through the same wrapper passed 20 tests, report `/tmp/luna-tools-t9ud665e/result.json`. Formatting passed `/tmp/luna-tools-8rlb5l7z/result.json`; `git diff --check` passed. No full suite rerun. Work remains uncommitted on dev.


### Single-variant payload observations and single-path match predicates

Unknown nominal values now retain the uniquely determined tag of a one-variant enum, while payload expansion remains lazy. Typed field reconstruction handles variant layouts through package metadata. Match evaluation preserves a Boolean condition when exactly one branch is reachable: it renames pattern locals onto surviving observations only using exact equality evidence, then checks those versions after the branch scope is removed. No condition is exported by this path when multiple branches are reachable.

A source regression proves a staged pair's ordering through one match and uses it in a contracted call through another match; reversing the payload arguments is rejected. This required both preserving the known sole tag and retaining the branch condition (initial failure `/tmp/luna-tools-mabp_1h6/result.json`). The broader three-variant regression remains explicitly ignored with reason `requires disjunctive variant-tag and payload facts across match joins`; its current definition-checking failure is captured in `/tmp/luna-tools-lfbp5m72/result.json`. Do not treat this as enum-contract completion. The next required work is a tag-qualified conditional fact representation and transfer through joins, not unconditional identification of different variant payloads. Existing `Condition` supports retained Boolean combinations, but unknown enum patterns currently have no symbolic tag condition and the match join drops differing branch predicates.

Validation in `/home/fin/luna`: `python3 -B development/check.py --timeout 900 -- test -p luna --test staged_aggregate_contracts` passed 8 tests with 1 explicitly ignored pending regression, report `/tmp/luna-tools-ln36mvnw/result.json`; the wider `test -p luna --test resources` through the same wrapper passed 355 tests, report `/tmp/luna-tools-_sy7wju1/result.json`. Formatting passed `/tmp/luna-tools-zpue5xmx/result.json`; `git diff --check` passed. Changes remain uncommitted on dev.


### Conditional program-fact propagation after branch selection

`Facts::assume` now transactionally propagates current retained Boolean premises after introducing a constraint. Refutation also performs the same propagation in its private opposite-condition frame. During replay, the already-known shortcut is bypassed for the saved premises so selecting a branch can yield new arithmetic bounds; propagation stops at a fixed point or the existing resource budget. Current versions are required, original premises are retained afterward, and proof replay still verifies lineage/premises before repeating entailment. No PTC integration or arithmetic degree extension was introduced.

The reproduced solver failure `/tmp/luna-tools-lx45uskh/result.json` involved chained implications. Prover-only propagation passed solver tests but failed a source helper-call obligation (`/tmp/luna-tools-3ylhj4ei/result.json`), since the caller had not exported its newly implied scalar bound. Moving the shared propagation into transactional assumption handling closes that path. Source regressions exercise enabled configurations with payloads 7/19, a disabled configuration with payload 255, and rejection of the reversed branch. Solver tests cover implication chains, stale payloads, stale guards before selection, replay, exhausted budgets, and exhaustive Boolean models. Changing a guard after a bound was already established does not erase a valid bound on an unchanged payload; replacing a guard before selection cannot activate old-version implications.

Validation in `/home/fin/luna`: `python3 -B development/check.py --timeout 900 -- test -p elaboration --test verification` passed 46 tests, report `/tmp/luna-tools-dow4kuv6/result.json`; `test -p luna --test staged_aggregate_contracts` passed 9 tests with the existing 1 pending enum regression ignored, report `/tmp/luna-tools-w4_3d43k/result.json`; `test -p luna --test resources` passed 355 tests, report `/tmp/luna-tools-py62r4ao/result.json`. All tests used the same check.py wrapper. Formatting passed `/tmp/luna-tools-czm6qj7p/result.json`; `git diff --check` passed. Multi-variant tag/payload observation and Boolean match-result composition remain required; this change supplies their conditional arithmetic premise machinery, not their missing flow representation. Work remains uncommitted on dev.


### Tag-qualified enum payload predicates

Program observations now include an explicit EnumTag component with stable S-expression display. Variant matching splits on a bounded tag observation when the constructor is unknown. Payload paths include their declaration and variant index; alternative payload observations remain facts rather than reconstructing an unknown enum as one chosen variant. Tags and typed scalar payload observations are prepared before match forks so their identities survive joins and can be transported through the existing copy-evidence machinery.

Boolean matches with distinct variant arms, no guards, and binding/wildcard payload patterns compose tag-qualified branch conditions. Pattern locals are rewritten through proved equalities onto surviving field observations. Existing single-reachable-arm handling remains available for other patterns. This implements the previously ignored three-variant regression: none, pair(7,19), and single(19) execute successfully; reversing the pair relation, tightening the single bound, or requiring an impossible bound in the none arm is rejected. Repeated matching also excludes contradictory tags, with a negative control that makes the bad call reachable. The ignored marker is removed; no test in the dedicated target remains skipped.

Initial tag/payload composition exceeded the ordinary budget because repeating a match copied an already frozen input into another equality-heavy analysis frame. Materialization now reuses only a current frozen root; mutable values retain their snapshot path. Retained-condition propagation has bounded sweeps (not an obligation to reach a nonlinear fixed point), and interval-decided conditions prune inactive branches before fork/join work. No default budget was increased. Initial profiling/failure reports: `/tmp/luna-tools-udxruxnp/result.json`, `/tmp/luna-tools-vx0u484u/result.json`; diagnostics were removed after the fix. Known constructor payloads preserve their actual ranges when observations are primed.

Validation in `/home/fin/luna`: `python3 -B development/check.py --timeout 900 -- test -p luna --test staged_aggregate_contracts` passed 11 tests with no ignored cases, report `/tmp/luna-tools-guenpsvl/result.json`; `test -p elaboration --test verification` through the same wrapper passed 46 tests, report `/tmp/luna-tools-pn_ncs36/result.json`; `test -p luna --test resources` passed 355 tests, report `/tmp/luna-tools-q7_8fg3f/result.json`. Formatting passed `/tmp/luna-tools-2odlqhl9/result.json`; `git diff --check` passed. Guarded/overlapping patterns, wildcard fallbacks, nested payload composition and general mutable-runtime enum contracts still require additional coverage/support; this is not a claim of complete pattern-specification inference. Changes remain uncommitted on dev.


### Ordered wildcard fallback in enum predicate matches

Boolean enum match composition now accepts a final unguarded wildcard after simple variant arms. Its selection condition negates the union of preceding tags, preserving ordered failure semantics rather than treating fallback as unconditional. Guards, overlapping/refutable payload patterns, and subsequent arms after fallback still disable this composition path. Covered-tag expressions are constructed only while composition remains eligible, avoiding new condition-depth costs in unrelated non-Boolean matches.

The initial source regression failed before implementation (`/tmp/luna-tools-bc70dgt1/result.json`). Final cases cover one/two preceding variants with none, pair(7,19), and single(255): six successful executions, six reversed-pair rejections, and six attempts to use an unjustified single payload bound through a nested fallback match. Validation in `/home/fin/luna`: `python3 -B development/check.py --timeout 900 -- test -p luna --test staged_aggregate_contracts --test match_cleanup` passed 12 staged-contract tests and 13 cleanup tests, report `/tmp/luna-tools-68co65rs/result.json`. Formatting passed `/tmp/luna-tools-aq9pu031/result.json`; `git diff --check` passed. No full resource rerun for this bounded match-composition extension. Work remains uncommitted on dev.


### Nested owned enum payload observations

Variant preparation now expands known owned payload layout and publishes its scalar component observations before branching, reusing the existing typed shape expansion and result-component tracking. This allows record, nested-record and tuple payload relations to survive pattern binding and later helper calls. Unknown alternative variants remain distinct; no arbitrary enum constructor is reconstructed from their combined observations.

The first record-payload source regression failed before implementation (`/tmp/luna-tools-ak8akpl4/result.json`). Expanding coverage to a tuple payload with an inner match exposed redundant snapshot equality costs (`/tmp/luna-tools-5cah5un1/result.json`). Walker now retains the full-function written-local set alongside address exposure. Materialization can reuse a current projection of a local that is neither written nor address-exposed; other mutable/addressable values keep the previous snapshot behavior. This is an analysis identity optimization, not permission to read pointees or reuse a stale version. No verification budget was increased.

Source cases cover records, deeper records and tuples, each with populated/none constructors and reversed-order rejection. A separate mutable-scrutinee test proves that bindings from the selected old tuple survive replacing the original, while claiming the old ordering for the new tuple is rejected. Validation in `/home/fin/luna`: `python3 -B development/check.py --timeout 900 -- test -p luna --test staged_aggregate_contracts --test match_cleanup` passed 14 contract tests and 13 cleanup tests, report `/tmp/luna-tools-l_3ap728/result.json`; `test -p elaboration --lib verification::flow` through the same wrapper passed 4 tests, report `/tmp/luna-tools-44dtti4s/result.json`; `test -p luna --test resources` passed 355 tests, report `/tmp/luna-tools-hwjqq4n1/result.json`. Formatting passed `/tmp/luna-tools-udpi1vjb/result.json`; `git diff --check` passed. Nested enum tag composition and guarded/refutable payload patterns remain separate work. Changes remain uncommitted on dev.


### Nested enum observations and frame-local frozen projections

Payload preparation now recursively publishes finite nested enum tags and their variant-qualified fields, including owned record/tuple paths. Repeated enum declarations stop unknown recursive expansion after publishing the current tag; depth/work remain bounded. This closes the nested Config.some(Inner.pair(...)) contract regression, with outer none, inner none and inner pair executions and reversed-order rejection. The first missing-proof report is `/tmp/luna-tools-8b30vpbp/result.json`.

The first implementation exhausted the unchanged budget because nested aggregate pattern binding generated redundant copied identities. Immutable aggregate bindings from a frozen, current source may now retain a frame-qualified projection when the destination has no writes or address exposure. This metadata is copied only when forking/merging analysis state; ordinary value snapshots drop it. Lookup checks frame, frozen source and source version. Invalid/stale/cross-frame metadata falls back to the ordinary binding. Its Display is explicit S-expression. Internal tuple-storage tests exercise same-frame branch preservation, ordinary-snapshot removal, frame mismatch, and source-version replacement. Mutable copies remain on the existing transfer path. Temporary fuel diagnostics from `/tmp/luna-tools-oz3rhike/result.json` were removed.

Condition assumption also eliminates enclosure-proven neutral Boolean operands before a fork/join; these are already-evaluated mathematical conditions, so this does not reorder source effects. No default budget increase was made.

Validation in `/home/fin/luna`: `python3 -B development/check.py --timeout 900 -- test -p luna --test staged_aggregate_contracts --test match_cleanup --test closure_environment_parameters` passed 15 + 13 + 20 tests, report `/tmp/luna-tools-p_djha0s/result.json`; `test -p elaboration --lib verification::flow` passed 4 tests, report `/tmp/luna-tools-6jhz446l/result.json`; `test -p elaboration --test verification` passed 46 tests, report `/tmp/luna-tools-_x__as42/result.json`; `test -p luna --test resources` passed 355 tests, report `/tmp/luna-tools-_f1ki1qz/result.json`. All tests used the same wrapper. Formatting passed `/tmp/luna-tools-txtdf9yt/result.json`; `git diff --check` passed. Guarded/overlapping patterns and arbitrary recursive-layout contract inference remain incomplete. Changes remain uncommitted on dev.


### Ordered guarded enum predicate composition

Match analysis now separates a top-level pattern guard from its inner pattern and evaluates it once in the same ordered candidate/failure flow as an arm guard. Boolean condition composition accepts guards admitted by `nir::program::checkable_predicate`; effectful/unsupported guards disable this inference. The accumulated remaining condition excludes each prior successful pattern+guard selection, allowing repeated variants with different guards and guarded wildcard arms without changing first-match semantics. Neutral Boolean constructors are simplified while composing conditions.

The initial guard-contract regression failed in `/tmp/luna-tools-zwny01i2/result.json`. It now proves a helper precondition from a guarded pair premise and rejects reversing its field order. Additional cases exercise an earlier `a == 0 => false` arm followed by `a <= b => true`: nonzero ordered pairs execute, while zero pairs, reversed pairs and a different tag are rejected at the call precondition. This explicitly prevents a later true result from overriding the first selected false result.

Validation in `/home/fin/luna`: `python3 -B development/check.py --timeout 900 -- test -p luna --test staged_aggregate_contracts --test match_cleanup` passed 17 contract tests and 13 cleanup tests, report `/tmp/luna-tools-q0i7wj68/result.json`; `test -p elaboration --lib verification::flow` through the same wrapper passed 4 tests, report `/tmp/luna-tools-9kcw83my/result.json`. Formatting passed `/tmp/luna-tools-r9ed45bb/result.json`; `git diff --check` passed. No full resource-suite rerun for this bounded match-composition change. Refutable payload patterns and more general recursive predicate inference remain separate work. Changes remain uncommitted on dev.

### Literal payload selection in enum contracts

Boolean enum-match composition now includes integer and Boolean payload literals in the ordered arm selection condition. Bindings recursively preserve their inner literal condition; unsupported payload patterns disable composition. Unknown Boolean field observations compare against their scalar 0/1 representation, while known Boolean constants retain the direct comparison path. Work and recursion remain bounded.

The original integer payload regression failed with a missing helper precondition in `/tmp/luna-tools-7_grq6o2/result.json`. Source tests now cover an earlier `.pair(0, _) => false` followed by an ordered-pair branch, plus `.flag(true, value)` carrying a range constraint. Wrong Boolean values, wrong tags, out-of-range payloads and unjustified helper arguments are rejected; accepted programs execute to true.

Validation in `/home/fin/luna`: `python3 -B development/check.py --timeout 900 -- test -p luna --test staged_aggregate_contracts --test match_cleanup` passed 19 contract tests and 13 cleanup tests, report `/tmp/luna-tools-21eb77s1/result.json`; `test -p elaboration --lib verification::flow` through the same wrapper passed 4 tests, report `/tmp/luna-tools-zcbecmpu/result.json`. Workspace formatting passed `/tmp/luna-tools-xd933vax/result.json`; `git diff --check` passed. No full resource rerun for this bounded extension. More complex payload patterns remain conservative. Changes remain uncommitted on dev.

### Recursive payload-pattern selection

Payload condition synthesis now descends through tuple, nominal/anonymous record and enum patterns using declared field types and existing qualified projections. Nested enum selection includes each tag; a known inactive tag returns false without inspecting a payload from the wrong layout. Binding wrappers preserve their inner condition. Unsupported patterns still disable composition, and depth/work remain bounded without raising verification budgets.

A source matrix exercises tuple, record and nested enum payloads with an earlier zero-literal rejection and a later ordered-field premise, followed by separate body destructuring and a contracted helper call. Each layout executes successfully and rejects zero, reversed ordering, incorrect outer tags and reversed helper arguments; the nested enum additionally rejects the incorrect inner tag.

Validation in `/home/fin/luna`: `python3 -B development/check.py --timeout 900 -- test -p luna --test staged_aggregate_contracts --test match_cleanup --test closure_environment_parameters` passed 20 + 13 + 20 tests, report `/tmp/luna-tools-aab7vgho/result.json`; `test -p elaboration --lib verification::flow` through the same wrapper passed 4 tests, report `/tmp/luna-tools-976gy2h2/result.json`. Workspace formatting passed `/tmp/luna-tools-4wt0bqeu/result.json`; `git diff --check` passed. Full resource suite was not rerun for this bounded extension. Changes remain uncommitted on dev.

### Structural and scalar match predicate composition

Boolean match analysis now uses the same recursive selection synthesis for top-level tuples, records, literals and bindings, instead of requiring an outer enum pattern. Structural observations are prepared once before branching; pure pattern conditions and guard conditions still participate in ordered remaining-case composition. Unsupported patterns retain conservative behavior. The helper is renamed `pattern_selection_condition` to reflect its broader role.

New source tests exercise direct tuple and record inputs with an earlier zero-field rejection, a later ordering premise and separate body projections. A runtime Byte parameter test proves a bound from `value match {0=>false,x=>x <= 19}`. Zero values, reversed/out-of-range fields and unjustified helper calls are rejected; accepted inputs execute successfully.

Validation in `/home/fin/luna`: `python3 -B development/check.py --timeout 900 -- test -p luna --test staged_aggregate_contracts --test match_cleanup --test resources` passed 22 contract tests, 13 cleanup tests and 355 resource tests, report `/tmp/luna-tools-1eik9ied/result.json`. Workspace formatting passed `/tmp/luna-tools-teq7sikt/result.json`; `git diff --check` passed. Changes remain uncommitted on dev.

### Alternative-pattern predicate composition

Pure structural Or patterns now synthesize the union of their candidate selection conditions. Remap wrappers preserve the inner selection condition because renaming binders does not change acceptance. Ordered arm composition excludes the entire union from later arms. Unsupported candidate computations or internal guards disable this composition; ordinary pattern execution, candidate ordering and binding transfer are unchanged.

Source tests cover top-level literal alternatives, an outer `as` binding, nested enum payload alternatives, and shared tuple field bindings through `(0,x) or (1,x)`. Both alternatives execute with valid bounds; excluded values, wrong tags and out-of-range payloads are rejected. The common-field test confirms that remapped binders retain the original second-field identity after the candidate merge.

Validation in `/home/fin/luna`: `python3 -B development/check.py --timeout 900 -- test -p luna --test staged_aggregate_contracts --test match_cleanup` passed 25 + 13 tests, report `/tmp/luna-tools-w_jp8irj/result.json`; `test -p elaboration --lib verification::flow` through the same wrapper passed 4 tests, report `/tmp/luna-tools-f4hpr279/result.json`. Formatting passed `/tmp/luna-tools-wiv_u7nz/result.json`; `git diff --check` passed. No full resource rerun for this bounded extension. Changes remain uncommitted on dev.

### Candidate-specific bodies and bounded copied premises

Top-level Or arms without an outer guard now defer the analysis-state join until after the shared body. Each candidate keeps its own remapped binder identities and ordered failure continuation. Runtime IR/execution is unchanged. Arms with an outer guard are not split: failure of that guard must skip the whole arm rather than retry a later alternative.

The new runtime tuple contract `(true,x,_) or (false,_,x) => x <= 19` initially failed to prove the selected field bound (`/tmp/luna-tools-ogy9fbpl/result.json`). Preserving candidate identity exposed redundant propagation that exhausted the unchanged budget (`/tmp/luna-tools-54a4x_up/result.json`). Copied conditions are now remembered from their existing copy evidence after bounds/relations are transported, rather than replaying all logical choices after every field binding. Retained-condition storage normalizes negations and decomposes true conjunctions / false disjunctions, retaining their operands rather than redundant compound premises. Depth and work are charged; no budget was increased.

The original positive case now passes. Source tests execute both selected fields, reject invalid selected bounds and attempts to use the unselected field, and verify that an outer guard does not retry another binding. Validation in `/home/fin/luna`, all through `python3 -B development/check.py --timeout 900 --`: `test -p luna --test staged_aggregate_contracts --test match_cleanup --test closure_environment_parameters` passed 27 + 13 + 20 tests (`/tmp/luna-tools-je70f1pu/result.json`); `test -p elaboration --test verification` passed 46 (`/tmp/luna-tools-685ho_ju/result.json`); `test -p elaboration --lib verification::flow` passed 4 (`/tmp/luna-tools-lcd5l1k5/result.json`); `test -p luna --test resources` passed 355 (`/tmp/luna-tools-y_2z92w_/result.json`). Formatting passed `/tmp/luna-tools-u7b3foaz/result.json`; `git diff --check` passed. Changes remain uncommitted on dev.

### Guarded alternative arm continuations

Top-level Or alternatives now retain their original arm boundary during analysis even when an outer guard is present. Pattern failure continues to the next candidate; guard failure is saved separately and joins the next original arm only after all candidates have been analyzed. Candidate-specific conditions use a separate remaining-pattern mask, while subsequent arms exclude the union of actual pattern-plus-guard successes. This preserves both selected field identities and the rule that guard failure never retries a different binding. Runtime IR remains unchanged.

The initial guarded cross-field contract failed to prove the selected bound (`/tmp/luna-tools-qdh57prb/result.json`). It now executes both flag selections and rejects invalid bounds and using the unselected field. A separate interpreter-event test verifies one gate invocation for both a first matching candidate whose guard fails and a later matching candidate; fallback returns false without retrying another binding.

Validation in `/home/fin/luna`: `python3 -B development/check.py --timeout 900 -- test -p luna --test staged_aggregate_contracts --test match_cleanup --test resources` passed 29 + 13 + 355 tests, report `/tmp/luna-tools-8c58yt0g/result.json`; `test -p elaboration --lib verification::flow` through the same wrapper passed 4, report `/tmp/luna-tools-g3rv23_s/result.json`. Formatting passed `/tmp/luna-tools-titllffs/result.json`; `git diff --check` passed. Nested computational alternatives remain conservative. Changes remain uncommitted on dev.

### Guard mutation and subject snapshot regressions

Revisited the prior rejected do-guard fixture: its `count+1` spelling violated the existing separated binary-operator grammar; legal `count + 1` parses and executes. No parser change was needed.

Added end-to-end checks for mutations in outer guards of Or arms. Both first-candidate and later-candidate selection increment a local exactly once and transfer that state to fallback; claiming two increments is rejected by the helper precondition. A mutable-subject case updates the source tuple inside the failed guard and confirms that subsequent arms still match the original subject snapshot, while external reads observe the updated tuple. The runtime result and count contract check these independently. Documentation now includes this concrete example.

Validation in `/home/fin/luna`: `python3 -B development/check.py --timeout 900 -- test -p luna --test staged_aggregate_contracts --test match_cleanup` passed 31 + 13 tests, report `/tmp/luna-tools-_cwmcbx9/result.json`. Formatting passed `/tmp/luna-tools-5ai05tsz/result.json`; `git diff --check` passed. No compiler implementation changed, so the full resource suite was not repeated. Changes remain uncommitted on dev.

### Direct handler writes with stationary pointer origins

Direct handler arms can now replace a captured pointer-bearing value when their
bounded input-pointer exit summary proves that every pointer leaf still has its
exact original formal origin. The memory and resource analyses share one proof
helper. No equality between two fields, joined origin set, local target, unknown
exit, traversed pointer or variant-dependent leaf substitutes for that evidence.

Authorization contains only ephemeral expression identities from the checked
arm. The installer's computation does not inherit permission to rebind a slot.
The arm must have no guard or pattern observation and no effect request,
computation-valued expression or nested handler; a completion invariant cannot
justify temporary rebinding visible during suspension or before a guard fails.
Code identity, address exposure, pointer liveness, ownership and cleanup remain
independent checks. Deferred resource overlap checks retain and charge their
complete pointer layouts before the arm summary is available.

Verified source cases rebuild a whole record directly and through an ordinary
function, retain state across repeated requests, and permit nested self-assignment
of a pointer field. Rejection cases include field exchange, conditional origin
joins, installer writes, local-pointer escape, failed-guard mutations and an
intermediate mutation around an outer effect request. Rejection tests check a
writeback/slot-rebinding diagnostic, not merely any failure.

In `/home/fin/luna`, the following command exited 0 with 225 tests passing
(94 captured-memory, 98 effect-memory-transfer, 33 iterator tests):

```sh
python3 -B development/check.py --timeout 900 -- test --release -p luna --test library_iteration --test effect_memory_transfers --test captured_memory --no-fail-fast
```

Evidence: `/tmp/luna-tools-i6bg8nj5/result.json`. The first run reached the
independent resource overlap gate; the next run exposed a stale self-assignment
rejection and fold's next, still-valid ownership restriction. These were not
counted as passing runs. The final tests explicitly execute self-assignment and
reject a fresh local pointer instead.

Unexpanded affine fold still requires its borrowed capture to remain complete on
every exit. Its pointer provenance gate is now discharged, but moving the
accumulator through the callback does not by itself prove this stronger ownership
contract. The regression retains rejection with the current ownership diagnostic;
expanded execution and existing native checks remain successful. General handler
writeback across suspension and pointer-bearing lexical search results remain
separate work. Existing uncommitted formatting in moves/effects.rs is preserved
outside this change's commit.
