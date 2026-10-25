# Definition builder implementation boundary

The intended feature covers most definition kinds, including the SoA example's
struct construction. This file tracks implementation, not new Flurry syntax.

## Generated node construction

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
does not account for the table copy or replace the elaborator's own limits.
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

This is **not** a working `StructBuilder.new/add_field/build` API. There is no
new runtime/VM builder value or source-language reflection/builder method yet.
The internal reflection query does not execute attribute consumers or reflect
field defaults. The SoA example still cannot run through this feature.

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
