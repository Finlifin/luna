# Definition builder implementation boundary

The intended feature covers most definition kinds, including the SoA example's
struct construction. This file tracks implementation, not new Flurry syntax.

Measured concrete publication costs and a reproducible executable are documented
in [builder-publication-performance.md](builder-publication-performance.md).
The comparison isolates retained checking state versus a freshly rebuilt world;
it is not a whole-compiler or historical-version speedup claim.

## Type receiver reflection methods

`meta.reflection.structure` now owns the seven public struct/enum observations
and exposes them as inherent comptime methods on builtin Type. The existing
root free-function exports remain available. StructBuilder and EnumBuilder
`from_type` use the methods; builder modules no longer own reflection exports.
The isolated meta fixture includes the same reflection source.

Receiver resolution first considers methods of a statically named subject, then
permits methods on the Type value itself. This fixes named enum receivers such
as `Event.enum_group_paths()` being rejected as unknown variants. Existing
variant construction and enum-group namespace behavior remain covered. A
subject's own same-named method retains priority; storing its Type in a local
permits ordinary Type receiver lookup. No special reflection spelling is built
into method dispatch.

Validation from `/home/fin/luna`:

```sh
python3 -B development/check.py --timeout 900 -- test --release -p luna --test builder_invocations --test enum_groups --test newtypes
python3 -B development/check.py --timeout 900 -- test --release -p luna --test source_reflection --test library_protocols --test normal_form_methods --test comptime_list_operations
python3 -B development/reproduce.py --timeout 120 --binary /tmp/luna-dev-002/target/release/luna -- development/examples/sysroot-edited-struct.fl --sysroot library --test-native
python3 -B development/reproduce.py --timeout 120 --binary /tmp/luna-dev-002/target/release/luna -- development/examples/sysroot-edited-enum.fl --sysroot library --test-native
python3 -B development/command.py --timeout 120 -- cargo fmt --all -- --check
git diff --check
```

All commands exited 0. The first suite passed 72 builder, seven enum-group and
211 newtype tests (`/tmp/luna-tools-6m_0mu6z/result.json`). The second passed
12 reflection, 27 library protocol, four normal-form method and 54 list tests
(`/tmp/luna-tools-ma7ad2cp/result.json`). Native sysroot tests passed four struct
and three enum cases (`/tmp/luna-tools-6t5bw_k9/result.json` and
`/tmp/luna-tools-cz_nnmsx/result.json`), covering all seven reflection methods,
generated and generic types, same-named static method priority and runtime
construction/access. Rejection tests cover wrong structural kinds and runtime
observation. Formatting passed at `/tmp/luna-tools-ekeevw8n/result.json`.

The original enum method reproduction failed with `unknown enum variant
 enum_group_paths` in `/tmp/luna-tools-no4odx5t/result.json`. The first module
name also collided with the existing meta/types directory; the final module
is reflection/structure. These methods retain existing reflection permissions,
nominal identities and phase checks. They do not transport field-default code
or copy generated methods, attributes or implementations.

### Computed Type receivers

Receiver dispatch can select a method on the known Type domain of a computed
receiver before asking for the concrete represented type. It retains the
receiver expression and the selected method for normal call preparation. This
avoids requiring ordinary invocation-time Type inputs during definition checking
and avoids executing receiver effects merely to choose a reflection method.
Explicit trait views retain their existing routing; named static subjects and
computed receivers without an applicable Type method retain subject lookup.

The regression exercises if, match, block and method-call receivers with a
comptime value-reference counter: two calls increment it exactly twice. A
parameter-controlled branch returns before reflection. The initial constant
return-only fixture instead folded to Never and had no method-bearing domain;
it was corrected to retain a Type-producing alternative. This change does not
infer methods for an arbitrary expression whose only known domain is Never.

Validation from `/home/fin/luna`:

```sh
python3 -B development/check.py --timeout 900 -- test --release -p luna --test source_reflection --test builder_invocations --test enum_groups --test newtypes
python3 -B development/check.py --timeout 900 -- test --release -p luna --test source_reflection
python3 -B development/reproduce.py --timeout 120 --binary /tmp/luna-dev-002/target/release/luna -- development/examples/sysroot-edited-struct.fl --sysroot library --test-native
python3 -B development/command.py --timeout 120 -- cargo fmt --all -- --check
git diff --check
```

The first command passed 72 builder, seven enum-group and 211 newtype tests but
failed the new reflection fixture (`/tmp/luna-tools-ukqji9xj/result.json`, exit
101). After correcting its return-only receiver, all 13 reflection tests passed
(`/tmp/luna-tools-y5gke1tw/result.json`, exit 0); production code was unchanged
between these runs. The intermediate constant-condition variant still folded
to Never and failed in `/tmp/luna-tools-x4zcim3n/result.json`.
All four real sysroot struct tests passed (`/tmp/luna-tools-srkvm184/result.json`)
with conditional/block receivers feeding the column builder. Formatting passed
(`/tmp/luna-tools-w9hgnar6/result.json`) and diff checks passed.

The original small source reproduction failed before the dispatch fix with an
unavailable comptime input (`/tmp/luna-tools-yzcz3_wf/result.json`). Afterward its
full sysroot invocation needed explicit target facts for an unrelated numeric
runtime proxy; with `--build-target x86_64 linux 64 --invoke-comptime run` it
returned `(bool true)` (`/tmp/luna-tools-ntvyy3j_/result.json`). The durable source
regression and sysroot builder example carry the same dynamic receiver case.

## CMTT and builder integration boundary

### Ordered Object observation from source

The public meta API now exposes `object_length`, `object_key`, `ObjectEntryType`
and `object_value<T>`. Reads preserve ordered entries, duplicate keys and nested
optional layers. Negative/out-of-range indices fail invocation; an absent key
means a positional entry, while an absent typed read means the stored domain tag
does not match T. Tag matching does not introduce type-predicate proofs or perform
implicit conversion. The new ObjectQuery NIR node has an S-expression dump and
copy/visit/type-rewrite/phase/materialization coverage. Runtime queries reject.

CMTT data reconstruction now uses the existing typed Object constant path rather
than attempting unsupported normal-term encoding. Nested entries retain their
types; active pointer payloads still reject. Functional editing is described
below, together with stable input encoding. Local Object queries now avoid
discarded snapshots; eligibility, retained validation costs and measurements are
recorded in [comptime-object-queries.md](comptime-object-queries.md).

Validation in `/home/fin/luna`:

- `python3 -B development/check.py --timeout 900 -- test --release -p luna -p nir
  -p comptime_interpretor --lib --test library_meta_objects --test builder_invocations
  --test copy_budget --test type_rewrite --test phases`: exit 0, 356 tests
  (`/tmp/luna-tools-2n037409/result.json`).
- `python3 -B development/check.py --timeout 900 -- test --release -p luna
  --test cmtt --test library_meta_objects`: exit 0, 527 CMTT and 8 Object tests
  (`/tmp/luna-tools-c70s3ycr/result.json`).
- `python3 -B development/host_examples.py object-queries --release-driver
  --compare-optimization`: O0/O3 pass
  (`/tmp/luna-host-examples-ub10tjwn/summary.json`).

Targeted formatting and `git diff --check` pass. Initial source failures exposed
missing target context for the private typed query and an ambiguous test Object
literal; the private entry now takes an explicit Type operand. The first native
attempt exposed the generator Object reconstruction gap, fixed above.

### Functional Object editing from source

`meta.object_concat`, `object_append`, `object_replace` and `object_remove`
return new ordered Objects. Concatenation preserves duplicate keys; append
accepts a dynamic optional Symbol key; replacement keeps the key and position
but stores the new value's actual domain. Removal shifts the remaining entries.
The VM checks bounds and replacement domains, charges container movement, key
allocation and type description copying, and leaves input snapshots unchanged.
ObjectEdit has S-expression display and the same traversal, substitution,
materialization and phase coverage as ObjectQuery. These APIs do not imply
constant-time editing or arbitrary definition publication.

The native example exposed a contextual literal bug: `.enabled` passed to
`?Symbol` was treated as an enum path. Symbol literal checking now recognizes
optional Symbol destinations and preserves nested optional layers; nominal enum
path resolution remains separate. Tests cover dynamic and literal keys, changed
payload domains, nested optionals, bounds, evaluation order, early return,
malformed VM inputs, failure recovery and runtime rejection.

Validation in `/home/fin/luna`:

- `python3 -B development/check.py --timeout 900 -- test --release -p luna -p nir
  -p comptime_interpretor --lib --test library_meta_objects --test copy_budget
  --test phases --test type_rewrite`: exit 0, 310 tests
  (`/tmp/luna-tools-kkp_af2p/result.json`).
- After the optional Symbol fix, `python3 -B development/check.py --timeout 900
  -- test --release -p luna --test library_meta_objects --test native_optional
  --test enum_groups`: exit 0, 30 tests
  (`/tmp/luna-tools-g4gvsipt/result.json`).
- `python3 -B development/host_examples.py object-edits --release-driver
  --compare-optimization`: O0/O3 pass
  (`/tmp/luna-host-examples-kbtmly9u/summary.json`).

Targeted rustfmt and `git diff --check` pass. The first source run failed because
the fixture lacked a char binding; the first native run exposed the optional
Symbol issue above. Both are corrected in the passing evidence.

### Stable Object inputs

Immutable Objects now freeze into tagged aggregate terms. Each ordered entry
packet retains its optional key, original domain and value; this is an internal
encoding, not a language Tuple or a record with merged keys. Scope checking,
substitution, copy accounting and iterative disposal reuse stable term traversal.
Display renders `(object-value (entry (key ...) (type ...) (value ...)) ...)`.
Reconstruction validates payloads against their domains and remains comptime-only.
Keys, type descriptions and entry width are charged before allocation; active
pointers and excessive nesting still reject.

Normal-form admission, ordinary function clause admission and nominal value
admission recognize the encoding. The ordinary function path retains trusted
List metadata for nested values. Known Object literals can provide structural
input evidence without executing calls or normal-form bodies. Typed normal-form
results retain their heads. Distinct representations are tested as representations;
this change does not infer semantic inequality or injectivity of opaque heads.

Validation in `/home/fin/luna`:

- `python3 -B development/check.py --timeout 900 -- test --release -p luna -p nir
  -p comptime_interpretor --lib --test library_meta_objects --test normal_inputs
  --test nominal_parameters --test objects --test type_rewrite --test phases`:
  exit 0, 346 tests (`/tmp/luna-tools-cos00wjq/result.json`).
- `python3 -B development/host_examples.py object-inputs --release-driver
  --compare-optimization`: O0/O3 pass
  (`/tmp/luna-host-examples-mvm5gkee/summary.json`).

Additional staging regression: `python3 -B development/check.py --timeout 900
-- test --release -p luna --test cmtt --test library_protocols`: exit 0,
527 CMTT and 25 library protocol tests
(`/tmp/luna-tools-eceatj1d/result.json`).

The original Boolean local-clause failure in
[comptime-local-clause-input.fl](repros/comptime-local-clause-input.fl) is now
covered by the invocation-time implementation below. Its earlier unavailable
input diagnostic (`/tmp/luna-tools-qwozxuxv/result.json`) predates that fix.

Initial source fixtures needed clause syntax corrections. Later failures exposed
an obsolete “Object inputs unsupported” assertion and the separate ordinary
function admission path, both corrected. An attempted negative Type comparison
was replaced with representation assertions: unresolved equality must not be
reported as false. Targeted formatting and `git diff --check` pass.

### Invocation-time function clause inputs

Ordinary comptime functions with a known callable signature can receive local
values in explicit positional clauses. NIR preserves the expressions in
`(comptime-function-instance (declaration ...) (inputs ...))`. The VM evaluates
them once in source order before ordinary arguments, freezes their values, and
delegates to the existing instance cache. Returning from an input expression
skips later inputs and arguments. Each callable retains its selected snapshot;
changing the local variable afterward does not modify that callable.

Input domain validation and type requirements run on cache hits too. Failed
admission does not publish a partial instance. Object/List inputs retain their
original domains, and static Type clauses can specialize later value domains.
This is invocation work: definition checking does not execute the generator or
unfold normal-form heads. Runtime use remains rejected. NIR phase validation,
dependency collection, substitution, copy accounting and Display traverse the
new operands.

The frontend route also accepts named optional clauses and tuple spreads using
the existing nominal input preparation logic. Explicit inputs are bound once
in source order, then omitted defaults are prepared in declaration order and
declaration scope. Defaults reuse the saved preceding inputs; explicit overrides
skip their default computation. The instance packet reads these bindings in
parameter order. Returning while preparing any input skips later inputs,
defaults and the call. Completed Type clauses trigger a final domain check even
when a named Type input follows its value input or the caller is never invoked.

Inherited lexical clause inputs are fixed by the enclosing instance and placed
before the local function's own inputs in the invocation packet. Explicit
argument routing uses only the local declaration's parameter identities and
domains, so a caller cannot supply or override inherited slots. Defaults can
reference inherited Types and values. Recursive local calls retain the current
outer instance, while different outer instances produce independent snapshots.
This uses the existing lexical staged-input representation; it does not add an
invocation environment for capturing ordinary comptime function parameters.

Ordinary parameter defaults are prepared using the selected callable instance.
Definition-side preparation substitutes known Type clauses; remaining clause
values stay symbolic. At invocation the VM substitutes the frozen instance
arguments into the default, charging copy/substitution work before evaluating it
in the existing prepared-parameter scope. Explicit inputs still execute first;
changing a source local during an ordinary argument does not change its already
frozen clause snapshot. Explicit overrides skip default evaluation. The existing
default stability, phase, ordering and cleanup checks remain in place.

Ordinary Type and trait requirements use the same definition-side clause evidence
as defaults. Known Type inputs and inherited inputs are retained; not-yet-evaluated
value inputs remain declaration rigids. The actual invocation packet still owns
their values. This prevents an explicit comptime instance from being checked as
though it had an empty clause telescope when a requirement also references an
ordinary comptime Type parameter. It does not assert unknown value equalities.
The instance formation path also registers declaration requirements through the
shared formation checker. Requirements independent of deferred value clauses are
discharged during definition checking, including defaulted Type clauses and
unused callers. Value-dependent requirements remain attached to the declaration
and are checked by invocation; a symbolic value is not assumed to satisfy them.

Explicit inherent method instances use this same path with their selected owner
inputs fixed as a prefix. Static methods can retain snapshots as function values;
receiver methods first bind the receiver outside the clause preparation block.
Spreads retain this outer order and keep the callable instance available to
ordinary defaults after staging it in a temporary. Trait methods still use their
interface-origin dispatch plan; this integration does not erase that origin or
claim dynamic trait-method clause support is complete.

Callable signatures and clause domains cannot depend on value-clause
binders. Open functions, quoted bodies and more general dependent signatures
retain their existing elaboration paths; this stage does not claim their dynamic
local-value support is complete.

Validation in `/home/fin/luna`: `python3 -B development/check.py --timeout 900 --
test --release -p luna -p nir -p comptime_interpretor --lib --test
comptime_clause_inputs --test library_meta_objects --test phases --test
type_rewrite --test copy_budget`: exit 0, 330 tests
(`/tmp/luna-tools-o5rexmuu/result.json`). Tests include source-order side effects,
early return, immutable callable snapshots, cache reuse and contract rechecks,
failed-input recovery, malformed domains, phase rejection and S-expression dump.

After aligning first-class callable conversion with the existing path, the
`comptime_clause_inputs` and `library_meta_objects` suites passed again (25 tests,
`/tmp/luna-tools-pw95y44k/result.json`). Native verification through
`python3 -B development/host_examples.py comptime-clause-inputs --release-driver
--compare-optimization` passed at O0 and O3
(`/tmp/luna-host-examples-vs51l7cn/summary.json`). Workspace formatting check
passed (`/tmp/luna-tools-2gl_ptj0/result.json`).
The existing CMTT and library protocol regression suites also pass (527 + 25
tests, `/tmp/luna-tools-t6jiyu4l/result.json`), using `python3 -B
development/check.py --timeout 900 -- test --release -p luna --test cmtt
--test library_protocols`.

Named/defaulted/spread clause routing validation in `/home/fin/luna`:
`python3 -B development/check.py --timeout 900 -- test --release -p luna
--test comptime_clause_inputs --test cmtt --test library_protocols` passed
568 tests (16 + 527 + 25), `/tmp/luna-tools-8w2c01tg/result.json`.
Coverage includes default declaration scope, saved input reuse, explicit override
of a nonterminating default, spread producer cardinality, early return before
defaults, duplicate/unknown names and unused-body rejection with a later Type
input. The expanded native example passed O0/O3 using the same host command above
(`/tmp/luna-host-examples-xjze21ce/summary.json`). The initial added fixtures had
an undeclared Type alias and an omitted function return type; both were corrected
before the final run. Final workspace formatting check passed at
`/tmp/luna-tools-zh7a833z/result.json`.

Inherited clause validation in `/home/fin/luna` with the same three-suite command
passed 572 tests (20 + 527 + 25), `/tmp/luna-tools-ln3a8ctw/result.json`.
New cases exercise fixed outer values, inherited Type/value defaults, recursive
local instantiation and rejection of attempts to supply inherited slots.
After aligning the default-cycle key with the local clause slot, the final
`comptime_clause_inputs` suite passed all 20 tests again
(`/tmp/luna-tools-b9azgx5m/result.json`). The expanded native example passed
at O0/O3 (`/tmp/luna-host-examples-b3uvnywp/summary.json`), and workspace
formatting passed (`/tmp/luna-tools-l07eqbqk/result.json`). Commands use the same
check, host example and formatting entrypoints described above; all exited 0.

Ordinary-default integration validation in `/home/fin/luna`:
`python3 -B development/check.py --timeout 900 -- test --release -p luna
--test comptime_clause_inputs --test default_calls` passed 67 tests (24 + 43),
`/tmp/luna-tools-dkrerx2l/result.json`. The first combined run exposed value
rigids incorrectly entering associated Type normalization. Default traversal now
preserves value terms for instance substitution while normalizing Type terms.
Before that final frontend correction, 166 VM unit tests, 79 luna unit tests and
527 CMTT tests passed in `/tmp/luna-tools-xu6i8wj2/result.json`; that combined
command exited 101 because the four new source cases failed, and is not recorded
as an overall pass. Native defaults/overrides passed O0/O3
(`/tmp/luna-host-examples-2r0hwxjx/summary.json`) and final workspace formatting
passed (`/tmp/luna-tools-7nm09mi_/result.json`).
Final CMTT regression also passed all 527 tests using `python3 -B
development/check.py --timeout 900 -- test --release -p luna --test cmtt`
(`/tmp/luna-tools-3gz17vq3/result.json`, exit 0).

Ordinary-requirement integration validation in `/home/fin/luna`:
`python3 -B development/check.py --timeout 900 -- test --release -p luna
--test comptime_clause_inputs --test default_calls --test cmtt
--test library_protocols` passed 622 tests (27 + 43 + 527 + 25),
`/tmp/luna-tools-m0o8ce98/result.json`. Before the fix the valid ordinary-Type
requirement reproduction failed with "expected 2 arguments, got 0"
(`/tmp/luna-tools-trvcf1h0/result.json`, exit 101). Tests now cover both equality
and trait requirements, rejected mismatches in unused callers, and inherited
Type/default combinations. Native O0/O3 passed
(`/tmp/luna-host-examples-a7vlld69/summary.json`), and workspace formatting passed
(`/tmp/luna-tools-465xaxac/result.json`).

Declaration formation-contract validation in `/home/fin/luna` used the same
four-suite command and passed 625 tests (30 + 43 + 527 + 25),
`/tmp/luna-tools-40cp5s36/result.json`. The initial reproduction accepted an
invalid fixed trait clause in an unused caller
(`/tmp/luna-tools-6e_v8u64/result.json`, exit 101). Connecting formation checks
then exposed a constructor-only deferral guard for trait capabilities; extending
that guard to comptime functions retained the invocation check instead of trying
to prove unknown value indices during definition checking. Tests cover fixed
trait/equality/default-Type rejection and both successful and failed invocation
of a value-indexed capability. Native O0/O3 passed
(`/tmp/luna-host-examples-kxlburmh/summary.json`); final formatting passed
(`/tmp/luna-tools-eboe7tw4/result.json`).

Inherent-method validation in `/home/fin/luna` used the same four-suite command
and passed 631 tests (36 + 43 + 527 + 25),
`/tmp/luna-tools-po75vjv_/result.json`. The initial static-method reproduction
failed by attempting to obtain a local clause input before invocation
(`/tmp/luna-tools-xeuzwa5c/result.json`, exit 101). New tests cover generic owner
Types, stored static-method snapshots, ordinary defaults, fixed-contract rejection
and receiver/clause/argument side-effect order, with and without a tuple spread.
The native example includes a receiver method with a spread and omitted default;
O0/O3 passed (`/tmp/luna-host-examples-yfxezr7e/summary.json`). Workspace formatting
passed (`/tmp/luna-tools-q7mcps_v/result.json`).

### Schema Type evidence

Known local schema evidence now survives immutable aliases and nested record
projections. The previous `let schema = Types {word: u8}; let alias = schema;
let Word = alias.word;` definition check lost the completed Type. It now retains
statically known aggregate terms without invoking factories or unfolding heads.
Calls, writes and address escape conservatively invalidate these aggregate
snapshots; address checking invalidates before selecting the actual local place.
Already extracted immutable Type identities remain available. This does not
resolve the dynamically built local Type quote described below.

Validation in `/home/fin/luna`: `python3 -B development/check.py --timeout 900 --
test --release -p luna --test cmtt --test cmtt_methods --test builder_invocations`,
exit 0, 720 tests (`/tmp/luna-tools-kknwhod8/result.json`). Cases cover nested
schemas, alias projection, direct/conditional writes, address escape, calls,
nonexecuted unknown factories, and field-pointer writes retaining extracted Type
identity. Targeted Rust formatting and `git diff --check` pass.

Whole-schema pointer transfer also works when both the schema and Type have
explicit Copy capabilities and Drop is bound. The earlier failure
(`/tmp/luna-tools-komi1lr1/result.json`) omitted Type's capability; it was not a
general prohibition on Type pointer copies. The regression
`generator_schema_pointer_copy_requires_type_and_outer_capabilities` verifies
whole-schema reads after pointer writes while preserving an already extracted
Type identity, then separately rejects omission of either Copy implementation.
Validation in `/home/fin/luna`: `python3 -B development/check.py --timeout 900 --
test --release -p luna --test cmtt generator_schema_pointer_copy -- --nocapture`,
exit 0 (`/tmp/luna-tools-evveahgk/result.json`). This does not grant automatic Copy
to Type or permit runtime Type storage.

A code generator can construct a StructBuilder, publish its Type, reflect the
result and choose runtime code using the reflected data. The source regression
`code_generator_reflects_its_published_struct_to_select_runtime_code` passes
(`/tmp/luna-tools-qv203wez/result.json`, exit 0). Reproduce in `/home/fin/luna` with
`python3 -B development/check.py --timeout 900 -- test --release -p luna
--test builder_invocations code_generator_reflects -- --nocapture`.
`python3 -B development/host_examples.py builder-code-generator --release-driver
--compare-optimization` also passes at O0/O3
(`/tmp/luna-host-examples-jjh5w6sj/summary.json`).

This does not establish generated local type use inside a quote. The attempted
`'(T {flag:true}.flag)` after `let T = builder.Build()` failed during generator
definition checking with `required comptime input ... is not available in this
invocation` (`/tmp/luna-tools-61rqqndc/result.json`, exit 101).
`development/repros/builder-local-type-in-quote.fl` retains that distinct case.
The abstract check in `bind_generator_pattern` retains only known Type evidence;
it does not execute preceding mutable builder operations to derive T's shape.
Resolving this requires an explicit formation/deferred-checking strategy, not
unconditionally running generator bodies while checking their definitions.

Default-code publication has a separate gap: contextual code currently lives in
elaboration's `ExpressionCode`/`PatternCode` environments, while the VM Value and
StructBuildRequest do not carry such witnesses. Concrete NIR struct descriptors
contain field types, but construction defaults still use source HIR owners.
The existing HIR default assembler tests do not bridge these interfaces. A full
implementation must transport checked code with its context, retain its owner
through publication, and connect omitted defaults through ordinary construction;
copying only an expression or adding an unchecked builder argument is insufficient.

## Editable struct reflection drafts

`meta.StructBuilder.from_type(subject, .name = .Generated)` copies ordered field
names and applied Types into an ordinary editable draft. The subject is evaluated
once, and may name a concrete generic application or a struct published earlier
in the same invocation. Extending one draft preserves saved snapshots. Every
`Build` publishes a fresh nominal identity; this operation does not copy field
defaults, attributes, methods or trait evidence.

Validation in `/home/fin/luna`: `python3 -B development/check.py --timeout 900 --
test --release -p luna --test builder_invocations --test source_reflection`,
exit 0, 63 tests (`/tmp/luna-tools-uju2y3ze/result.json`). Added cases cover applied
source fields, extension and runtime construction, live generated subjects,
single subject evaluation, draft independence, fresh identity, wrong subjects,
duplicate extensions and runtime rejection. The existing native methods/resource
example now uses this API: `python3 -B development/host_examples.py
generated-struct-methods --release-driver --compare-optimization`, O0 and O3 pass
(`/tmp/luna-host-examples-i5qv7cp3/summary.json`).

## Local field access cost during builder invocation

Comptime local record projections now borrow their root and copy only the selected
field, instead of checkpointing the entire record first. This applies to plain
locals and addressable Storage, with the original local-read tick and lock errors.
The selected value still consumes representation budget and retains snapshot/shared
identity behavior. Anonymous field names and domains, nominal declaration identity,
and field bounds remain checked. Nonlocal receivers, runtime execution and cleanup
schedules retain the general evaluator; no user code executes while storage is held.

This addresses repeated reads of wide builder state without dropping payload
formation checks from enum reflection. Each structural reflection query still uses
the complete checked reflection path; `from_type` continues to make five queries.

A controlled bool-field projection with unrelated tuples of 1, 1,024 and 65,536
values consumes `[3, 3, 3]` VM steps on the local path, versus
`[26, 2072, 131096]` on the unchanged general path. Both addressable and plain local
cases return the same value. This measures charged work for this operation, not a
whole-compiler or wall-clock speedup. Reproduce in `/home/fin/luna` with
`python3 -B development/check.py --timeout 900 -- test --release
-p comptime_interpretor --lib field_access -- --nocapture`; 5 tests pass, exit 0
(`/tmp/luna-tools-ts1wdva9/result.json`). Coverage includes missing bindings,
locked storage, invalid selectors, snapshots after mutation, selected-data budget
exhaustion, nominal identities and early-return fallback.

Regression validation in `/home/fin/luna`: `python3 -B development/check.py
--timeout 900 -- test --release -p comptime_interpretor -p luna --lib
--test builder_invocations --test source_reflection --test nominal_value_parameters`,
exit 0, 562 tests (`/tmp/luna-tools-mdylbiuz/result.json`). Native validation:
`python3 -B development/host_examples.py mirrored-enum generated-enum
generated-struct-methods --release-driver --compare-optimization`; all three
examples build and run successfully at O0 and O3
(`/tmp/luna-host-examples-9cdhc2ni/summary.json`).

## Source enum structural reflection

The public meta wrappers expose ordered group paths, variant paths, shapes, named
payload field names and instantiated payload Types. `EnumStructure` carries a
closed query selector and one Type subject, with a stable S-expression dump and
ordinary visitor/copy/type-rewrite integration. Runtime placement is rejected.
The subject evaluates once, early control flow propagates, and the VM checks the
result List domains before returning the reflected structure.

`EnumBuilder.from_type` uses these queries to create an editable concrete draft.
It supports instantiated source enums and enums already published in the current
invocation, preserving paths, shapes and payload identities. Building the draft
produces a fresh nominal identity. It copies logical structure, not encodings,
attributes, default code or implementations; declaration order never implies a
storage discriminant. The native example is `host-mirrored-enum.fl`.

Validation in `/home/fin/luna`: `python3 -B development/check.py --timeout 900 --
test --release -p luna -p nir -p comptime_interpretor --test builder_invocations
--test source_reflection --test enum_groups --test phases --test type_rewrite
--test copy_budget --lib`, exit 0, 356 tests
(`/tmp/luna-tools-qjdw0lnl/result.json`). Source cases cover instantiated generic
subjects, current-invocation generated subjects, editable copies, nominal identity,
wrong subject/runtime rejection, early return and single subject evaluation.
All five queries are included in phase and type-rewrite coverage.
`python3 -B development/host_examples.py mirrored-enum generated-enum
--release-driver --compare-optimization`, exit 0, both examples pass at O0 and O3
(`/tmp/luna-host-examples-cnf9bory`). Targeted formatting and diff checks pass.

## Segmented enum group paths

EnumBuilder now retains group and variant paths as `List<List<Symbol>>`.
`add_group(name, .parent = [...])` declares a group, and each variant method accepts
`.group = [...]`; omitted paths retain root behavior. The compiler-private
`build_enum_paths` entry lowers to the same BuildEnum operation with typed nested
path lists. The existing flat-name private entry remains available to old hosts.

The VM validates every segment before joining it: empty paths, empty segments and
embedded dot separators are rejected. Allocation and joining consume invocation
fuel before creating the canonical dot-separated path. Ordinary enum admission
then checks parent declarations, duplicate paths and group/leaf collisions.
No general Symbol concatenation or new source syntax is introduced. Groups remain
metadata on the outer enum; they do not create a second type or payload layer.
Qualified group prefixes are kept as paths during projection lookup instead of
being evaluated as static method receivers. This also applies to aliases of
ordinary source enums; a regression covers both source and generated aliases.

Validation in `/home/fin/luna`: `python3 -B development/check.py --timeout 900 --
test --release -p luna --test builder_invocations --test source_reflection
--test enum_groups --test enum_configurations --test encoded_enums
--test nominal_value_parameters`, exit 0, 346 tests
(`/tmp/luna-tools-ax72kmzb/result.json`). The earlier combined run passed 153 VM
and 79 luna library tests before exposing the alias projection bug
(`/tmp/luna-tools-91t7zygp/result.json`, overall exit 101). The final source run
covers its correction and ordinary member projection regressions. Rejecting group
receiver probing only at static-method lookup was insufficient; complete group
paths now resolve before member/place probing.
`python3 -B development/host_examples.py generated-enum --release-driver
--compare-optimization`, exit 0, nested group payloads produce `(bool true)` at
both O0 and O3 (`/tmp/luna-host-examples-6zgsjo7p`). Formatting/diff checks pass.

## Source concrete EnumBuilder

The private `build_enum` builtin now lowers typed requests to BuildEnum, and public
`meta.EnumBuilder` provides `new`, `add_unit`, `add_tuple`, `add_named`, `add_group`
and `Build`. Required invocation (including deferred sessions) and explicit driver
invocation configure the enum callback alongside the struct callback. Both use one
AllocatedDefinitionPublisher; registration and output collection retain both kinds
and the complete reserved identity frontier.

Generated enum aliases participate in ordinary constructors and pattern matching.
Named payload construction no longer requires a source HIR owner when all fields
are supplied; missing fields on generated descriptors produce diagnostics instead
of attempting to read nonexistent source defaults. Types retain nominal identity.
The native example is `development/examples/host-generated-enum.fl`.

This entry supports ordinary concrete enums. Encoded enums, generic clauses,
attributes and default payload expressions remain outside this builder's request
model. Group paths are assembled through the segmented interface above. Legacy HIR-only StructBuildService implementations explicitly reject
enum requests; source drivers use the shared allocated service.

Validation in `/home/fin/luna`: `python3 -B development/check.py --timeout 900 --
test --release -p luna -p elaboration --test builder_invocations
--test source_reflection --test enum_groups --test enum_configurations
--test encoded_enums --test allocated_definition_publisher
--test checked_struct_publisher`, exit 0, 90 tests
(`/tmp/luna-tools-vwdj2f3l/result.json`). The first run exposed an unconditional
source-owner unwrap during named payload construction; the implementation fix and
missing/unknown/duplicate field regressions are included in the passing run.
`python3 -B development/host_examples.py generated-enum --release-driver
--compare-optimization`, exit 0: native O0 and O3 both produce `(bool true)`
(`/tmp/luna-host-examples-d2firinu`). Formatting and diff checks pass.

## Enum construction in NIR and the comptime VM

`ExprKind::BuildEnum` is a comptime operation with an explicit S-expression dump.
Copy budgeting, type rewriting, expression visitors, materialization, required
invocation dependency traversal and verification operand traversal preserve its
request. Phase validation rejects it in runtime bodies even if its result has been
misannotated as a runtime type. IIR interpreters have no enum builder configured.

The internal request operand evaluates once to a tuple of enum name, group names,
variant names, shape symbols, lists of field names and lists of field Types.
Shape symbols are `unit`, `tuple` and `named`; their field constraints remain
separate. Lists retain the ordinary meta.List normal-form identity. Evaluation
propagates early control flow before invoking the host. The VM checks list domains,
parallel lengths, duplicate variant/field names and formed payload Types before
calling its `with_enum_builder` callback.

The callback borrows the VM allowance and cannot replenish it. Its result identity
must name a returned descriptor matching the requested name, groups, shapes and
payloads; comparison work is charged before walking those trees. The VM then uses
transactional enum installation, so a failed host commit removes the tentative
descriptors. The source frontend and public builder now connect to this executable IR path.

Validation in `/home/fin/luna`: `python3 -B development/check.py --timeout 900 --
test --release -p comptime_interpretor -p nir -p elaboration -p luna --lib
--test phases --test type_rewrite --test copy_budget
--test allocated_definition_publisher --test builder_invocations
--test source_reflection`, exit 0, 610 tests
(`/tmp/luna-tools-99rtg0t8/result.json`). New VM cases cover installation and
reflection, shape/runtime rejection without host calls, mismatched host output,
failed commit rollback, early return and attempted host budget refund. NIR
regressions also visit/rewrite/copy the new node and reject runtime placement.
Initial checks found a missing IIR interpreter field initialization and a test
import; both were fixed before this passing run. Formatting and diff checks pass.

## Shared request assembly and identity allocation

`EnumBuildRequest` retains the enum name, ordered groups and ordered variants with
unit/tuple/named shapes and payload Types. Its S-expression dump escapes names and
preserves these distinctions. `PreparedEnumBuild` carries the admitted immutable
batch, reserved result identity and host commit callback, matching the existing
struct publication boundary.

`CheckedDefinitionPublisher` now assembles both request kinds after charging their
names, fields and nested Type trees. `AllocatedDefinitionPublisher` reserves their
identities from one source frontier and returns both descriptor catalogs together.
Structural assembly exhaustion allocates no identity; after reservation, failed
invocation checking, dropped candidates and stale commits never reuse that ID.
Both assembly and invocation work retain their consumed budgets on rejection.

Existing checked/allocated struct publishers are adapters over this common path.
The struct-only adapter cannot request enums, so its legacy result remains a list
of struct batches. The mixed service returns `PublishedDefinitions` plus the final
frontier. The source enum builtin and public meta.EnumBuilder now use this host API.

Validation in `/home/fin/luna`: `python3 -B development/check.py --timeout 900 --
test --release -p elaboration -p luna --test allocated_definition_publisher
--test checked_definition_publisher --test checked_struct_publisher
--test builder_invocations --test source_reflection`, exit 0, 66 tests
(`/tmp/luna-tools-maccaq78/result.json`). Four new tests cover mixed allocation
and live-VM installation, shape/payload retention, abandoned/rejected/stale ID
reservations, structural versus invocation exhaustion, identity overflow and
escaped request dumps. The first run found test-only borrow scopes extending
past publisher consumption; those scopes were corrected before this passing run.
Targeted Rust formatting and `git diff --check` pass.

## Checked concrete struct and enum publication

`CheckedDefinitionPublisher` now combines the shared declaration admission session
and incremental finite-storage checker. Both struct and enum batches use the same
committed world and revision. A preparation retains immutable descriptors and a
private commit callback; it publishes neither descriptors nor representation proofs.
A successful commit installs both checking states and retains the generated batch.
Stale candidates are rejected before representation publication, and a live VM's
outer installation rolls back if that commit rejects the candidate.

`CheckedStructPublisher` delegates to this service after charging and constructing
its request descriptor. Existing source struct builders therefore use the shared
implementation. The new enum path accepts ordinary concrete, closed enum batches,
including forward payload references; malformed formation and infinite by-value
storage reject the entire batch. Pointer recursion stays legal. Descriptor kinds
and declaration identities are preserved when extracting committed additions.

This completes the checked concrete enum publication service. Source EnumBuilder construction and dispatch are now connected as described
above. Generic/encoded enum generation needs additional contracts.

Validation in `/home/fin/luna`: `python3 -B development/check.py --timeout 900 --
test --release -p elaboration --test checked_definition_publisher
--test checked_struct_publisher --test representations --test generated_enums
--test struct_publisher`, exit 0, 37 tests
(`/tmp/luna-tools-vhnnwnic/result.json`). Five new cases exercise the live VM,
shared ownership, alternating kinds, forward batches, formation/representation
failure, cross-kind stale receipts, external admission rejection and all
insufficient preparation budgets. Targeted formatting and diff checks pass.
Source regression with the same wrapper: `test --release -p luna
--test builder_invocations --test source_reflection`, exit 0, 49 tests
(`/tmp/luna-tools-dhwxxx_4/result.json`).

## Shared declaration admission session

`DefinitionAdmissionSession` retains committed struct and enum descriptions in
one non-executing VM. `StructAdmissionSession` remains a compatibility alias.
Struct preparation and `prepare_enums` return opaque immutable receipts sharing
one world identity and revision. Either kind of successful commit invalidates all
competing receipts; failed callbacks and unwinding roll back the candidate without
invalidating other prepared work. Newly committed enum payloads may refer to prior
structs, and later structs may refer to those enums. Declaration identity collisions
are checked across both catalogs.

Preparation borrows the current caller's budget and prepays installation work;
commit needs no new fuel. Dropping a receipt publishes nothing and releases its
shared batch. This adds enum admission to the reusable checking boundary; it does
not itself expose a source EnumBuilder; the checked publisher above combines it
with finite representation checking.

Validation in `/home/fin/luna`: `python3 -B development/check.py --timeout 900 --
test --release -p comptime_interpretor --lib --test struct_admission_session`,
exit 0, 148 VM tests and 9 admission session tests
(`/tmp/luna-tools-7g3epk3i/result.json`). New regressions cover alternating kinds,
cross-kind stale receipts, foreign sessions, ownership release, callback error and
panic rollback, and every insufficient enum preparation allowance followed by a
successful prepaid commit.
Downstream validation with the same wrapper: `test --release -p elaboration
--test checked_struct_publisher --test representations --test generated_enums
--test struct_publisher`, exit 0, 32 tests
(`/tmp/luna-tools-9_op8fl8/result.json`). Targeted Rust formatting and
`git diff --check` pass.

## Live concrete enum publication

`representations::IncrementalChecker::prepare_batch` now checks mixed struct/enum
batches against the committed representation world. Forward payload edges may
cross kinds; by-value cycles are rejected and pointer recursion remains finite.
Both descriptor catalogs retain immutable shared batches, and all roots share one
revision-bound receipt. Failed/dropped candidates cannot retain successful subroot
proofs; a commit of either kind invalidates competing receipts. Existing struct
callers keep using `prepare`, which delegates to the mixed path with no enums.
Receipt Display includes enum identities in the S-expression dump.

This supplies the finite-storage prerequisite used by the checked publisher above;
it does not connect source EnumBuilder dispatch or replace declaration admission.
Validation in `/home/fin/luna`: `python3 -B development/check.py --timeout 900 --
test --release -p elaboration --test representations`, exit 0, 18 tests
(`/tmp/luna-tools-t9lb9eou/result.json`). New cases cover cross-kind cycles and
collisions, pointer recursion, foreign/stale receipts, shared ownership release,
budget failure/retry and a 256-layer alternating chain with stable per-kind fuel
cost after the first layer.
The related `--test checked_struct_publisher --test struct_publisher
--test generated_enums --no-fail-fast` run passes 14 tests using the same
`development/check.py --timeout 900 -- test --release -p elaboration` entrypoint
(`/tmp/luna-tools-juu711o7/result.json`, exit 0). Formatting/diff checks pass.

The comptime VM now exposes `install_generated_enums`, its transactional `_with`
variant and `install_owned_generated_enums_with`. Its enum catalog retains either
borrowed descriptors or shared immutable batches, so a running VM can outlive the
producer's temporary batch handle without copying old catalogs or payload trees.
Reflection, variant construction and name observation use the same catalog.

Admission checks declaration identity across kinds, closed payload Types, variant
shapes, named-field arity/uniqueness, group paths and formation requirements. All
candidate identities are temporarily visible together for forward references.
Admission error, publisher error, solver unwind or publisher unwind removes the
entire candidate batch and retains previous declarations. Failure consumes budget;
insertion/rollback work is prepaid. The host commit callback must itself preserve
its own publication state on failure.

This boundary accepts ordinary concrete enums in a comptime declaration VM.
It rejects encoded/intrinsic and generic enums, abstract invocation scope and
runtime/IIR execution. The caller must reserve identities and perform ordinary
semantic/representation checking first; VM admission is not a finite-layout proof.
Source EnumBuilder and source dispatch are connected through the checked service above. Existing encoded source enums remain supported through initial
package construction.

Validation in `/home/fin/luna`: `python3 -B development/check.py --timeout 900 --
test --release -p comptime_interpretor --lib`, exit 0, 148 tests
(`/tmp/luna-tools-vuduq2m5/result.json`). Seven new cases cover shared ownership,
construction/name/reflection, forward pointers, invalid payloads, commit failure,
unwind, collision/shape/scope rejection, empty-enum formation, phase checks and
every insufficient admission budget followed by a successful corrected retry.

Source integration command: `python3 -B development/check.py --timeout 900 -- test
--release -p luna --test enum_groups --test enum_configurations --test encoded_enums
--test source_reflection --test builder_invocations --no-fail-fast`
(`/tmp/luna-tools-kpyqawl3/result.json`). It passes 70 tests and exits 101 for one
unchanged `enum_configurations` assertion: `.base_type Missing` is accepted at
`Emit::Hir`, where that test expects rejection. `frontend.rs` returns the HIR dump
before entering elaboration/invocation for this mode, so the failing path does not
use the VM catalog changed here. The assertion is preserved for separate diagnosis;
this run is not reported as wholly passing. All 14 encoded-enum, six enum-group,
12 reflection and 37 builder tests pass. Targeted formatting and diff checks pass.

Follow-up diagnosis: `ast_lowering/src/expr.rs` deliberately preserves unresolved
attribute/configuration names for elaboration, while still reporting access errors.
The obsolete assertion has been replaced with phase-specific coverage for all three
encoding configuration values: valid NIR baseline, unresolved HIR node preserving
the name, and rejection by NIR elaboration. A private alias is rejected at HIR;
making that same alias public succeeds through NIR. No compiler behavior changed.
The same source integration command now passes all 73 tests
(`/tmp/luna-tools-x2ojsbbn/result.json`, exit 0), resolving the previously reported
failure without suppressing unknown-name rejection.

## Nested descriptors during early reflection

Computed nominal applications now use the same descriptor closure as literal
applications, including the dependent-domain input path. Argument expressions
remain in NIR and execute once in source order. The regression exercises plain
Type inputs, a later value input whose domain is the earlier Type, and side effects
encoding invocation order as 12. It consumes the reflected fields through public
`meta.fold` and executes a method on the resulting generated record.

Validation in `/home/fin/luna`: 34 builder tests pass via
`python3 -B development/check.py --timeout 900 -- test --release -p luna
--test builder_invocations` (`/tmp/luna-tools-q9lr2bk5/result.json`, exit 0).
The same production revision passed 450 elaboration and 389 open-function tests
in `/tmp/luna-tools-ot8nd0pp/result.json`; that combined run exited 101 because an
additional builder test initially used incorrect block syntax. Its corrected
version exposed the separate indexing limitation below. The final builder group
uses ordinary `meta.fold` and passes all four computed-input cases.
`python3 -B development/host_examples.py generated-nested-struct --release-driver
--compare-optimization` passes LLVM O0 and O3
(`/tmp/luna-host-examples-yqf3ouxq`, exit 0), now with a computed dependent-domain
source application.

The later flow integration fixes direct Type/Symbol list indexing after effectful
arguments (original failure: `/tmp/luna-tools-vq7x0oy_/result.json`). Machine
arithmetic activates a whole-body flow walk; the missing index case was unrelated
to nominal descriptor preparation. The walker now visits list and index operands
in order, returning no arithmetic or alias evidence for the Type/Symbol result.
Struct reflection/publication operands also remain observable to the walk.
The VM's checked indexing is unchanged. Primitive elements now retain their domain
bounds, with no invented constant contents or relationship to another observation.
Object and structured element domains still require their own alias transfer;
this is not a general collection proof model or literal-element propagation.

The former repro is now `development/examples/host-effectful-reflection-index.fl`.
`python3 -B development/check.py --timeout 900 -- test --release -p luna
--test builder_invocations --test integer_contracts --no-fail-fast` passes 36
builder and 15 integer-contract tests (`/tmp/luna-tools-m1f00735/result.json`,
exit 0). The regression encodes receiver/name-index/type-index evaluation order as
123, rejects both negative and out-of-range indices with `IndexOutOfBounds`, and
rejects machine overflow in either operand with a range diagnostic.
`python3 -B development/host_examples.py effectful-reflection-index --release-driver
--compare-optimization` returns `(bool true)` at LLVM O0 and O3
(`/tmp/luna-host-examples-ljiy1318`, exit 0).
The related `--test elaboration --test source_reflection` run passes 450 and 12
tests respectively (`/tmp/luna-tools-c1316ccl/result.json`, exit 0), using the same
`development/check.py --timeout 900 -- test --release -p luna` entrypoint.

The same flow path now handles a Never-valued index: returning from an operand
preserves prior effects and skips the remaining operand/index operation. Returning
from the receiver likewise prevents evaluation of an overflowing index. Regression
cases use Type, Symbol, bool and u8 lists where no element escapes the early return;
ordinary returning bool/u8 indexing is now handled by primitive domain bounds.
The original early-return case failed with `unsupported-construct`
(`/tmp/luna-tools-bmfhpw_s/result.json`). Validation in `/home/fin/luna`:
`python3 -B development/check.py --timeout 900 -- test --release -p luna
--test builder_invocations --test integer_contracts --test source_reflection
--no-fail-fast` passes 37, 15 and 12 tests (`/tmp/luna-tools-w8_oqwz0/result.json`,
exit 0). The `effectful-reflection-index` native example now uses an early-returning
comptime `FirstType` helper; the same host command passes O0 and O3
(`/tmp/luna-host-examples-4cc9bslj`, exit 0).

Scalar-index validation in `/home/fin/luna`: `python3 -B development/check.py
--timeout 900 -- test --release -p luna --test comptime_list_operations
--test builder_invocations --test integer_contracts --test target_integers
--no-fail-fast` passes 26, 37, 15 and 27 tests respectively
(`/tmp/luna-tools-6bz9ee77/result.json`, exit 0). The new regression executes
guarded byte increments and Boolean-controlled counters. It rejects an unguarded
increment, an unrelated Boolean guard, and reuse of an old range after assigning
a fresh indexed value. `python3 -B development/host_examples.py
comptime-scalar-index --release-driver --compare-optimization` passes O0 and O3
(`/tmp/luna-host-examples-jfzpam6e`, exit 0). The example uses the computed scalar
results to select a runtime Type without granting an unproved arithmetic bound.

Required Type expressions now prepare the reachable nominal descriptor graph,
including nested fields, applied type arguments, parameter domains, enum payloads
and newtype representations. A budgeted structural walk collects nominal identities;
normal-form heads remain opaque. Each declaration is visited once per preparation,
so recursive pointer fields terminate without erasing their type identity. Both
ordinary Type expressions and literal generic applications use this preparation.

The regression originally failed when reflecting `Source { inner: Inner }` before
the impl world was ready (`/tmp/luna-tools-yn9ruwv9/result.json`). It now executes a
generated record's nested field read, including the `Generic<Inner>` source variant.
A separate case reflects and republishes a pointer to a recursively defined Node.

Validation in `/home/fin/luna`:

- `python3 -B development/check.py --timeout 900 -- test --release -p luna
  --test builder_invocations --test elaboration --no-fail-fast`: exit 0,
  33 builder and 450 elaboration tests (`/tmp/luna-tools-uowt2255/result.json`).
- `python3 -B development/check.py --timeout 900 -- test --release -p nir
  --test type_computations`: exit 0, 14 tests including budget exhaustion and
  opaque-head separation (`/tmp/luna-tools-6upujtoh/result.json`).
- `python3 -B development/host_examples.py generated-nested-struct
  generated-struct-methods --release-driver --compare-optimization`: exit 0,
  both examples return `(bool true)` at LLVM O0 and O3
  (`/tmp/luna-host-examples-k55gkfhk`).

## Generated types as ordinary implementation subjects

An impl head can now demand a Type produced through public `meta.StructBuilder`
methods before the complete trait world is ready. Early inherent selection builds
the complete source candidate set for the requested name, checks coherence and
replays applicability evidence. It retains receiver type arguments and rejects
competing later candidates. Trait-dependent candidates still wait for the complete
implementation world; this path does not publish a partial trait index.

Early reflection and struct construction demand `meta.List` metadata. Nominal
Type values in the required-expression path also demand their source descriptor
before invocation. This repairs the public reflection wrapper failure that was
not exercised by direct intrinsic builder tests.

Generated structs accept ordinary source inherent and trait implementations;
this does not yet expose a source API for generating methods or impl definitions.
The native example `development/examples/host-generated-struct-methods.fl` reflects
two fields, builds a type, provides Once/Clone/Copy and an inherent method, forwards
the result through a generic function, explicitly clones it and mutates the original.
Both independent copies retain the earlier field values.

Validation in `/home/fin/luna`:

- `python3 -B development/check.py --timeout 900 -- test --release -p luna
  --test builder_invocations --test elaboration --no-fail-fast`: exit 0,
  31 builder and 450 elaboration tests passed
  (`/tmp/luna-tools-bu_wc0p0/result.json`).
- `python3 -B development/host_examples.py generated-struct-methods
  --release-driver --compare-optimization`: exit 0, LLVM O0 and O3 both return
  `(bool true)` (`/tmp/luna-host-examples-0scoftvw`).
- Targeted Rust formatting and `git diff --check`: exit 0.

## Generated layouts before behavioral analysis

Body/default/template invocations may publish structs after the initial layout
snapshot. Their descriptions now enter the output before contract inheritance,
default purity and quote expansion. Identity-set merging preserves existing
descriptions and avoids repeated full-vector searches. Quote-generated internal
packets remain intact.

The regression binds Copy/Drop, builds a bool-field struct inside a runtime pure
function and passes its field to a pure quote function. The old synchronization
order rejects `run` for missing P4 evidence (`/tmp/luna-tools-8630pehm/result.json`);
the repaired order accepts it and execution returns true. A preliminary test
without resource protocols had an independent cleanup-evidence problem and was
replaced with the explicitly bound case.

Validation in `/home/fin/luna`: `python3 -B development/check.py --timeout 900 --
test --release -p luna --test builder_invocations --test quote_functions --
--skip quote_implementations_supply_ordinary_trait_callable_entries`, exit 0,
26 builder and 103 quote tests passed (`/tmp/luna-tools-ss1f8l9v/result.json`).
The excluded test independently expected `.callable` but observed the existing
`::callable` adapter name. That name was subsequently corrected in a separate
change. The complete `--test quote_functions` group then passed all 104 tests,
exit 0 (`/tmp/luna-tools-defoous_/result.json`), including direct trait calls,
generic forwarding and callable-value adapters.

## Immediate required source invocation

Immediate required calls now install a checked publisher for both Type results
and other comptime values. Generated descriptions return to the current source
checker, enter its nominal/layout tables and appear in subsequent invocation
snapshots and the completed NIR. `typealias` initializers and constants can use
`meta.StructBuilder`; runtime construction, projection and field writes use the
same checked descriptors. Missing fields produce diagnostics even though these
concrete, body-free declarations have no HIR field-default owner.

The checker allocates from the complete HIR source identity frontier and records
`definition_identity_cursor` in NIR, with an S-expression dump field. The explicit
invocation driver uses this frontier and checked snapshot publication as well,
so it neither reexecutes typealias initializers nor reuses their identities.
Inactive builder paths consume no structural budget. Required source calls use
the existing required-invocation allowance; explicit calls additionally retain
their caller-supplied structural allowance. Failed reservations are not reused
within the publisher. Ordinary source defaults continue to use their HIR owners.

Saved/deferred open-function annotations, staged defaults, computed constructors
and computed projection receivers now use the same checked publication boundary.
Prepared snapshots record the definition identity frontier. If a caller has
generated definitions since preparation, annotation computation is prepared again
from source with completed inputs, without replaying their evaluation or copying
active inference state into the saved computation. Fresh type-proposition contexts
also admit checked generated nominal identities.

Explicit staged arguments skip their default suppliers, including suppliers that
would fail. Omitted defaults can generate fresh, distinct types. A default that
returns a struct does not authorize field construction on an arbitrary `T: Type`
inside a generic body; that still requires appropriate evidence.

Concrete fields must be closed Types. Generic declaration generation, field defaults,
attributes, methods and implementation evidence remain separate work. The
remaining sections retain earlier validation evidence; this section supersedes
older statements that initial or deferred required invocations lack a publisher.

Deferred publication validation in `/home/fin/luna`:

- `python3 -B development/check.py --timeout 900 -- check --release -p luna`:
  exit 0 (`/tmp/luna-tools-p1ng0v5j/result.json`).
- `python3 -B development/check.py --timeout 900 -- test --release -p luna
  --test builder_invocations --test open_functions --test elaboration
  --no-fail-fast`: 864 passed (25 builder, 389 open-function, 450 elaboration),
  exit 0 (`/tmp/luna-tools-55wup078/result.json`).

These tests cover publication once per formed domain, reflection of a type newer
than the saved snapshot, omitted versus supplied defaults, computed construction,
distinct generated nominal requirements and rejection of unsupported generic field
construction. The initial test draft incorrectly constructed fields on an
unconstrained type parameter; it was corrected and retained as a rejection test,
without weakening generic checking.

Validation in `/home/fin/luna`, through `python3 -B development/check.py
--timeout 900 --`:

- `check --release -p luna`: exit 0 (`/tmp/luna-tools-rgum6j19/result.json`).
- `test --release -p luna --test builder_invocations --test elaboration --test
  source_reflection --test packages --test frontend_cli --no-fail-fast`: 748
  passed, exit 0 (`/tmp/luna-tools-mg44tjfj/result.json`).
- `test --release -p elaboration --lib program::required::prepared`: 10 passed,
  exit 0 (`/tmp/luna-tools-arvfu6ly/result.json`).
- `test --release -p elaboration --test checked_struct_publisher --test
  struct_publisher --test delivered_types --test representations --no-fail-fast`:
  27 passed, exit 0 (`/tmp/luna-tools-664lf8_w/result.json`).

The source tests cover a generated typealias used at runtime, constants and
chained aliases with distinct identities, nested field assignment, subsequent
explicit generation without identity collisions, missing/invalid field
rejection and the public meta builder's reflection loop during initialization.
The first builder run caught a missing structural-budget diagnostic in the new
adapter; the corrected diagnostic passes the final run. The new allocator
unit test's temporary borrow scope was corrected before its final passing run.
Targeted rustfmt and `git diff --check` pass.

## Publication from a checked snapshot

`definitions::CheckedStructPublisher` publishes concrete, body-free struct
fields against an immutable checked NIR baseline. It never reruns the source
checker or invokes a source initializer. Completed-Type admission uses the same
VM boundary as delivered HIR field types, followed by the shared finite-storage
check. Unbound, unknown, non-Type and duplicate fields, identity collisions and
by-value cycles are rejected; legal pointer recursion remains possible.

The enclosing compiler must reserve each supplied declaration identity against
its complete source world, including declarations absent from the invocation
snapshot. The publisher does not guess a package identity or allocate from the
maximum visible NIR declaration. It returns only committed immutable batches;
merging them into source-checker tables and preserving the source allocation
frontier remain caller responsibilities. The immediate source-call adapter and
explicit driver now perform these responsibilities.

Preparation retains no candidate on failure or drop. A commit against a changed
publication revision is rejected, allowing the invoking VM to roll back its
installation. Later preparation sees earlier committed definitions and can use
them in fields. Baseline descriptions and committed batches remain borrowed or
shared. A lazy `StructAdmissionSession` now retains the validation VM and committed
descriptors, avoiding repeated VM construction and admission of prior generated
batches. Preparation admits and rolls back a tentative batch, returning an opaque
receipt tied to the session and publication revision. Commit installs only that
immutable receipt with prepaid work. Foreign/stale receipts cannot call the host;
callback failure or unwind rolls back the entries without advancing the revision.
The session exposes no program execution, host solver or live inference state.

Each attempt borrows its caller's allowance; failure and unwinding return only
the remaining fuel. Exhaustion can be retried with a separately supplied budget,
and commit cannot secretly draw on an old allowance. Finite-storage validation
now also uses a lazy append-only `representations::IncrementalChecker`. It builds
baseline indices once, borrows them through a candidate overlay, and reuses
committed finite-representation proofs. Candidate descriptions and new proofs
remain isolated in a revision-bound receipt until publication. A failed batch,
dropped receipt or stale commit cannot leave a proof for a reusable identity.
The existing representation walk still checks every new layout and uncached
generic application with the caller's budget. The optimization does not unfold
opaque normal-form heads or treat storage finiteness as type admission.
Representation failures now retain a structured category: `Budget`, `Invalid`
or `Stale`. The builder boundary maps these to `StepsExhausted`, `Type` and
`InvalidCallPlan`, respectively. Index construction, recursive expansion,
newtype expansion and proof-cache work preserve resource exhaustion instead of
misreporting an invalid type. Callers can retry with a new allowance; this does
not automatically retry invocations or guarantee that every bounded query will
complete. Ordinary source diagnostics still display the specific failure text.
Defaults, attributes, clauses,
methods and new implementation evidence require their own checking paths.

Failure-category validation in `/home/fin/luna`, via `python3 -B
development/check.py --timeout 900 --`:

- `check --release -p elaboration`: exit 0
  (`/tmp/luna-tools-igybnn1y/result.json`).
- `test --release -p elaboration --test representations --test
  checked_struct_publisher --test struct_publisher --no-fail-fast`: 26 passed
  (`/tmp/luna-tools-a_rgvkk5/result.json`).
- `test --release -p luna --test builder_invocations --no-fail-fast`: 27 passed
  (`/tmp/luna-tools-21kd_vxm/result.json`).

All commands exited 0; targeted rustfmt and diff checks passed. A publisher test
measures complete preparation, retries with one fewer step and confirms a
representation-budget `StepsExhausted` failure without publication. Renewed fuel
then commits successfully; real by-value recursion still produces `Type`.
Separate cases cover index budgets, stale receipts and budget versus malformed
argument failures inside newtype expansion.

Incremental representation validation in `/home/fin/luna`, through `python3 -B
development/check.py --timeout 900 --`:

- `check --release -p elaboration`: exit 0
  (`/tmp/luna-tools-oror4qd3/result.json`).
- `test --release -p elaboration --test representations --test
  checked_struct_publisher --test struct_publisher --no-fail-fast`: 22 passed
  (`/tmp/luna-tools-hee0xrut/result.json`). After adding the forward-reference
  batch case, `test --release -p elaboration --test representations` passed all
  12 representation tests (`/tmp/luna-tools-4_g4gscy/result.json`).
- `test --release -p luna --test builder_invocations --test source_reflection
  --no-fail-fast`: 39 passed (`/tmp/luna-tools-i24p771t/result.json`).

All commands exited 0; targeted rustfmt and diff checks passed. Regression
coverage includes foreign/stale receipts, discarded and partially checked failed
batches, budget retry, legal pointer recursion and failed cyclic batches repaired
using the same identities. With 256 baseline descriptions, a further 256 committed
wrapper types have equal per-layer metered cost after the initial proof. A source
builder loop also generates 256 wrappers, then reflects the entire distinct chain
back to its original field. The initial source fixture missed spaces around `+`;
it was corrected rather than relaxing the lexer. These are work-budget and
behavior checks, not an overall wall-clock speedup measurement.

Reusable admission validation in `/home/fin/luna`, via `python3 -B
development/check.py --timeout 900 --`:

- `test --release -p comptime_interpretor --test struct_admission_session`: 5
  passed (`/tmp/luna-tools-uz_9e_bb/result.json`). Tests cover dropped, foreign and
  stale receipts; callback rejection and panic rollback; renewed allowances after
  exhaustion; and prepaid commit without further fuel. With 256 baseline types,
  consecutive admissions cost the same fuel while a fresh session additionally
  pays baseline preparation. This measures metered work, not wall-clock speed.
- `test --release -p elaboration --test checked_struct_publisher --test
  struct_publisher --test representations --no-fail-fast`: 18 passed
  (`/tmp/luna-tools-_xrhjz3y/result.json`).
- `test --release -p luna --test builder_invocations --test source_reflection
  --no-fail-fast`: 38 passed (`/tmp/luna-tools-o2lnqk1c/result.json`).

All commands exited 0. Targeted rustfmt and `git diff --check` passed. An initial
check required an explicit error type on the publisher's preparation closure;
that compile error was corrected before the passing tests above.

Validation in `/home/fin/luna`, via `python3 -B development/check.py
--timeout 900 --`:

- `test --release -p elaboration --test checked_struct_publisher --test
  representations --test struct_publisher`: 16 passed, exit 0
  (`/tmp/luna-tools-nng08ogw/result.json`).
- `test --release -p luna --test builder_invocations --test source_reflection
  --no-fail-fast`: 26 passed, exit 0
  (`/tmp/luna-tools-y1m_6zbk/result.json`).

These include generation from a source nominal behind an opaque normal-form
head, chained generated fields and reflection inside a Type-computation session,
source-world preservation, candidate drop, stale commit with VM rollback,
independent recovery, malformed/recursive Type rejection and shared fuel limits.

## Shared representation validation

`elaboration::representations::Checker` checks completed struct, enum and
newtype descriptions against an immutable borrowed world. Source elaboration
and the current HIR builder publisher use this same boundary. The check follows
by-value optional payloads, anonymous records, error-result branches and closure
environments, in addition to nominal fields and the configured Tuple head.
Pointers and function signatures break storage recursion; unrelated normal-form
heads remain opaque. Missing nominal descriptions are rejected.

A caller supplies one shared expansion allowance. Type traversal, hashing and
field substitutions are charged before copying growing generic terms. Successful
nominal applications are memoized; failed attempts leave no active traversal
state. Reuse across an actual declaration-world mutation requires a new checker.
This is representation validation, not type formation, visibility checking or
an allocator for generated declaration identities.

Validation in `/home/fin/luna`, through `python3 -B development/check.py
--timeout 900 --`:

- `test --release -p elaboration --test representations`: 7 passed, exit 0
  (`/tmp/luna-tools-wi5g6agd/result.json`).
- `test --release -p luna --test elaboration --test newtypes --test
  builder_invocations --no-fail-fast`: source elaboration 450 passed and builder
  invocation 13 passed. The newtype group had one stale diagnostic assertion
  (`meta.List<Type>` versus the existing `meta.List<T>` diagnostic), so this
  combined command exited 101 (`/tmp/luna-tools-xxj7gkjn/result.json`).
- After the independent assertion correction, `test --release -p luna --test
  newtypes`: all 211 passed, exit 0 (`/tmp/luna-tools-dlx8wo_h/result.json`).

The source regression rejects direct, mutual and generic optional recursion,
while accepting optional pointers and callable signatures returning the owner.

## Type-computation sessions

`type_computations::Session::new_with_struct_builder` configures a checked host
publication callback before any waiters are queued. Calls share the same live
VM catalog: a generated Type can settle one inference variable, feed a dependent
builder call, and be reflected by a subsequent computation. Waiting does not
invoke the callback; completed and failed computations are terminal and do not
replay publication. A rejected commit rolls back VM admission, and an independent
later computation can publish successfully in the same session.

The host still owns declaration identity allocation and semantic publication.
The generic session API does not attach generated HIR owners. Immediate source
calls use the adapter described above to register concrete descriptors for
nominal lookup, construction and subsequent snapshots, without reentering an
initializer by rechecking the complete source package.

Validation in `/home/fin/luna`: `python3 -B development/check.py --timeout 900 --
test --release -p luna --test builder_invocations`, exit 0, 13 tests passed
(`/tmp/luna-tools-lby_n72b/result.json`). This includes dependency waiting,
chained generation and reflection, no replay or fuel use on terminal resume,
commit rejection, failed-dependency propagation and independent recovery.

## Default CLI invocation

`--invoke-comptime` uses checked snapshot publication in single-file, package-set
and sysroot modes. Compilation and invocation share one hook; source-selected
Copy/Drop identities, proof callbacks and proven-check elimination follow the
existing invocation path. Observed inputs and output source protection remain.

The explicit driver supplies a separate structural allowance of 1,000,000 for
generated descriptor preparation and output registration. VM validation uses
the invocation allowance. Unexecuted builder branches consume no structural
allowance. IIR-only staged publication remains unsupported. Calling
`Action::run` on frozen NIR alone does not install a publisher. Full repository
sysroot acceptance still has independent library gaps; the routing test uses
minimal builtin/std and real meta.

Validation in `/home/fin/luna`:

```sh
python3 -B development/check.py --timeout 900 -- test --release -p luna --test builder_invocations --test source_reflection --no-fail-fast
python3 -B development/check.py --timeout 900 -- test --release -p luna --test builder_invocations --test packages --test sysroot_preludes --no-fail-fast
python3 -B development/check.py --timeout 900 -- test --release -p luna --test frontend_cli comptime_invocation --no-fail-fast
```

Final applicable results: 59 tests at `/tmp/luna-tools-d3j33s1i/result.json`,
7 invocation regressions at `/tmp/luna-tools-967cfn8g/result.json`, and 12 source
reflection tests at `/tmp/luna-tools-nppbu0fe/result.json` (78 distinct tests).
The latter run failed its new CLI assertions before correction: standalone
selectors require `input.meta.List`, and observed paths use encoded bytes.
Final tests cover all three CLI paths, observed source inputs, source output
protection, defaults, closure execution, type dispatch, resource bindings,
budget exhaustion and output preservation. Targeted rustfmt and diff checks pass.

## Public field-only struct builder

`meta.StructBuilder.new(.name = .Columns)` creates an ordinary comptime draft;
omitting the name uses `.Generated`. `add_field(*self, name, field_type)` appends
ordered Symbol and Type lists through field pointers. `Build(self) -> Type`
delegates to checked publication, preserving opaque Type heads and rejecting
duplicate fields. Empty drafts are supported. Repeated default names do not
reuse nominal identity. The library wrapper uses no alternate type registry.
Ordinary consumers need no direct builtin access. The builder is not added to
the default prelude; use its qualified name or an explicit import.

This API covers concrete fields without defaults. Explicit comptime invocation uses the builder-enabled driver or CLI path;
deferred-invocation integration, defaults/attributes/clauses and other definition
kinds remain open.
Appending inherits the current List validation cost and is not a constant-time
draft operation. Runtime signatures cannot carry this comptime-only draft.

Validation in `/home/fin/luna`:

```sh
python3 -B development/check.py --timeout 900 -- test --release -p luna --test builder_invocations --test source_reflection --test comptime_list_operations --no-fail-fast
```

Exit 0, 43 tests passed at `/tmp/luna-tools-cr24c7mx/result.json`. Ordinary-package
tests use the real meta library without direct builtin references, copy reflected
fields through a loop and mutating builder methods, verify order and destination
identity, build independent empty types with default names, and reject duplicate
fields. Runtime construction and pass-through runtime signatures are rejected
during elaboration. Existing reflection and List tests remain green. Raw VM
fixtures now use the unique entry name `Generate` so their name-only lookup does
not accidentally select the newly introduced `Build(self)` method. Targeted
rustfmt and `git diff --check` passed.

## Driver compilation followed by a builder invocation

`luna::packages::compile_and_invoke_with_struct_builders` resolves and lowers an
explicit package set, binds its protocols and preludes, performs initial
elaboration, then invokes a selected comptime entry with a compiler-owned
publisher. It returns the invocation value, the latest checked NIR artifact and
bound protocols. If the entry creates no definitions, it returns the initial
NIR. Source package identities, dependency types, target and prelude configuration
remain those of the original compilation. Generated identities belong to the
explicit primary package. On failure no artifact or value escapes; structural
fuel retains consumed work.

`luna::invocation::invoke_in_package_with_struct_publisher` exposes the invocation
part for a host already holding the matching HIR publisher and checked NIR. It
uses ordinary default argument preparation, primary-package entry selection,
proof callbacks and invocation budgeting. Publication diagnostics accompany
invocation errors. Generated descriptors are not silently added to existing
trait/Copy evidence. Builder installation still requires the NIR interpreter;
an invocation requiring the IIR staging path cannot publish structs through it.

This is a driver API for an explicit invocation after initial elaboration. It
does not enable builders in typealias initializers or other required invocations
during that initial elaboration or provide the complete public StructBuilder
library API. The default CLI now uses this retained-HIR invocation boundary. Candidate publication
still checks the full package. Active-checker integration and incremental
semantic updates remain separate work.

Validation in `/home/fin/luna`:

```sh
python3 -B development/check.py --timeout 900 -- test --release -p luna --test builder_invocations --test generated_definitions --test reflected_definitions --test source_reflection --no-fail-fast
python3 -B development/check.py --timeout 900 -- test --release -p luna --test builder_invocations --no-fail-fast
```

The final builder suite passed 3 tests at `/tmp/luna-tools-pwzf2_v9/result.json`.
The other 15 integration tests passed at `/tmp/luna-tools-yo4gx28b/result.json`;
that earlier run failed the new fixture because it used `.opaque`, a reserved
word, as an unquoted field symbol. The multi-package fixture also needed unique
temporary directory counters and an explicit comptime signature for its helper.
No language rule was relaxed. Final coverage includes default arguments, opaque
normal-form fields, retained generated NIR, no-build results, unknown/runtime
entries, exhausted budgets and retry, dependency types, configured preludes and
primary entry identity. Targeted rustfmt and `git diff --check` passed.

## Compiler-owned struct publisher

### Transactional field-default assembly

`StructPublisher::prepare_with_defaults` connects the existing checked HIR field
assembler to the publisher's candidate/commit boundary. The host callback is
called in field declaration order with each field's hygienic identity and the
shared structural allowance. It can supply an ordinary expression or CMTT
quote/splice expression, or leave the field without a default. Ordinary
elaboration checks the complete candidate before producing a prepared result.

Rejected, discarded, or stale candidates cannot replace committed definitions.
Callback failure consumes the structural work already performed; unwind retires
the reserved structural allowance. Host callback side effects and arena
allocations are not rolled back. The callback must not capture active inference
state. The existing `prepare` method uses the same path without defaults.

Defaults remain in the HIR returned by `finish`, so later constructor elaboration
can instantiate them. The VM's concrete descriptor packet contains field Types,
not default expressions. This entrypoint does not add a source `add_field`
default parameter, code-value transport from a running invocation, or generated
method publication. Those remain necessary for the full SoA example.

The regression constructs a quoted Boolean field default, rejects a Char
replacement, retries successfully, commits, then elaborates and executes a new
consumer that omits the field. Additional checks cover field order, optional
absence, abandoned/stale candidates, callback failure and unwind. A later
publication re-elaborates the retained default, and misplaced default roots are
rejected before commit.

Validation from `/home/fin/luna` (2026-10-29):

```sh
python3 -B development/check.py --timeout 900 -- test --release -p elaboration --test struct_publisher_defaults --test struct_publisher --test delivered_types --test generated_methods
python3 -B development/check.py --timeout 900 -- test --release -p luna --test builder_invocations --test generated_definitions
```

Both commands exited 0: 18 elaboration tests and 73 source/generated-definition
tests. Reports: `/tmp/luna-tools-xge6j_e6/result.json` and
`/tmp/luna-tools-hxw8sc7s/result.json`. Formatting and diff checks passed.


`DeferredStructPublisher` borrows the source package and configuration until the
first request. It then creates one owned `StructPublisher` through budgeted HIR
forking. Both implement `StructBuildService`, so invocation uses the same checked
prepare/commit contract. Later requests observe the same committed world; skipped
branches do not initialize it, and finishing an unused service returns no new
NIR. Initialization failure retains its diagnostic and spent structural work
without changing the source package. Its S-expression dump reports whether the
world has been initialized and the remaining structural allowance.

First-use validation in `/home/fin/luna`:

```sh
python3 -B development/check.py --timeout 900 -- test --release -p elaboration --test struct_publisher -p luna --test builder_invocations --no-fail-fast
python3 -B development/check.py --timeout 900 -- test --release -p luna --test builder_invocations --no-fail-fast
python3 -B development/check.py --timeout 900 -- test --release -p luna --test frontend_cli comptime_invocation --no-fail-fast
```

Final applicable results: 4 publisher tests at
`/tmp/luna-tools-jl0lth1a/result.json`, 11 builder/driver tests at
`/tmp/luna-tools-8wlvtl5e/result.json`, and 7 invocation regressions at
`/tmp/luna-tools-054nnyi7/result.json`, all exit 0 (22 distinct tests).
Tests establish zero structural debit for skipped builder branches and resource
setup rejection, first-request initialization, unchanged source HIR, continued
publication in one world, and budget rejection before initialization. Formatting
and diff checks passed. This removes eager publication-world copying; ordinary
invocation preparation retains its existing costs.

`elaboration::definitions::StructPublisher` provides the reusable host side of
source construction. It owns the committed HIR package and the latest checked
NIR, an explicit destination resolution/semantic package identity, protocols,
target and structural allowance. `prepare` forks the committed package with
metered table copying, reserves a resolver identity, assembles delivered field
Types and runs ordinary semantic publication against the private candidate.
It returns the VM's `PreparedStructBuild`; only the final callback replaces the
committed package. A discarded, rejected or stale candidate cannot overwrite it.
`last_diagnostic` retains structural/semantic failures alongside the VM's request
source location. The service has an S-expression state dump.

Subsequent requests see earlier committed generated definitions, so field Types
may refer to those identities. `finish` returns the committed HIR and latest
checked NIR without another elaboration. Until the first successful commit the
checked output is absent. Only the new concrete descriptor is copied into shared
VM storage; its name and field-Type copying consume structural fuel. Candidate
package checking still re-elaborates the full candidate. This service is not an
incremental active-checker transaction. Explicit CLI invocation installs the
service; required invocations during initial checking still do not. Hosts must supply the relevant source world and
matching package identities; an empty world only supports self-contained fields
and earlier generated declarations.

Validation in `/home/fin/luna`:

```sh
python3 -B development/check.py --timeout 900 -- test --release -p elaboration --test struct_publisher --test delivered_types -p luna --test source_reflection --no-fail-fast
```

Final run exited 0 at `/tmp/luna-tools-uk1qmfmq/result.json`: 24 tests passed.
Coverage includes explicit destination identity, discarded/stale candidates,
semantic rejection and repair, structural-budget exhaustion, chained generated
Types and their use by a fresh VM from retained NIR. The source integration
constructs two dependent structs through this production publisher. The first
run failed to compile the new tests because candidate borrows outlived the
intended scope; explicit candidate/VM scopes fixed the test lifetime issue.
Targeted rustfmt and `git diff --check` passed.

## Source requests during an existing invocation

The compiler-private `build_struct` primitive accepts a Symbol name,
`meta.List<Symbol>` field names and `meta.List<Type>` field descriptions. It
lowers to a `build-struct` NIR node with an internal argument packet; this packet
does not define a new source tuple type. Operands execute once in source order,
and ordinary control flow escapes before any publication request. Runtime and
quoted runtime contexts reject the primitive. The field-only public
`StructBuilder.new/add_field/Build` wrapper uses this boundary; more general
definition construction remains incomplete.

`Interpreter::with_struct_builder` supplies a host service receiving an owned
`StructBuildRequest` and the remaining invocation allowance. The host reserves
declaration identities and checks a private candidate through ordinary
elaboration, then returns `PreparedStructBuild`: shared immutable descriptors,
the result identity, and a final transactional commit callback. The VM checks
the response against the requested name and fields, admits the descriptors in
its live catalog and commits before delivering the Type. It continues the same
invocation without replaying earlier operands. Duplicate names and mismatched
list lengths reject before the service; a missing service gives an error.

Host code cannot replenish the VM allowance. Host unwind retains the reserved
allowance rather than refunding unmeasured work. Descriptor admission and final
commit failures use the existing rollback path. As with the lower-level API,
the host callback must not publish external state and then fail. The compiler
integration test constructs real HIR fields from delivered Types, elaborates a
private package and publishes it only in the final callback; it does not rebuild
source text or substitute structural registration for semantic checking.

This implements source-to-host dispatch in a configured NIR comptime VM.
Ordinary compiler invocation sessions do not yet supply the publisher service;
typealias initialization cannot use it automatically. Defaults, clauses,
attributes, methods, other definition kinds and integration with an active
elaboration session remain open. The SoA example is not yet supported end to end.

Validation in `/home/fin/luna`:

```sh
python3 -B development/check.py --timeout 900 -- test --release -p luna --test source_reflection --no-fail-fast
python3 -B development/check.py --timeout 900 -- test --release -p comptime_interpretor --lib -p nir --test phases --test type_rewrite --test copy_budget --no-fail-fast
```

Final source run: 11 tests, exit 0, `/tmp/luna-tools-pi3u_crd/result.json`.
VM/NIR run: 198 tests, exit 0, `/tmp/luna-tools-heqe0t7l/result.json`.
Coverage includes ordered single evaluation, real HIR semantic publication,
immediate reflection in the same invocation, malformed requests, missing host,
host rejection, mismatched responses, commit failure and same-VM retry. Early
return bypasses dispatch; zero host budget prevents commit; attempted budget
replenishment is clamped; host unwind retains the reservation. NIR tests cover
runtime phase rejection, type rewriting, traversal and bounded copying.
Targeted rustfmt and `git diff --check` passed.

## Source struct reflection

`meta.struct_field_types(Type)` and `meta.struct_field_names(Type)` expose the
existing checked struct observation to source comptime functions as List<Type>
and List<Symbol>. Privileged primitive calls are confined to the meta wrappers;
ordinary consumers use the public package APIs. NIR retains explicit unary
struct-field-types/struct-field-names nodes with S-expression dumps, metered
copy/traversal, type rewriting and comptime phase checks. VM evaluation calls
reflect_struct and validates the delivered immutable List domain.

Applied generic field types and declaration order are preserved. Results can
feed ordinary list transformations and variadic normal-form inputs; no struct
layout or field access permission is implied. A names-only query currently
performs the full checked field observation too. This adds source reflection,
not source builder publication or resumption of a live invocation.

## Installing completed structs in an existing VM

`Interpreter::install_generated_structs` admits a batch of immutable, concrete
NIR struct descriptors after ordinary checked publication. The same NIR comptime
VM can then reflect or admit values of those identities; it retains its previous
functions, storage, events, budget and cached instances. Old declarations cannot
be replaced, and the batch cannot reuse known callable, trait/member, impl,
normal-form, alias, effect or clause-parameter identities. Global identity
reservation remains the publisher's responsibility.

The batch checks field names, closed Type descriptions and formation predicates
in a combined candidate table. Forward references within the batch are allowed;
pointer-recursive field observation still does not compute layout. A borrowed
batch journal records inserted identities without cloning the existing catalog.
Insertion and possible rollback are charged before mutation; descriptor, field
and contract preflight and subsequent admission use the same invocation budget.
Rollback removes only the new entries, even when admission exhausted that budget.
Per-installation work no longer includes copying all previously generated structs;
ordered-map lookups and formation checking retain their own costs. An error or
host-solver unwind restores the old table, including when a later descriptor
fails. Work and arbitrary external solver effects are not rolled
back. Empty structs still check their formation requirements.

This entrypoint accepts concrete builder results with no declaration parameters.
It rejects runtime/IIR interpreters and active abstract normalization scopes.
The descriptors must outlive the VM. It does not add functions, defaults or impls,
change trait selection, authorize field access or replace the HIR publisher's
semantic/resource checks. Existing external type/Copy callbacks remain attached;
the host remains responsible for providing evidence for newly generated types.
Source builder dispatch during a running expression is still unimplemented.
This closes the VM catalog-installation step, not the whole builder pipeline.

`install_owned_generated_structs_with` additionally accepts an
`Arc<[nir::program::Struct]>` created after VM construction. Catalog entries hold
shared batch handles and indexes, so publishing does not copy field trees and
the host may release its temporary storage immediately. Existing package
descriptors retain the borrowed representation. Both forms use the same
preflight, admission, budget reservation and rollback journal. Failed admission,
callback errors and callback unwind release the VM's new ownership; committed
batches remain available until the VM is dropped. A batch's descriptors share
one allocation and are retained together. This removes the descriptor-lifetime
obstacle for a future running-invocation builder service without introducing
source builder dispatch or changing type identity.

Owned-storage validation in `/home/fin/luna`:

```sh
python3 -B development/check.py --timeout 900 -- test --release -p comptime_interpretor --lib --no-fail-fast
python3 -B development/check.py --timeout 900 -- test --release -p luna --test source_reflection -p elaboration --test delivered_types --no-fail-fast
```

Both exited 0. The VM suite passed 141 tests at
`/tmp/luna-tools-_9yu9x_2/result.json`; source reflection and semantic delivery
passed at `/tmp/luna-tools-grc08vqs/result.json`. New cases verify descriptors
outlive temporary publisher storage, forward references and subsequent batches,
release after VM destruction, and release after admission failure, callback
rejection or unwind followed by retry. An existing source comptime function
reflects a dynamically installed type after publisher storage is dropped.
Targeted rustfmt and `git diff --check` passed.

## Coordinating HIR publication and VM admission

`DefinitionBatch::prepare_checked` checks a private candidate once and returns a
`PreparedPublication` holding exclusive access to the original draft. Dropping
the preparation leaves the draft and published package unchanged. Its `commit`
installs the candidate without further checking or user callbacks. Existing
`publish_checked` delegates through this preparation. The elaboration adapter
`prepare_with_invocation_budget` uses ordinary semantic checking and retains
spent invocation fuel on rejection.

The checked output can move into stable host storage with `map_checked(Arc::new)`.
The caller retains that storage for the VM's lifetime, then coordinates both
tables as follows (Rust host integration):

```rust
let prepared = prepared.map_checked(Arc::new);
let checked = Arc::clone(prepared.checked());
let (owners, committed) = vm.install_generated_structs_with(
    &checked.structs,
    || Ok::<_, std::convert::Infallible>(prepared.commit()),
)?;
```

Pass only newly generated concrete descriptors when the candidate also contains
previously published structs. Admission failure never calls the final callback.
A callback error or unwind removes the tentative VM entries. The callback must
itself leave external publication unchanged on failure; the interpreter cannot
undo arbitrary host effects. Use the infallible prepared commit as the final
operation, with no fallible work after it. Neither final commit repeats semantic
checking nor replays the active invocation. This provides the coordinated host
boundary; source builder dispatch and suspension/resumption remain unimplemented.

Validation in `/home/fin/luna`:

```sh
python3 -B development/check.py --timeout 900 -- test --release -p hir --test definitions -p elaboration --test definitions --test delivered_types --no-fail-fast
python3 -B development/check.py --timeout 900 -- test --release -p comptime_interpretor --lib -p luna --test source_reflection --test reflected_definitions --test generated_definitions --no-fail-fast
```

Both exited 0: 30 tests at `/tmp/luna-tools-kjfyr6qo/result.json` and
227 tests at `/tmp/luna-tools-grkadj0n/result.json` (including the luna library
tests selected by `--lib`). Tests cover private preparation, abandonment,
storage-transfer unwind, admission rejection without callback execution,
callback failure/unwind and retry, and coordinated semantic publication without
rechecking. Targeted rustfmt and `git diff --check` also passed.

## Callable type reflection

`Interpreter::reflect_callable` observes a legal callable Type in a comptime
invocation. `CallableReflection` retains both the original subject and the
selected, admitted callable view. The latter retains closure environment Types,
comptime phase and unsafe responsibility. Completed contextual closures may
normalize into ordinary closure descriptions; this does not erase the original
query or authorize a type-identity coercion. Already selected associated Types
can supply the view, but reflection does not search implementations or unfold
normal-form heads.

`signature()` returns the entire function telescope. Parameter and result Types
may reference its earlier binders; extracting a result does not turn it into a
closed standalone Type. Type-sorted parameters alone do not imply comptime
execution. `is_comptime()` and `is_unsafe()` observe the actual callable wrappers.
The S-expression dump retains both descriptions and uses the existing escaped
Term display. No function body, captured value, parameter name, default, access
permission or concrete layout is obtained from this observation.

Selection, formation validation and the retained subject copy consume invocation
fuel. Malformed or unavailable structure, formation rejection, wrong invocation
phase and exhaustion return no view; a later valid request can succeed with the
remaining or explicitly replenished budget. Contextual closure admission now
traverses both the environment description and the signature, including nested
formation predicates, even before normalization converts a completed context.

This is a compiler-side reflection adapter. Source reflection API naming,
definition-builder dispatch and suspension/resumption across declaration-world
publication remain separate work; this API does not implement those features.

Verification from `/home/fin/luna`:

```sh
python3 -B development/check.py --timeout 900 -- test --release -p comptime_interpretor --lib --test function_types --test environment_types --test runtime_function_identities --test normal_values --test type_list_inputs
python3 -B development/command.py --timeout 120 -- cargo fmt --all -- --check
```

All 179 tests passed (125 library tests and 54 integration tests), exit 0,
at `/tmp/luna-tools-1b7fcmip/result.json`. Six new reflection tests cover scoped
dependent signatures, runtime/comptime and unsafe combinations, closure context
normalization, escaped stable dumps, environment/result formation rejection,
invalid structure, wrong phase, budget exhaustion/replenishment, selected
associated views and opaque normal-form preservation. The preceding run at
`/tmp/luna-tools-3vlo2tb5/result.json` exposed an incorrect test expectation that
a completed contextual closure would remain structurally unchanged. The corrected
test requires preservation of the original subject and verifies the normalized
environment/signature separately; normalization semantics were not altered.
Formatting passed at `/tmp/luna-tools-hu0n882i/result.json`; `git diff --check`
passed. These results do not establish source-level builder support.

## Generated node construction

`assemble_function` constructs a function descriptor and its separately stored
body using one `DefinitionNodes` draft. Its callback supplies a complete ordinary
`FnSig` and the body expression, so parameter dependencies, optional/default
expressions, clauses and modifiers remain HIR rather than being flattened into
independent closed Types. This does not infer a signature from callable reflection
or copy implementation details out of a reflected function Type.

The callback reserves parameters and the body root directly under the owner,
leaves those nodes unregistered, and reserves/registers all other nodes using the
shared construction fuel. The assembler derives BodyParams from FnParams, checks
parameter ordering and mirrored identities, registers them and the body root,
then finishes the canonical owner index. Pending reservations, duplicate
registration, invalid parents and exhausted fuel return no assembled function.
The callback remains responsible for registering every child it constructs;
this adapter does not recursively discover omitted reservations or perform name
resolution. Arbitrary callback effects and arena allocations are not rolled back.

`AssembledFunction` returns the definition, BodyId and Body together, with an
escaped S-expression Display. The caller stages both descriptor and body in the
same DefinitionBatch before ordinary checked publication. No body is evaluated
or published by assembly itself. Effect declarations and declaration-only
functions retain their distinct HIR item kinds. Source builder dispatch and live
VM suspension/resumption remain unimplemented.

`assemble_function_declaration` uses the same parameter indexing and identity
checks for a bodyless FnDeclaration. Its callback supplies only FnSig; no BodyId
or package body is created. The parameter nodes still exist in the owner's index
for binding lookup. Assembly does not authorize a free-standing bodyless function:
ordinary publication must find its legitimate trait/template/quote context.
Trait and impl containers continue to use general DefinitionNodes/ItemKind
construction, retaining their real membership lists and normal protocol checking.

Generated protocol verification from `/home/fin/luna`:

```sh
python3 -B development/check.py --timeout 900 -- test --release -p hir -p elaboration --test function_assembly --test generated_functions --test generated_traits
```

All seven tests passed, exit 0, at `/tmp/luna-tools-wc4pn47y/result.json`.
Four HIR tests cover dependent parameter/result references, matching body
bindings and indexes, modifier preservation, escaped dumps, invalid parameter
identities and parents, unfinished reservations, exact budgets, replenishment,
and bodyless declarations without phantom body entries.

The generated-function integration test constructs a function and caller together,
rejects a Char body against the Bool result without publication, repairs the draft,
publishes it, and invokes both omitted-default and explicit-argument calls. It
runs for runtime and comptime functions. The protocol test reserves five owners
(trait, required method, impl, implementation method and caller), checks a real
method-signature rejection, repairs the contract, publishes the batch, and invokes
the caller through trait selection. Declaration references retain explicit source
DefId mappings. A separate test rejects a free-standing ordinary bodyless function.
Generic trait/impl clauses, supertrait composition and capability synthesis remain
on their existing paths and are not established by this static-method coverage.

The preceding run `/tmp/luna-tools-eg_cjbk0/result.json` completed the build after
automatic quota cleanup but lost log bytes (`evidence-error`, wrapper exit 125).
The clean rerun above is the verification evidence for this checkpoint.

`hir::definitions::DefinitionNodes` provides an owner-local identity/index
builder shared by all existing HIR node kinds. Reserve a child under an already
reserved parent, construct arena nodes with those identities, register each once,
then finish with the canonical definition Item. This permits bottom-up node
construction while parent reservation order prevents cycles. Foreign, duplicate,
unreserved and root-node registration is rejected; incomplete drafts and exhausted
budgets remain repairable. Success transfers the index without cloning nodes and
seals the node draft. The caller must reserve/register the children it constructs;
this helper does not recursively discover omitted nodes or perform name resolution.

The resulting OwnerInfo still goes through DefinitionBatch and semantic admission.
Finishing nodes does not publish a type or prove field/default contracts. To revise
an already finished description after semantic rejection, construct a fresh node
draft for the same reserved owner and replace its private batch description.
Previously published definitions remain immutable. This is compiler-side assembly,
not an implementation of source `StructBuilder.new/add_field/build` yet.

`hir::definitions::assemble_struct` accepts a reserved owner, definition name and
ordered `TypedField` inputs (name, stable Type and source span). It allocates the
field/type identities, retains Type data through Arc, and constructs a complete
canonical root/member/type node index. Identity capacity and total assembly fuel
are checked before arena allocation. The returned OwnerInfo still goes through
DefinitionBatch and ordinary semantic admission; duplicate field names and invalid
Types are not accepted merely because assembly succeeded. The current convenience
entrypoint constructs fields without defaults or clauses. Contextual default code,
generic declaration clauses and nested items continue to use the general HIR path.

`assemble_struct_with_defaults` adds a callback for each field in declaration
order. The callback receives its reserved field identity and the shared node
builder/budget, constructs and registers child nodes, and returns an unregistered
default root reserved directly under that field. The assembler validates the
parent and registers the root. Pending reservations, wrong-parent roots, callback
errors and exhausted fuel return no OwnerInfo. Existing batches remain editable;
arena allocation and arbitrary callback side effects are not rolled back. Default
expressions retain ordinary HIR and CMTT checking rather than becoming trusted
pretyped code. The callback is a compiler adapter, not a source-language API or
live interpreter continuation.

The VM-reflection integration test now uses this assembly entrypoint to turn field
descriptions into a checked generated struct, rather than assigning raw local IDs
in the caller. Field-description dumps use escaped S-expressions.

`assemble_inherent_impl` constructs an inherent implementation for a completed
receiver Type with an indexed receiver node and an ordered member-owner list.
Members may refer forward to reservations in the same batch. Existence, bodies,
receiver rules and type formation remain normal publication/elaboration checks.
Budget preflight covers the member list and node construction. Trait references
and generic impl clauses continue to use the general HIR representation.

Container membership and block-local lexical scope are distinct: impl methods
belong in `ImplDef.items`; `DefinitionBatch::define`'s `lexical_parent` is for
declarations inside a block, not a substitute for impl/struct/trait membership.
The generated-method integration test combines struct assembly/defaults, an
inherent impl, an instance method and a consumer in one transaction, then invokes
the compiled consumer through the interpreter.

`assemble_enum` reserves and registers outer variants in declaration order. A
payload callback uses the same `DefinitionNodes` and fuel to build ordinary HIR
unit, positional, named, pattern, expression or grouped variants. Every direct
payload root must be reserved under its variant; unfinished reservations and
exhaustion reject construction. Payload scanning is metered as well as node
construction. Existing HIR S-expression dumps preserve variant kinds and names.
The callback owns payload registration, including nested group members and field
types. This adapter publishes nothing and does not roll back arbitrary callback
side effects. Configuration metadata, generic clauses, nested definitions and
intrinsic enum declarations continue through general HIR assembly.

Ordinary checked publication still decides duplicate-name, payload-Type and
encoding validity. Accepting a pattern/expression payload in the assembler does
not certify its encoding, coverage or layout. The semantic integration test
rejects invalid delivered payload types and duplicate variants without changing
the package, then publishes a repaired enum and reflects its positional/named
payload types and ordered group leaves.

## Implemented: atomic HIR registration

`hir::definitions::DefinitionBatch` reserves owner identities and accepts the
existing `ItemKind` representations. It is not restricted to struct fields:
functions, declarations, structs, enums, traits, implementations, effects,
containers, aliases, and other supported HIR kinds use the same registration path.
`Invalid` is rejected. Function and template definitions must supply their bodies.

The batch holds exclusive access to its target package, but does not alter it
while constructing drafts. Previously published definitions remain available
through a read-only view. A failed structural validation or exhausted validation
budget leaves owner, body, identity, and lexical-parent tables unchanged. Drafts
can be corrected and retried. Dropping a batch does not consume package owner IDs.

Publication checks:

- Every reserved owner has a description; published owners/metadata are not replaced.
- Definition identities are valid and unique, including against the target package.
- Function bodies exist; body roots and formal inputs belong to the right owner.
- Container references exist and form an acyclic graph. Forward references within
  a completed batch are supported.
- Lexical parent links exist and are acyclic.
- Indexed HIR nodes agree with their owner/local identities; their parent links
  exist and reach the owner's root without cycles. An indexed root must refer to
  the canonical definition item.
- Declaration members have distinct, non-root, valid local identities belonging
  to their declaration, even when no node index was supplied. This includes fields,
  nested enum variants and record payload fields, parameters, return bindings,
  clause inputs and constraints. Parameter/type-parameter mirrors must agree.
  Template instance bindings are local; linked declaration signatures, interfaces
  and formal references retain their original context. Explicit instance parameter
  lists and their binding mirrors must agree instead of counting as duplicates.
- Deferred attributes retain their structured expressions, application order,
  configuration keys and lexical context. Their targets must be draft owners
  (or indexed members of those owners); their lexical owners must exist and
  match the expression owner. The complete sequence can be replaced for retry.

Container, lexical-parent, and node-parent traversal reuse validated paths rather
than repeatedly traversing entire ancestor chains. Body replacement uses an ID
index. Tests exercise 1,024 nested owners and 1,024 node parents with a validation
budget proportional to their size.

Nested variants use an explicit iterator stack and charge each member against
the shared publication budget. Member validation does not recursively validate
expression ownership, require all children to be indexed, or establish global
resolver identities for clause inputs. These remain separate assembly/admission
responsibilities; type and field-name checks still belong to elaboration.

Only after every check passes are all table entries installed. No user callback
or fallible semantic check runs during that installation. Draft and diagnostic
Display implementations use stable S-expressions, including escaped names, node
indices, parent relationships, and separately ordered bodies.

The actual AST-lowering provider now uses this path to construct directory-only
module definitions. Scope identities are deduplicated before reservation, matching
the previous sequential-registration behavior. Existing source module linking,
elaboration, and execution remain consumers of ordinary HIR.

## Still required for source-level builders

### Delivered Type inputs

Generated HIR can carry `ExprKind::DeliveredType(Arc<nir::Term>)` directly in a
field annotation or other Type position. This is a compiler-side input node, not
new source syntax. HIR depends on the stable NIR term vocabulary for this staging
edge; the value contains no active inference state, interpreter frame or waiter.
Its S-expression dump includes the original description. The immutable value
remains in HIR after publication, so later candidate elaboration does not depend
on an ephemeral side table or reconstruct source names from normalized types.

Ordinary elaboration charges copying, checks Type sort and closed scope, retains
each observed input by HIR identity, and rejects inconsistent descriptions for
one identity. Before returning checked NIR it admits delivered inputs against the
complete destination declaration world, including nominal/normal-form arguments
and formation predicates. Unknown foreign identities and unauthorized symbolic
inputs fail. Default expressions still use ordinary field type checking. A stable
normal-form description keeps its head; it does not become its body's result.
Checked DefinitionBatch publication therefore retains the original draft/package
on any of these failures. Raw structural publication remains only registration.

This bridges completed Type data from VM reflection to candidate definition
checking. It does not yet resume a suspended invocation, bind generated names,
expose a source builder, or translate captured runtime dependencies across scopes.

### Candidate semantic admission

`elaboration::definitions::publish` now offers a separate checked publication
entrypoint. It constructs an isolated HIR candidate and runs the ordinary package
elaborator with the supplied package identities, language protocols, and target.
Only successful elaboration installs the HIR and returns its checked NIR. Both
structural and semantic errors preserve the original package and the editable
draft. Unwinding from a check also leaves them intact. Diagnostics retain the
distinction between registration failure and elaboration failure.

The checker starts fresh; this is not rollback of an existing checker or arbitrary
comptime side effects. The initial implementation clones package tables (sharing
immutable arena nodes) and checks the complete candidate. It is a correctness
baseline, not the final incremental builder execution design. Structural fuel
now covers a copy preflight before candidate allocation: owner and node slots
(including holes), map capacity, draft tables, pending diagnostic strings and
attribute expression/configuration strings. Arena references and Arc member
candidate slices remain shared and are not charged as deep copies. Preflight
itself is bounded by charging table slots before scanning their owned contents;
failure leaves the original package and drafts intact and never invokes the
semantic callback. These are structural work units, not a byte-exact allocator
quota. Callback/elaborator work retains its own limits; this is not yet one
shared budget for an active VM-to-elaboration session.

`elaboration::definitions::publish_with_invocation_budget` additionally accepts
the caller's remaining required-comptime-invocation steps. It feeds the ordinary
checker through `program::elaborate_packages_with_invocation_budget`, passing the
same allowance to the candidate after function-template expansion and returning the checker's
remaining balance on success, semantic error or unwind. Structural rejection
does not invoke this checker and does not debit that allowance. The default
entrypoints retain their standalone million-step limit. Rechecking an existing
candidate still costs work; a new publication no longer needs to imply a fresh
allowance when the caller selects the shared-budget entrypoint.

Delivered Type world admission now returns its VM's remaining steps on failure
as well as success, including partial preparation-copy work. A failed VM
constructor cannot expose its remainder, so that reserved allowance is consumed
instead of refunded. Other analyses (including PTC and template preparation)
retain their separate limits. This is a reusable invocation-budget boundary,
not an implementation of source builders, incremental rechecking or live VM
suspension/resumption.

`Interpreter::for_comptime_in_budget` centralizes that construction reservation.
It zeroes the caller's available steps before preparation, restores only the
successful constructor's remaining balance, and leaves the caller's depth
configuration unchanged. Phase and declaration/premise failures keep their
original diagnostics. Failed construction consumes the reservation conservatively;
it does not claim exact failed-work accounting. The required-invocation adapter,
type-computation session creation and delivered-Type admission all use this
entrypoint. Adapters reserve their outer counter before passing a temporary
Budget, so unwinding cannot bypass that reservation. The returned VM owns its
own counter: callers still synchronize remaining steps after execution; this API
does not share mutable budget state across concurrently live interpreters.
The existing lowering consumer still uses structural publication, because its
ordinary semantic checking happens later in the driver.

Attribute consumer execution is not implemented. The ordinary elaborator now
checks the attribute table directly, in addition to lowering's pending-semantics
markers. This prevents generated HIR without those markers from silently losing
an attribute or configuration. Checked publication rejects such candidates and
retains their drafts; structural publication alone preserves the metadata for
later checking. This does not add a new accepted attribute API.

`program::attributes::inspect` provides a separate metadata inspection entrypoint.
It runs ordinary declaration/body checking and required comptime evaluation,
using each attribute's lexical owner and structured HIR expression. Prefixes
must return Object; configuration retains its key and typed payload. It returns
materialized NIR descriptions in HIR application order, with target, owner and
source provenance, and a stable S-expression Display. Counts, configuration keys
and materialization share the elaboration budget. Invalid targets, payloads and
unsupported modifiers fail without returning a partial list or modifying HIR.

This entrypoint returns **no executable package**. It is an inspection API, not
an ABI/layout consumer, attribute reflection implementation, or portable closure
serialization. Normal elaboration and checked builder publication continue to
reject deferred attributes until their consumer contracts are implemented.
Generic or runtime-bound metadata is not deferred by this entrypoint; it still
requires a concrete invocation context. Attribute merging is a separate step.

`program::attributes::merge` selects named properties from completed inspection
data independently for each definition/member target. It applies configurations
first, then the already inner-to-outer HIR prefix sequence; duplicate keys inside
one Object keep their last entry. The ordered-entry merge reuses the runtime
Object layer's bounded map insertion and per-target unique-key limit.
Selections borrow both their typed expression and original attribute, retaining
materialization scopes and provenance without copying payloads or closure state.
The selection dump points back to source/entry/lexical owner instead of repeating
the entire source Object for every key. Positional entries are rejected by this
named-property projection, and malformed Object shapes are diagnosed. Failure
exposes no partial map and leaves inspection data untouched. This API does not
execute consumers or change the raw `--emit attributes` dump into a merged map.

`comptime_runtime::object` now provides the value-container layer: `Object<V>`
retains ordered positional and named entries (including duplicates), while
`merge_named_attributes` produces a separate borrowed map from completed values.
It applies configuration first and prefixes inner-to-outer, without cloning
payloads. Entry/key work consumes the caller's shared budget; unique keys have a
separate limit. Failure returns no partially merged map. Positional entries are
preserved by Object but are unsupported by this named-property projection; this
does not assign their general language meaning. Dumps escape keys and use stable
S-expressions.

The interpreter now has `Value::Object`, using this ordered container. Each entry
holds an `ObjectValue` with its original NIR type and payload; metadata never
reconstructs a type from an erased value. Type labels participate in budgeted
copying and admission, and a forged label/payload pair is rejected. Ownership
traversal visits all entry payloads, including shared storage and declaration
references. Immutable-copy accounting covers keys and nested values; embedded
Type descriptions use normal admission. Comptime representation reads preserve
storage identity, explicit representation copies create fresh owned cells, and
owned-graph checkpoints restore cells reachable through Object entries. Runtime
representation copying and effect copying reject Objects. Normal-term freezing
still rejects them until an encoding exists; rejection releases deeply nested
Object payloads iteratively. Constant materialization reconstructs a dedicated
Object expression using each entry's retained type, charging entry/key work and
sharing the ordinary constant snapshot context across nested payloads. Pointers
and free storage remain subject to ordinary constant-escape rejection.

These are VM representation operations, not a source-level Copy promise. NIR now
has a comptime-only Object primitive, available through `.Object'builtin` and
ordinary aliases. Typed Objects can be transported through comptime parameters
and results. Phase checking rejects runtime Object interfaces, including nominal
containers with Object fields. Object still cannot be a frozen normal-form value
input. Source Object expressions now lower to a dedicated NIR constructor and
evaluate in comptime functions. Entries evaluate once in source order; duplicate
keys and positional entries survive construction, and an early control transfer
skips later entries. Type tags remain attached to their values, including Type
payloads and nested Objects. NIR traversal, type rewriting, copy budgets and phase
validation include these entries; runtime construction is rejected. Its freezing
representation and attribute consumer execution remain unimplemented.

`Interpreter::reflect_struct` now provides an internal comptime structure query
for applied nominal structs. It retains the exact subject Type and returns fields
in declaration order with instantiated types. Declaration-world and concrete IIR
inputs use their existing admission/substitution paths; formation conditions and
phantom inputs are checked even for empty structs. Recursive pointer fields need
no target layout. Opaque normal-form heads are preserved in field types and are
not unfolded into structs. The query charges term work, substitution, field counts
and copied names against the invocation budget. Results have stable S-expression
dumps. These descriptions grant no field access, visibility or layout permission.

`Interpreter::reflect_enum` provides the corresponding structural query for an
applied enum: original subject, intrinsic classification, ordered logical variants,
unit/tuple/named payload shapes and substituted payload types, plus fully qualified
group paths. Groups do not consume variant indices. No tag value, bit encoding,
layout or runtime inspection is inferred from the result. Concrete IIR descriptors
are selected once per query rather than once per variant. Metadata copying and
payload substitution share the invocation budget; failed queries deliver no
partial result. Empty enums still check formation predicates and input arity.

Both queries can observe compiler-selected associated Type results, including
associated types used as nominal arguments or field/payload types. Selection uses
the existing complete interface/subject/member key and performs no implementation
search. Query resolution and copying consume the invocation's budget; missing,
wrong-sort and cyclic selections fail explicitly. Returned subjects retain the
original query while fields/payloads use admitted selected descriptions. Opaque
normal-form heads still do not unfold. This does not admit unresolved inference
state or establish a source spelling for reflection APIs.

The source field-only `StructBuilder.new/add_field/Build` API and concrete enum
builders are now available (see the source integration sections above). The
internal reflection query does not execute attribute consumers or reflect field
defaults. Host field-default assembly is connected to transactional HIR
publication, but source default-code transport and generated methods remain
unfinished; the full SoA example still cannot run through these APIs.

The next integration must transport owned, invocation-local definition drafts
from the VM to a candidate elaboration world. Generated names and use-site
permissions still need their normal resolution/lowering paths; the batch can
transport attribute metadata but does not resolve or execute it. The checked entrypoint runs the
ordinary semantic passes, but cannot extend an already running checker or resume
a suspended invocation. Structural registration must never be treated as proof
of semantic properties. Neither published HIR nor frozen
comptime values may contain active inference variables, waiters, or solver
transactions.

Builder-specific assembly must reuse this path rather than making a separate
nominal-type registry for SoA. Generated bodies must keep their CMTT context and
source provenance. Normal-form heads remain opaque; construction of a structural
view does not identify a type-producing function application with its body result.

The first source-level acceptance remains: inspect fields, construct SoA columns
and defaults in a comptime loop, finalize the definition, add methods, and use the
result. Broader definition kinds need equivalent positive/rejection tests.

## Validation

Working directory: `/home/fin/luna`.

```sh
python3 -B development/check.py --timeout 900 -- test --release -p hir -p luna --test definitions --test display --test modules --test packages --no-fail-fast
python3 -B development/command.py --timeout 120 -- cargo fmt --all -- --check
```

The implementation run completed with exit 0: 7 definition-registration tests,
16 Display tests, 56 module tests, and 48 package tests (127 total). Evidence:
`/tmp/luna-tools-y_2y9jer/result.json`, with per-suite summaries in `stdout.log`.
The final formatting check also exited 0:
`/tmp/luna-tools-8mz6eg2k/result.json`.
These checks establish structural registration and the existing module consumer;
they do not establish source-level definition-builder support.

Checked-publication follow-up (same working directory):

```sh
python3 -B development/check.py --timeout 900 -- test --release -p hir -p elaboration --test definitions --test display --no-fail-fast
```

Final-source run exited 0 with 26 tests passing: 1 semantic admission test,
8 registration tests, and 17 Display tests. Evidence:
`/tmp/luna-tools-dfjcamjm/result.json` and its `stdout.log`. The semantic test
rejects a generated function whose return annotation is a Boolean value, repairs
the same draft to use Void, checks the returned NIR's package identity and target,
then rejects another incomplete batch without changing the published package.
Registration tests also check callback rejection, unwinding and exhausted fuel.
Formatting exited 0 at `/tmp/luna-tools-9q6kt_f7/result.json`.

Attribute transport and admission follow-up (same working directory):

```sh
python3 -B development/check.py --timeout 900 -- test --release -p hir -p elaboration -p luna --test definitions --test display --test frontend_cli --no-fail-fast
```

The final run `/tmp/luna-tools-kdmlu6ch/result.json` exited 0: 2 semantic
admission, 9 registration, 17 Display and 204 frontend CLI tests (232 total).
Formatting exited 0 at `/tmp/luna-tools-a41yeld3/result.json`.
The first attempt hit a link/storage quota failure and could not save its final
report (`/tmp/luna-tools-9w4h8boq/`, terminal wrapper exit 125, command exit 101).
Authorized cache cleanup removed about 625 MiB before retry. The intermediate
run `/tmp/luna-tools-vxv5e75i/result.json` exposed three diagnostic-order
regressions, fixed by retaining the existing pending-semantics checks before the
generated-attribute fallback. Four library fixture failures were also repaired:
the fixtures now copy the split Option/Result unwrap modules and the conversion
module required by traits. Their original execution assertions remain intact.

Object value-container checks (same working directory):

```sh
python3 -B development/check.py --timeout 900 -- test --release -p comptime_runtime --test object
```

All 7 tests passed, exit 0; `/tmp/luna-tools-yx4rkvap/result.json` records the
run. Tests cover mixed/nested entries, ordering and duplicate overrides,
non-Clone borrowed payload identity, unique-key limits, every insufficient
budget below a successful merge, positional-entry diagnostics and escaped dumps.

VM Object representation checks (same working directory):

```sh
python3 -B development/check.py --timeout 900 -- check --release -p luna
python3 -B development/check.py --timeout 900 -- test --release -p comptime_interpretor
```

The final luna check exited 0 at `/tmp/luna-tools-90kw8mvu/result.json`.
All 217 interpreter tests passed at `/tmp/luna-tools-vkzf04hk/result.json`,
including seven new Object tests. They verify nested graph traversal and shared
storage deduplication, embedded-type rejection, snapshot restoration, independent
owned copies, runtime and budget rejection without input mutation, escaped dumps,
and iterative disposal of 20,000 nested Objects after unsupported freezing.
Formatting exited 0 at `/tmp/luna-tools-n15wvcu7/result.json`.

Typed Object admission follow-up (same working directory):

```sh
python3 -B development/check.py --timeout 900 -- test --release -p comptime_interpretor -p nir -p luna --lib --test phases --test frontend_cli --no-fail-fast
python3 -B development/check.py --timeout 900 -- test --release -p comptime_interpretor --lib
```

The combined run `/tmp/luna-tools-jpacc7t4/result.json` passed 75 luna unit,
205 frontend CLI, 5 NIR unit and 28 phase tests. Its interpreter suite had one
incorrect dump expectation: the newly retained type correctly prints as
`(primitive bool)`, not `bool`. After correcting that expectation, all 91
interpreter unit tests passed at `/tmp/luna-tools-sb18seao/result.json` (exit 0).
This gives 404 distinct passing tests across those runs. New behavior checks
roundtrip nested typed Objects and opaque normal-form Type payloads through a
comptime function, reject forged entry labels, recover after rejection, and
reject runtime Object storage through direct and nominal interfaces.
Formatting exited 0 at `/tmp/luna-tools-_1187u4v/result.json`.

Source Object construction follow-up (working directory `/home/fin/luna`):

```sh
python3 -B development/check.py --timeout 900 -- test --release -p comptime_interpretor -p luna -p nir --lib --test frontend_cli --test objects --test phases --no-fail-fast
```

Final run `/tmp/luna-tools-loafrfwk/result.json` exited 0: 92 interpreter unit,
75 luna unit, 209 frontend CLI, 5 NIR unit, 2 Object traversal/budget and 28 phase
tests passed (411 total). Source tests cover mixed/nested entries evaluated once
in order, retained duplicate keys, Type payloads in pure comptime code, early
return skipping a recursive later entry, and invalid-name/runtime-call rejection.
Direct VM checks reject malformed constructors and forged payload type labels.
The first focused run hit storage quota and automatically cleaned/retried; it
then exposed invalid test syntax, not an execution failure. Corrected fixtures
use explicit expression positions and reject parse failures in negative tests.
Formatting check exited 0 at `/tmp/luna-tools-x_vn8ujt/result.json`;
`git diff --check` passed.

Object constant materialization follow-up (`/home/fin/luna`):

```sh
python3 -B development/check.py --timeout 900 -- test --release -p luna --test frontend_cli --test comptime_memory --no-fail-fast
```

The final run `/tmp/luna-tools-n3s70076/result.json` exited 0: 212 frontend CLI
and 17 comptime memory tests passed (229 total). Three new tests exercise
module-to-local Object constants with nested opaque normal-form Type payloads,
duplicate keys and positional entries, nested pointer-escape rejection, and
runtime interface rejection. The focused run also passed all three at
`/tmp/luna-tools-edst1xi_/result.json`. Formatting check exited 0 at
`/tmp/luna-tools-41ejwznb/result.json`; `git diff --check` passed.
This enables materializable Object constants, not frozen normal-form Object
arguments, arbitrary storage export, attribute consumers or source builders.

Attribute inspection follow-up (`/home/fin/luna`):

```sh
python3 -B development/check.py --timeout 900 -- test --release -p elaboration -p luna --test definitions --test frontend_cli --no-fail-fast
```

Final run `/tmp/luna-tools-1rt85u4c/result.json` exited 0: 3 definition
admission/inspection tests and 212 frontend CLI tests passed (215 total). The
new inspection test uses structured HIR with deliberately unrelated source text,
checks typed configuration and positional/named Object data, escaped dumps,
rejection of a non-Object prefix, successful retry, invalid member rejection and
unsupported-modifier rejection. Ordinary elaboration still rejects deferred
attributes. This is generated-HIR coverage; source-level inspection invocation
and generic/member binding contexts are not yet end-to-end covered.

The first check exposed a method visibility error, corrected before testing.
The first test build automatically cleaned/retried after a storage quota failure
and then exposed a Vec-versus-slice mistake in the new fixture; that fixture was
corrected before the final run. Formatting check exited 0 at
`/tmp/luna-tools-h3kpl7ce/result.json`; `git diff --check` passed.

Source attribute inspection is available through `luna input.fl --emit attributes`
and the equivalent explicit package request. Frontend/sysroot paths use normal
source protocol binding, and package output can retain its observed input set.
The dump is a typed inspection description, not a portable executable package.
Source tests exposed and fixed a scope distinction: modules retain lexical name
resolution but must not be installed as callable/type signature owners during
required-call preparation. Generic/member binding contexts still require their
concrete invocation context; this entrypoint does not add reflection consumers.

Final verification in `/home/fin/luna`:

```sh
python3 -B development/check.py --timeout 900 -- test --release -p luna --lib --test frontend_cli --test packages --no-fail-fast
```

`/tmp/luna-tools-qd3ut1v2/result.json` exited 0: 75 luna unit, 216 frontend CLI
and 48 package tests passed (339 total). New source coverage verifies comptime
function calls, configuration/prefix application order, lexical constant
shadowing, cross-package function resolution with recorded inputs, continued
ordinary-NIR rejection, and output preservation after inspection failure.
An intermediate package test incorrectly expected literal paths in the existing
hex-encoded input protocol; its assertion now checks source resolution/semantic
identities. Formatting passed at `/tmp/luna-tools-p8d75s7d/result.json`;
`git diff --check` passed.

Typed attribute merge verification (`/home/fin/luna`):

```sh
python3 -B development/check.py --timeout 900 -- test --release -p luna -p elaboration -p comptime_runtime --test frontend_cli --test attribute_merge --test object --no-fail-fast
```

Final run `/tmp/luna-tools-leq2_ws6/result.json` exited 0: 8 Object runtime,
4 typed merge and 217 frontend CLI tests passed (229 total). Coverage includes
configuration-before-prefix ordering even with interleaved inputs, last duplicate
entry selection, target isolation, member identity, source/scoped-origin
retention, escaped selection dumps, non-Clone borrowed values, malformed and
positional entries, unique-key limits, every insufficient budget below success,
and successful retries without input mutation. The source integration test
compiles actual attribute function calls and verifies the outer prefix wins with
its original typed value and source descriptor.

The initial formatting run caught an unclosed delimiter in the new fixture; it
was corrected. The initial test build hit the storage quota and automatically
cleaned/retried, then passed all 12 local tests at
`/tmp/luna-tools-qyj1t2hd/result.json`. Final-source formatting check passed at
`/tmp/luna-tools-7yw9kkl8/result.json`; `git diff --check` passed.
The merge API does not implement `meta.attrs_of`, typed source property access,
attribute consumers or source-level definition builders.

The VM now exposes `lookup_object_property` as a borrowing, comptime-only
internal operation. It distinguishes Missing, TypeMismatch and Found without
choosing the source `Object.get` optional/error policy. The last named entry is
selected; positional entries do not take part in named lookup. Every stored tag
and payload is admitted, including shadowed entries, before returning a result.
Requested type descriptions and key comparisons consume the invocation budget.
The declared entry type is compared directly: no payload-derived type, implicit
conversion, normal-form unfolding or Copy permission is introduced. A returned
borrow does not extend pointer target lifetimes. This does not yet implement
source `get`, Any packing/unpacking, or `meta.attrs_of`.

Validation in `/home/fin/luna`:

```sh
python3 -B development/check.py --timeout 900 -- test --release -p comptime_interpretor --lib
```

`/tmp/luna-tools-g1_34g3v/result.json` exited 0, all 96 unit tests passed. Four
new tests cover missing/mismatch/found distinctions, last-entry selection,
borrowed payload identity, runtime rejection, forged shadowed tags, budget
exhaustion and recovery, opaque type domains and Type payloads, and pointer
storage lifetime. The focused first two tests also passed at
`/tmp/luna-tools-5fp39jjc/result.json`. Formatting check passed at
`/tmp/luna-tools-h3cbwokd/result.json`; `git diff --check` passed.
The documented source example does not specify the get failure channel; that
choice has been asked separately and is not inferred from this internal API.

Generated node-index construction verification (`/home/fin/luna`):

```sh
python3 -B development/check.py --timeout 900 -- test --release -p hir --test definition_nodes --test definitions --test display
python3 -B development/check.py --timeout 900 -- test --release -p elaboration --test definitions
```

Both runs exited 0: `/tmp/luna-tools-jl7efvig/result.json` passed 2 node draft,
9 structural publication and 17 Display tests;
`/tmp/luna-tools-hooz2c3f/result.json` passed 4 semantic publication/inspection
tests (32 distinct tests total). New checks exercise bottom-up registration,
canonical root identity, foreign/unreserved/duplicate/root registration rejection,
incomplete and budget failure recovery, sealing, and generated struct fields with
incorrect then repaired default types. Semantic failure preserves the unpublished
batch and original package; repaired publication retains the indexed FieldDef
and checked Boolean field type. Formatting passed at
`/tmp/luna-tools-fq589rme/result.json`; `git diff --check` passed.

Generated declaration member identity verification (`/home/fin/luna`):

```sh
python3 -B development/check.py --timeout 900 -- test --release -p hir
python3 -B development/check.py --timeout 900 -- test --release -p elaboration --test definitions
python3 -B development/command.py --timeout 120 -- cargo fmt --all -- --check
```

All exited 0. HIR passed 34 tests (3 existing doctests ignored), report
`/tmp/luna-tools-xd3hqo0r/result.json`; semantic publication passed 4 tests,
`/tmp/luna-tools-wk43rtnp/result.json`; formatting passed at
`/tmp/luna-tools-jqpy1j77/result.json`. Before the fix, the new publication
regression accepted a foreign-owned field (`/tmp/luna-tools-fgi56_19/result.json`).
New checks cover foreign/root/duplicate field rejection, draft preservation and
repair, budget exhaustion and retry, parameter mirrors, return-binding collision,
nested enum collisions, field/clause collisions, invalid member IDs, and template
instance binding ownership while allowing external formal/interface references.
`git diff --check` passed. This adds structural admission checks, not the source
StructBuilder API or live invocation-to-elaboration integration.

Internal comptime struct reflection verification (`/home/fin/luna`):

```sh
python3 -B development/check.py --timeout 900 -- test --release -p comptime_interpretor --lib
python3 -B development/command.py --timeout 120 -- cargo fmt --all -- --check
```

The final interpreter run passed 104 tests, exit 0,
`/tmp/luna-tools-2xq9mp3d/result.json`. Eight reflection tests cover substituted
fields in declaration order, recursive pointers without a target layout, escaped
S-expression output, incorrect arity/value inputs and unknown declarations,
runtime phase rejection, opaque heads, phantom inputs/false formation predicates
on empty structs, budget failure/retry and name-copy accounting, concrete IIR
parity, and malformed/unbound field descriptions. Formatting passed at
`/tmp/luna-tools-fgyqtxbb/result.json`; `git diff --check` passed. This is an internal
VM query; no source-level reflection spelling or new attribute/default semantics
are established by these checks.

Internal comptime enum reflection verification (`/home/fin/luna`):

```sh
python3 -B development/check.py --timeout 900 -- test --release -p comptime_interpretor --lib
python3 -B development/command.py --timeout 120 -- cargo fmt --all -- --check
```

Both exited 0. Interpreter tests: 111 passed at
`/tmp/luna-tools-ad801tyw/result.json`; formatting:
`/tmp/luna-tools-goo1_aq7/result.json`. Seven added tests cover group/leaf order,
all three payload shapes and generic substitution, concrete IIR parity, input and
phase rejection with recovery, empty-enum formation and intrinsic classification,
metadata budget failure/retry, opaque payload identity and malformed fields/shapes.
The focused initial five tests also passed at `/tmp/luna-tools-gur9qs5f/result.json`.
`git diff --check` passed. Source-level reflection/builder calls remain unconnected.

Selected associated Type reflection verification (`/home/fin/luna`):

```sh
python3 -B development/check.py --timeout 900 -- test --release -p comptime_interpretor --lib --test type_equality --test nominal_types
python3 -B development/command.py --timeout 120 -- cargo fmt --all -- --check
```

Both exited 0. `/tmp/luna-tools-ubstejkb/result.json` passed 115 interpreter unit,
23 nominal-type and 3 type-equality tests (141 total). The focused 19 reflection
tests also passed at `/tmp/luna-tools-c7fonrn_/result.json`. Added checks cover
selected struct/enum subjects, associated nominal arguments, associated field
types, original subject retention, missing/wrong-sort/cyclic selection failure,
repair and shared-budget exhaustion. Formatting passed at
`/tmp/luna-tools-hzlh4xgt/result.json`; `git diff --check` passed. Tests exercise
already-selected VM evidence; they do not add source-level implementation search.

Delivered Type input verification (`/home/fin/luna`):

```sh
python3 -B development/check.py --timeout 900 -- check --release -p luna
python3 -B development/check.py --timeout 900 -- test --release -p elaboration --test delivered_types --test definitions
python3 -B development/check.py --timeout 900 -- test --release -p hir
python3 -B development/command.py --timeout 120 -- cargo fmt --all -- --check
```

All exited 0. The compiler check passed at `/tmp/luna-tools-bn97_3c1/result.json`.
Elaboration passed 9 tests at `/tmp/luna-tools-4qc_hsav/result.json`; HIR passed
35 tests (3 existing doctests ignored) at `/tmp/luna-tools-vh44ps4z/result.json`.
The 5 new semantic tests cover reflected VM Type data becoming a generated field,
ordinary default checking and retry, later-batch re-elaboration, non-Type/hole/
unauthorized-rigid/unknown-nominal/unknown-head rejection, opaque head retention,
and a false destination formation predicate rejected before publication then
repaired. The new dump test covers stable descriptions and escaped field names.
Formatting passed at `/tmp/luna-tools-t_4snacv/result.json`; `git diff --check`
passed. This is a compiler-side value-to-declaration bridge, not source StructBuilder
or a resumed live invocation. HIR now has an explicit dependency on stable NIR terms.

Indexed struct assembly verification (`/home/fin/luna`):

```sh
python3 -B development/check.py --timeout 900 -- test --release -p hir --test struct_assembly --test definition_nodes --test definitions --test display
python3 -B development/check.py --timeout 900 -- test --release -p elaboration --test delivered_types --test definitions
python3 -B development/command.py --timeout 120 -- cargo fmt --all -- --check
```

All exited 0. `/tmp/luna-tools-crbqrqcz/result.json` passed 32 HIR tests;
`/tmp/luna-tools-kv_kd7is/result.json` passed 10 semantic publication tests (42 total).
New checks cover canonical root/field/type indexing, parent relationships, shared
Type ownership, empty assembly, budget preflight and retry, invalid owners and
escaped descriptor dumps. The reflected-field integration now uses the public
assembler; duplicate names and non-Type descriptions are rejected by ordinary
semantic publication, preserving drafts until repaired. Formatting passed at
`/tmp/luna-tools-mrq3aon_/result.json`; `git diff --check` passed. No source builder
syntax, default-code assembler or invocation suspension/resumption is added.

Generated default-code assembly verification (`/home/fin/luna`):

```sh
python3 -B development/check.py --timeout 900 -- test --release -p hir --test struct_assembly --test definition_nodes --test definitions --test display
python3 -B development/check.py --timeout 900 -- test --release -p elaboration --test delivered_types --test definitions
python3 -B development/command.py --timeout 120 -- cargo fmt --all -- --check
```

All exited 0: 34 HIR tests at `/tmp/luna-tools-prci7i44/result.json` and 11 semantic
publication tests at `/tmp/luna-tools-hf_4lgav/result.json` (45 total). New assembly
checks cover callback order, optional defaults, canonical default-root indexing,
wrong parents, unfinished reservations, exhausted fuel and existing-draft retention.
The semantic/execution test assembles a quote/splice field default, rejects a Char
result for a Boolean field without publication, repairs the draft, constructs an
instance with the field omitted, and executes a generated consumer returning true.
Formatting passed at `/tmp/luna-tools-61tux6dv/result.json`; `git diff --check`
passed. This validates compiler-side CMTT default assembly and execution, not the
source builder API or suspension/resumption of an active builder invocation.

Generated inherent implementation verification (`/home/fin/luna`):

```sh
python3 -B development/check.py --timeout 900 -- test --release -p hir --test impl_assembly --test definitions --test display
python3 -B development/check.py --timeout 900 -- test --release -p elaboration --test generated_methods --test delivered_types --test definitions
python3 -B development/command.py --timeout 120 -- cargo fmt --all -- --check
```

All exited 0: 29 HIR tests at `/tmp/luna-tools-kee9k2dp/result.json` and 12
semantic publication tests at `/tmp/luna-tools-eqe9csk6/result.json` (41 total).
Assembly checks cover ordered forward members, canonical receiver indexing,
shared Type ownership, budget preflight, empty impls and invalid owners. The
integration test rejects an incorrectly typed method body without publishing any
of the four owners or their bodies, repairs it, publishes the batch, and executes
the generated instance method twice. Formatting passed at
`/tmp/luna-tools-cwsv66jd/result.json`; `git diff --check` passed. This remains a
compiler-side assembly API; source builders and live invocation resumption are
not implemented by this change.

Generated enum assembly verification (`/home/fin/luna`):

```sh
python3 -B development/check.py --timeout 900 -- test --release -p hir --test enum_assembly --test definitions --test definition_nodes --test display
python3 -B development/check.py --timeout 900 -- test --release -p elaboration --test generated_enums --test generated_methods --test delivered_types --test definitions
python3 -B development/command.py --timeout 120 -- cargo fmt --all -- --check
```

All final commands exited 0: 32 HIR tests at
`/tmp/luna-tools-s_gt4sk1/result.json` and 13 semantic tests at
`/tmp/luna-tools-4wa9ookk/result.json` (45 total). The first semantic build failed
on the new test's VariantShape import; fixing it to nir::program::VariantShape
resolved that fixture error. Tests cover payload parent/index consistency, order,
empty assembly, fuel preflight, unfinished registrations, callback exhaustion,
pattern/expression distinction and escaped dumps. Semantic checks reject a
non-Type payload and duplicate members atomically, then reflect the repaired
enum's positional/named payload types and group leaves. Formatting passed at
`/tmp/luna-tools-72st19a0/result.json`; `git diff --check` passed. Pattern/expression
encoding validity is not claimed by the structural tests. Source builders and
live invocation resumption remain separate work.

Candidate-copy budget verification (`/home/fin/luna`):

```sh
python3 -B development/check.py --timeout 900 -- test --release -p hir
python3 -B development/check.py --timeout 900 -- test --release -p hir --test publication_budget
python3 -B development/check.py --timeout 900 -- test --release -p elaboration --test definitions --test delivered_types --test generated_methods --test generated_enums
python3 -B development/command.py --timeout 120 -- cargo fmt --all -- --check
```

The regression failed before the fix at `/tmp/luna-tools-cajy4155/result.json`:
a sparse published node table escaped the supplied copy budget. After the fix,
the HIR suite passed 47 tests (3 existing doctests ignored) at
`/tmp/luna-tools-fu7fk2_9/result.json`. Two further copy-budget cases were added;
the final 4-case focused suite passed at `/tmp/luna-tools-gtbphqkd/result.json`,
for 49 distinct HIR tests. The semantic suite passed 13 tests at
`/tmp/luna-tools-gj3jq08d/result.json` (62 distinct tests overall). Cases exercise
published sparse node tables, pending text, attribute expressions and keys,
draft attribute text, retained empty-map capacity, retry and shared Arc payloads.
Existing semantic rejection/unwind behavior and generated struct/enum/method
publication remain covered. All final commands exited 0. Formatting passed at
`/tmp/luna-tools-gk8lox6u/result.json`; `git diff --check` passed. This bounds
candidate copying; it neither removes full-candidate elaboration nor unifies the
semantic callback's budget with the caller's VM.

Shared invocation-budget verification (`/home/fin/luna`):

```sh
python3 -B development/check.py --timeout 900 -- test --release -p elaboration --test delivered_types --test definitions --test generated_methods --test generated_enums
python3 -B development/check.py --timeout 900 -- test --release -p elaboration --test invocation_budget
python3 -B development/check.py --timeout 900 -- test --release -p luna --test comptime_memory --test test_templates --test quote_functions
python3 -B development/check.py --timeout 900 -- test --release -p luna --test test_templates
python3 -B development/command.py --timeout 120 -- cargo fmt --all -- --check
```

The definition suite passed 14 tests at `/tmp/luna-tools-20ip7s9h/result.json`.
The required-function invocation test passed at `/tmp/luna-tools-w90hayqm/result.json`
after correcting its DefId import to the HIR re-export. That build hit a disk
quota during archive creation; the existing wrapper cleaned rebuildable outputs,
retried once and completed successfully. Tests cover structural rejection without
invocation debit, repeated publication sharing the remaining allowance, failure
without refund of Type-admission VM work, zero-budget rejection of a required
comptime function, and explicit replenishment/retry with unchanged draft state.

The broader source run (`/tmp/luna-tools-_pm53g29/result.json`) passed 17 comptime
memory and 104 quote tests, but failed one of 81 template tests. A targeted run
(`/tmp/luna-tools-b1_bsxpq/result.json`) identified its stale expectation: a loop
with unconditional break was still required to fail finite-completion checking.
The test now executes that finite loop for ordinary and templated defaults,
including explicit override, and uses a nonterminating loop as the rejection
case. It retains rejection of input mutation, unavailable private bindings and
nominal destructuring without the cleanup evidence supplied by the separate
protocol-bound tests. The corrected targeted case passed at
`/tmp/luna-tools-fjb7z0ku/result.json`; all 81 template tests then passed at
`/tmp/luna-tools-rd606oed/result.json`. No purity implementation or language
contract was relaxed. Overall, 217 distinct tests passed across the final
applicable runs. Formatting passed at `/tmp/luna-tools-4wu_18rz/result.json` and
`git diff --check` passed. Source builder dispatch and live VM resumption remain
unimplemented; PTC and other analysis limits remain separate.

World-construction reservation verification (`/home/fin/luna`):

```sh
python3 -B development/check.py --timeout 900 -- test --release -p elaboration --lib admission_budget_tests
python3 -B development/check.py --timeout 900 -- test --release -p comptime_interpretor -p elaboration --test runtime_function_identities --test invocation_budget --test delivered_types --test definitions
python3 -B development/command.py --timeout 120 -- cargo fmt --all -- --check
```

All final commands exited 0. The session-specific test passed at
`/tmp/luna-tools-ff28ym9s/result.json` (256 unrelated library tests filtered out).
The combined integration run passed 32 tests at
`/tmp/luna-tools-iy8tocse/result.json` (33 total). During that build an archive
hit disk quota; the existing wrapper cleaned rebuildable artifacts and completed
its single automatic retry. New VM tests compare original diagnostic kinds and
source locations, verify phase/catalog/premise failures retain the reservation,
check successful remaining balances and unchanged caller depth, and exercise
explicit replenishment after correction. The session regression verifies that
invalid world premises cannot leave the caller's original budget available.
Existing required invocation and delivered-Type publication tests remain green.
Formatting passed at `/tmp/luna-tools-nwko06qw/result.json`; `git diff --check`
passed. Reservation does not measure every constructor validation operation or
provide a concurrently shared VM counter; source builder dispatch and live-world
replacement remain separate work.

### Configured preludes at the host assembly boundary

`luna::packages::compile_with_definitions_and_preludes` accepts the same explicit
provider IDs used by ordinary multi-package compilation. It resolves source
imports before invoking the definition callback, then publishes and elaborates
the generated batch through the existing path. This closes the previous host
entrypoint's inability to supply meta/build/attrs/verification preludes alongside
builtin. Provider identity comes from configuration, not a package-name heuristic.
The older `compile_with_definitions` API retains its builtin-only configuration
and delegates to the new entrypoint.

The caller supplies the configured default providers, excluding std. Provider
self-exclusion, ordinary wildcard lookup and declaration identity follow the
shared resolver. Duplicate or unknown providers and unresolved source imports
fail before the callback executes. The callback still runs once; this is not a
source builder API or live VM publication/resumption. Prelude configuration,
source packages and callback inputs belong in the caller's build cache key.

Validation in `/home/fin/luna`:

```sh
python3 -B development/check.py --timeout 900 -- test --release -p luna --test generated_definitions --test reflected_definitions --no-fail-fast
```

Exit 0, three integration tests passed; `/tmp/luna-tools-lp2ayuo9/result.json`.
The new test uses independent fixture providers, verifies NIR and IIR execution
of generated definitions calling source functions that rely on both preludes,
checks missing/duplicate/unknown provider rejection before callback execution,
and verifies std names are unavailable until an explicit `use std.prelude.*`.
Existing source-dependent builders, reflection and failure/retry cases also pass.
The fixture providers do not constitute full sysroot acceptance.

A fresh full sysroot smoke invocation still fails during HIR lowering:
`/tmp/luna-tools-k_7yhl0i/result.json`, exit 1, using
`development/reproduce.py --timeout 60 --binary /tmp/luna-dev-002/target/release/luna -- development/examples/library-smoke.fl --sysroot library --interpret run`
(via `python3 -B`). Its unresolved names include panic, String in builtin
formatting protocols, and placeholder std allocation/collection/I/O hooks.
These independent library-contract and implementation gaps remain open; no
placeholder intrinsic or implicit dependency was introduced to hide them.
