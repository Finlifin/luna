# Derivation implementation

Contract: `docs/traits/derivation.md` and the `deriver_def`/`derive_def` rules in
`docs/grammar/syntax`. A strategy generates an ordinary checked implementation;
its own inputs must not become runtime parameters of the generated methods.

## Parsed representation

`derive fn name (inputs)? for Trait clauses? { members }` has a distinct
`DeriverDef` AST kind. Its five slots retain the name, input list, full trait
expression, clause list and body. Existing function-shaped storage, traversal,
layout validation and S-expression dumping handle this structure, without
classifying it as an ordinary function definition.

The parser accepts no-input and explicit-empty-input strategies, optional and
variadic parameters, associated declarations, method bodies, ordinary statements,
visibility and attribute wrappers. It requires a strategy name, trait target
and block body; a normal function return arrow, bodyless declaration or `=` body
is not accepted for a strategy. Derive applications retain their separate AST
kind, explicit `.use(strategy)` expression and optional comptime wrapper.

`default` is accepted as a member name after dot projection so the documented
`^attrs.derivation.default` path parses. It remains reserved as a standalone
name, including function and strategy declarations. Default strategy registration
itself is not implemented by this parser change.

## HIR declarations and requests

Named strategies are scanned into their own definition scopes with clause and
input metadata. `ItemKind::Deriver` retains parameters, trait expression, clause
parameters/constraints and a comptime-context body. Generated member definitions
and associated declarations remain body items with distinct owners. The strategy
is not represented as an ordinary callable function. HIR definition publication
checks parameter identities and requires its body to exist under the same owner.

Anonymous derive requests have file-qualified declaration identities and clause
scopes. The scanner records `ImplKind::Derivation` only as a declaration-scope
record; the HIR product is `ItemKind::Derive`, never `ItemKind::Impl`. It retains
the requested phase, trait, subject, clauses and optional explicit strategy.
`.use(strategy)` is split at the request boundary rather than treated as a runtime
handler installation. Dumps distinguish `(deriver ...)` and `(derive-request ...)`.

Both forms mark pending `derivation expansion`. NIR compilation rejects that
pending feature explicitly; declarations cannot silently become implementation
evidence before expansion and checking have been implemented.

## Remaining integration

Strategy selection, input elaboration, generated members and implementation
publication remain unfinished. Generated implementations must enter the existing
ownership, coherence, associated-member, signature and phase checks. Structural
Eq/Hash generation additionally needs field projection and code assembly through
the declared reflection/CMTT interfaces. Default strategy attribute evaluation
and registration remain pending; parsing its spelling does not register a strategy.

## Validation

Working directory `/home/fin/luna`; final commands exited 0:

```sh
python3 -B development/check.py --timeout 900 -- test --release -p parse --tests
python3 -B development/check.py --timeout 900 -- test --release -p ast_lowering --lib
python3 -B development/command.py --timeout 120 -- cargo fmt --all -- --check
```

- Parser suite: 92 passed, `/tmp/luna-tools-o_ggbjqi/result.json`.
  Five derivation tests inspect slot separation, list contents, member kinds,
  layout validity and stable dumps, and exercise malformed headers/bodies,
  default attribute paths and reserved-name rejection.
- AST lowering library: 4 passed, `/tmp/luna-tools-9l5zax7p/result.json`.
  This verifies the existing consumer still builds and passes its unit tests;
  it does not verify lowering of new strategies.

## HIR integration checks

The CLI tests inspect real source lowering of strategy parameters, associated
members, captured generation inputs, explicit selection, request phases and
separate subject types. They reject duplicate strategy/input names, parameter
scope leaks and missing names. NIR rejection is checked for the explicit pending
expansion diagnostic rather than accepting a partially generated implementation.
The HIR publication regression rejects a strategy whose body was never supplied,
without changing the published definition count.

Final HIR integration commands in `/home/fin/luna` (tests exit 0; format correction noted below):

```sh
python3 -B development/check.py --timeout 900 -- test --release -p hir -p resolve --tests
python3 -B development/check.py --timeout 900 -- test --release -p luna --test frontend_cli derivation_
python3 -B development/command.py --timeout 120 -- cargo fmt --all -- --check
```

- HIR/resolve: 101 passed, `/tmp/luna-tools-t0mhhv_w/result.json`.
- Derivation CLI: 3 passed, `/tmp/luna-tools-rnhjza0k/result.json`.
- The standalone CLI fixture explicitly spells `.Type'builtin`, since it does
  not import a sysroot prelude. Generation-input captures remain
  `resolved-fn-param` references to the strategy's parameter identity; they are
  not new method-local variables. The assertions reflect that distinction.

The last assertion wording edit required rustfmt line wrapping. The format check
reported by `/tmp/luna-tools-25cdrxxe/result.json` exited 1; the main commit's
format-pass line was premature. A follow-up formats that assertion only and
rechecks formatting. The 104 passing behavioral tests are unaffected; no tests
were rerun solely for whitespace.

## Generated method drafts

The shared HIR rebinder now has a method-draft entrypoint. Each invocation uses
an unoccupied destination owner, clones the signature, contracts and bodies,
and returns the identity map needed for resolver metadata transfer. Explicit
old/new definition mappings redirect references between generated methods;
references outside that mapping retain their original definition identity.
No source package or published definition is mutated by cloning.

A source receiver and body may both use local zero. Mapping that identity to a
new definition root would produce a forbidden parameter identity. The rebinder
instead allocates a fresh nonzero identity and preserves references from both
the signature and body. The regression clones the same method twice and sends
both drafts through DefinitionBatch publication, checking distinct receiver
identities and preserved body references.

Foreign parameter/local captures currently require substitution and are rejected.
Method clause parameters are also rejected until their definition identities can
be instantiated. This is an internal draft operation, not executable derivation:
strategy selection, generation-input interpretation, metadata/attribute transfer,
and normal implementation checking still need integration. Existing derive
requests retain their explicit pending-expansion diagnostic.

Checks in `/home/fin/luna`:

```sh
python3 -B development/check.py --timeout 900 -- test --release -p elaboration --lib member_clone
python3 -B development/command.py --timeout 120 -- cargo fmt --all -- --check
```

- Member clone: 3 passed, exit 0, `/tmp/luna-tools-unj8z151/result.json`.
- Format: exit 0, `/tmp/luna-tools-wyh_9z80/result.json`.
- Initial test compilation exposed a fixture Ident/Symbol mismatch; the first
  executable test run then rejected differing parameter/type-parameter IDs in
  the fixture. Both fixture errors were corrected before the passing run.

The shared-rebinder regression command also passed (18 tests, exit 0):

```sh
python3 -B development/check.py --timeout 900 -- test --release -p luna --test test_templates signature_template
```

Report: `/tmp/luna-tools-ngy1qj9z/result.json`. `git diff --check` passed.

## Transactional resolver evidence

Generated method drafts now carry member projections, deferred static-member
candidates and receiver access candidates. Both container and member identities
inside each candidate are remapped through the generation identity map; external
identities and candidate order are preserved. Missing evidence stays distinct
from an explicitly empty candidate list. Copying candidates consumes the same
rebind fuel as the generated method, and does not mutate source metadata.

DefinitionBatch accepts the complete evidence collection with `set_resolution`.
It validates draft ownership, duplicate keys, invalid definition identities and
collisions with published metadata before making changes. `prepare_checked` and
`publish_checked` expose this evidence in the candidate package before the
ordinary checking callback executes. Rejection, budget exhaustion, or dropping a
prepared candidate leaves published tables unchanged. Successful commit consumes
the evidence with the definitions, bodies and attributes. Publication dumps show
resolution evidence in S-expression form, sorted by use-site identity.

Signature-template expansion now uses the same evidence-copying helper with an
empty definition mapping. This retains existing resolver behavior while making
candidate copying resource-bounded.

This does not yet implement generation-input substitution, staged parameter
identity allocation, attribute cloning, strategy selection or derive execution.
Those remain necessary before the method draft can replace a derive request.

Checks in `/home/fin/luna`:

```sh
python3 -B development/check.py --timeout 900 -- test --release -p hir --test definitions
python3 -B development/check.py --timeout 900 -- test --release -p elaboration --lib member_clone
python3 -B development/command.py --timeout 120 -- cargo fmt --all -- --check
```

- Definition publication: 20 passed, exit 0, `/tmp/luna-tools-_xpiaiax/result.json`.
- Member cloning: 4 passed, exit 0, `/tmp/luna-tools-lzelifas/result.json`.
- Formatting: exit 0, `/tmp/luna-tools-ykjmb3ds/result.json`.
- Initial compilation required sorting HirId by its owner/local components and
  exposing the evidence helper to its parent module; both were fixed before the
  successful runs.

Signature-template regressions also passed (18 tests, exit 0):

```sh
python3 -B development/check.py --timeout 900 -- test --release -p luna --test test_templates signature_template
```

Report: `/tmp/luna-tools-ncpdebmu/result.json`. `git diff --check` passed.

## Generic member binders and attributes

Member cloning now accepts generic method clauses when the caller supplies a
fresh resolver identity for every clause parameter. Type, positional, optional,
variadic, trait-bound and quoted clause forms are rebuilt under the same budget.
References in defaults, domains, return types, contracts and bodies use the
provided identity mapping. Missing, unchanged or duplicate clause identities are
rejected. DefinitionBatch remains responsible for checking identity collisions
against the published package. This supersedes the earlier blanket rejection of
method clauses; it does not substitute the enclosing strategy's input values.

Drafts also carry attributes and a parameter/clause node index. Attribute order,
configuration keys and source text survive copying. A prefix attribute keeps its
original outer lexical expression and owner, as required by source lowering;
a definition-local configuration expression is rebound to the generated owner.
Member-target attributes point to remapped parameter or clause identities. The
returned nodes must be installed in OwnerInfo and the returned attributes passed
to DefinitionBatch::set_attributes before candidate checking. Unknown member
attribute targets are rejected instead of being silently discarded.

Staged function parameter identities are still unsupported in this draft path
and now explicitly rejected, rather than losing their source identity metadata.
General generation-input substitution, allocation/publication of staged parameter
identities and the complete derive expansion remain outstanding. Prefix
attributes capturing strategy inputs still require the enclosing generation
context; this change preserves that context rather than evaluating the inputs.

Checks in `/home/fin/luna`:

```sh
python3 -B development/check.py --timeout 900 -- test --release -p elaboration --lib member_clone
python3 -B development/command.py --timeout 120 -- cargo fmt --all -- --check
```

- Member cloning: 6 passed, exit 0, `/tmp/luna-tools-py14a1up/result.json`.
- Formatting: exit 0, `/tmp/luna-tools-mkzkm2hz/result.json`.
- The new tests publish two independent generic methods and check dependent
  defaults/return references. Attribute publication checks prefix versus local
  lookup contexts, source ordering, configuration keys and remapped parameter
  targets while leaving source metadata unchanged.

Signature-template regressions passed as well (18 tests, exit 0):

```sh
python3 -B development/check.py --timeout 900 -- test --release -p luna --test test_templates signature_template
```

Report: `/tmp/luna-tools-yspdak7y/result.json`. `git diff --check` passed.

## Staged parameter identity publication

DefinitionBatch now accepts generated comptime parameter registrations through
`set_staged_parameters`. Targets must name actual comptime parameters in the
draft. Registrations participate in the same identity set as definitions and
clause parameters, including staged identities already present in source HIR.
Duplicate bindings, invalid identities, reused identities and runtime/nonparameter
targets are rejected before mutation. Registering an identity advances the draft
allocation frontier; committing also advances the package frontier.

Prepared candidates expose the table to ordinary checking. Failed checks,
exhausted fuel and abandoned preparations leave the published table unchanged.
The registrations are consumed on successful publication and rendered in stable
S-expression order. Copying the draft and scanning existing staged identities
are budgeted; the existing public lowering table is scanned once per publication
rather than assuming that its identities are free.

Generated method drafts now remap each comptime parameter's source identity to an
explicit fresh identity and return the registrations with the signature/body.
Callers must pass the complete registration list to the definition batch before
checking. Missing source identities or fresh mappings remain errors. This
supersedes the earlier rejection of generated methods with staged parameters.
It does not yet substitute captured strategy inputs or wire derive execution.

Checks in `/home/fin/luna`:

```sh
python3 -B development/check.py --timeout 900 -- test --release -p hir --tests
python3 -B development/check.py --timeout 900 -- test --release -p elaboration --lib member_clone
python3 -B development/command.py --timeout 120 -- cargo fmt --all -- --check
```

- HIR suites: 74 passed, exit 0, `/tmp/luna-tools-ju1hm0jk/result.json`.
- Member cloning: 7 passed, exit 0, `/tmp/luna-tools-53mabdfp/result.json`.
- Formatting: exit 0, `/tmp/luna-tools-34nf9avz/result.json`.
- Publication tests cover failed checking, discarded preparations, exhaustion,
  allocation after publication, malformed targets and identity conflicts. The
  member regression publishes two independently rebound staged parameter sets.

The first end-to-end builder run found a regression in module registration:
6 builder-invocation tests exhausted publication fuel because providers.rs still
budgeted only the module entries. The entrypoint now also budgets the existing
staged identity scan; empty publication batches skip the scan. The failure report
is `/tmp/luna-tools-tkb9orgg/result.json` (46 passed, 6 failed; the generated
suite was not reached). This was a new integration regression, not a baseline
failure. Final format check after the fix passed:
`/tmp/luna-tools-hy683eqs/result.json`.

After the budget fix, both end-to-end suites passed (52 builder invocations and
2 generated-definition tests, exit 0), `/tmp/luna-tools-zxv2jxtc/result.json`:

```sh
python3 -B development/check.py --timeout 900 -- test --release -p luna --test generated_definitions --test builder_invocations
```

The final HIR rerun also passed (74 tests, exit 0),
`/tmp/luna-tools-57szjqrg/result.json`. `git diff --check` passed.

## Executable explicit declaration strategies

NIR elaboration now expands explicit `.use(strategy)` requests whose strategies
contain method declarations/definitions only. Each request reserves a fresh impl,
method, generic-clause and staged-parameter identity set, clones the methods and
publishes them into an isolated HIR candidate. The candidate then re-enters the
ordinary elaboration pipeline: method signatures and bodies, ownership, overlap,
contracts and implementation phases use the existing checks.

ImplDef retains a `generation_trait` expression (also in its S-expression dump).
Interface checking forms both the requested and strategy interface instances in
the generated receiver context and compares their complete identities/arguments.
This does not unfold normal-form heads. Generated definition identities belong to
the derive request's package, never the strategy provider's package. A public
elaboration regression admits an external strategy for a local subject but
rejects an external trait plus external subject with NoLocalIdentity.

Source strategies, their copied members and consumed requests are omitted from
the candidate so template methods are not checked as independent ordinary
functions. The source package stays intact. Omission preserves owner/definition
allocation frontiers, updates module/container edges and root node indices, and
removes retired bodies and resolver metadata. Expansion budgets cover source
scanning, member copying, publication and container reconstruction. Allocation
and budget failures discard the private candidate.

Current limits are explicit diagnostics: default/computed strategy selection,
strategy parameters/clauses/contracts, request clauses/contracts, executable
strategy statements, associated-member construction, nested generation,
strategy/request attributes, captured attribute environments and independently
checking unused strategies. Generated methods may have their own generic and
staged parameters. Completing general strategies still requires executing their
inputs and body and substituting captured values; this slice does not stand in
for that work.

The first source test run executed the two-subject method example successfully,
but the interface-mismatch fixture failed in lowering because it omitted the
ordinary u32 alias. The fixture now declares the alias explicitly, and rejected
method/overlap cases assert the specific ordinary impl diagnostic. Before the
final ownership adjustment, six source tests and 29 phase regressions passed:
`/tmp/luna-tools-fsfa7rzw/result.json`. The ownership integration and four existing
generated-trait tests passed after that adjustment:
`/tmp/luna-tools-_j1r7_s8/result.json`.

Commands (cwd `/home/fin/luna`):

```sh
python3 -B development/check.py --timeout 900 -- test --release -p luna --test derivation --test trait_phases
python3 -B development/check.py --timeout 900 -- test --release -p elaboration --test derivation --test generated_traits
```

Final checks in `/home/fin/luna` (all exit 0):

```sh
python3 -B development/check.py --timeout 900 -- test --release -p luna --test derivation
python3 -B development/check.py --timeout 900 -- test --release -p hir --tests
python3 -B development/check.py --timeout 900 -- test --release -p luna --test frontend_cli derivation_
python3 -B development/command.py --timeout 120 -- cargo fmt --all -- --check
```

- Derive source execution/rejections: 9 passed,
  `/tmp/luna-tools-v75tn50u/result.json`.
- HIR: 75 passed, `/tmp/luna-tools-bidbezuy/result.json`.
- Existing derivation CLI: 3 passed, `/tmp/luna-tools-w9aablxw/result.json`.
- Formatting: `/tmp/luna-tools-icsgpse_/result.json`; `git diff --check` passed.
- The final source suite executes runtime and comptime generated methods,
  independent subject instances and generic methods. It rejects mismatched full
  trait instances, normal-form head erasure, missing/incompatible methods and
  overlapping requests through the expected ordinary checks.
- Block-local requests are explicitly rejected until their enclosing Item
  references can be rewritten. Prefix method attributes whose lexical owner is
  the retired strategy also remain unsupported, even when their expression is
  otherwise constant. No strategy input/statement execution is claimed here.

## Associated declarations in explicit strategies

Declaration-only strategies may now supply associated types and values alongside
methods. All member identities are reserved before copying, so generated methods
and associated initializers can refer forward to other generated members. The
clone preserves an AssociatedDecl descriptor, its parameters, clauses,
constraints, type and optional initializer; it does not synthesize a function
body for an associated member. Parameters/clauses share the existing identity,
attribute and resolver-evidence machinery with methods.

These definitions enter ordinary associated contract, default and phase checking.
The source tests put the method before its associated declarations and compute an
associated Bool from `Self == First`, proving that two uses of one strategy keep
independent receiver contexts. Another execution test returns `Item = Self` into
differently typed consumers. Comptime contributions and incompatible/missing
associated contracts are checked through the ordinary paths.

This supersedes the associated-declaration restriction of the initial explicit
strategy slice. Executing arbitrary strategy statements, constructing members
from computed inputs and default strategy selection remain unfinished. Cloning
preserves associated telescopes; this does not add independent execution support
for associated forms that the ordinary checker cannot yet handle.

Checks in `/home/fin/luna` (all exit 0):

```sh
python3 -B development/check.py --timeout 900 -- test --release -p elaboration --lib member_clone
python3 -B development/check.py --timeout 900 -- test --release -p luna --test derivation --test trait_phases
python3 -B development/check.py --timeout 900 -- test --release -p luna --test derivation
python3 -B development/command.py --timeout 120 -- cargo fmt --all -- --check
```

- Member clone: 8 passed, `/tmp/luna-tools-cm7znmbl/result.json`.
- Initial source/phase run: 12 + 29 passed,
  `/tmp/luna-tools-zp6o5v9n/result.json`.
- Final source run including distinct associated Self types: 13 passed,
  `/tmp/luna-tools-v5gtx7qh/result.json`.
- Formatting: `/tmp/luna-tools-dco3uinn/result.json`; `git diff --check` passed.

## Prefix attributes on generated members

Member prefix expressions whose lexical owner is the strategy now move into
the generated impl context. They share the impl receiver/trait expression ID
mapping, preserve resolver-selected declaration identities, and obtain the
generated receiver's `Self`. The generated method's own parameters remain outside
this surrounding scope. Definition-local configurations retain their existing
member context, and prefix application order is unchanged.

Attribute expressions and any nested bodies are copied into the private candidate
before publication. The ordinary attribute evaluator and consumer checks remain
responsible for the result: preserving an ABI attribute does not implement its
consumer. Free generation inputs still require explicit value substitution;
strategy input execution and attributes on strategies/requests remain unsupported.

The first execution run exposed an ordinary checker gap: impl members had a
`Self` container, but their surrounding impl scope did not. Recognizing an impl
owner directly in `self_type` lets handwritten and generated member prefix
attributes interpret the receiver lazily through the same path. The initial derive run passed 15 of
16 tests and failed the new `Self` case (`/tmp/luna-tools-s_9bqxyy/result.json`);
the earlier build also caught and corrected the relocation helper's visibility.
An intermediate attempt to add a self-container edge from an impl to itself
caused five capture-depth failures (`/tmp/luna-tools-wz_2itdr/result.json`). The
final approach adds no such edge and preserves the acyclic capture hierarchy.

Final checks in `/home/fin/luna` (exit 0):

```sh
python3 -B development/check.py --timeout 900 -- test --release -p luna --test derivation --test trait_phases
python3 -B development/command.py --timeout 120 -- cargo fmt --all -- --check
git diff --check
```

- Derivation: 16 passed; trait phases: 30 passed,
  `/tmp/luna-tools-2axf4n0r/result.json`.
- Formatting: `/tmp/luna-tools-em8q313t/result.json`.

## Generic derive requests

Explicit requests now transfer their clause telescope and constraints into the
fresh generated implementation. Each request binder receives a reserved
implementation-package identity before cloning the subject, requested interface,
parameter domains/defaults and constraints. The generated owner indexes the cloned
clause nodes; no request binder is retained by identity after source retirement.
The same budgeted constraint copier serves generated members and impl headers.

This enables a declaration-only strategy to generate implementations for generic
subjects, with ordinary implementation applicability controlling their use. It
does not instantiate parameterized strategies or infer strategy inputs. The
strategy's declared full interface still has to match the requested instance;
normal-form identity and ordinary coherence checks remain in force.

Source coverage executes independently generated generic subjects, Bool-valued
subject parameters and comptime implementation contributions. Both trait-bound
binder syntax and explicit `requires` accept a satisfying subject and reject an
unsatisfied one; a mismatched scalar-indexed subject remains a type error.
The initial source run passed 18/19 cases; its remaining fixture omitted the
required comma before `requires` (`/tmp/luna-tools-j9le1q_9/result.json`). After
correcting that fixture and adding comptime coverage, all 20 passed
(`/tmp/luna-tools-bqohtl9v/result.json`).

Check commands, working directory `/home/fin/luna`:

```sh
python3 -B development/check.py --timeout 900 -- test --release -p luna --test derivation
python3 -B development/check.py --timeout 900 -- test --release -p elaboration --lib member_clone
python3 -B development/command.py --timeout 120 -- cargo fmt --all -- --check
git diff --check
```

All final checks exited 0. Member cloning: 8 passed,
`/tmp/luna-tools-u8drqxes/result.json`. Formatting:
`/tmp/luna-tools-t05kw8un/result.json`; `git diff --check` passed.

## Derivation diagnostic sources

Elaboration errors now carry structured secondary source notes. Their Display
keeps the primary range and message and prints notes separately. Both expansion
failures and ordinary checking failures of generated candidates can point back
to the selected strategy and its derive use sites. When the primary location
falls inside a source member, that member's declaration is included as well.

The current checker returns source ranges rather than a failing generated owner.
If one strategy is used repeatedly, notes therefore list its related use sites;
they do not claim that one arbitrarily chosen instance caused the error. Errors
outside a request or strategy range receive no inferred derivation attribution.
The scan is bounded and ordinary error flags are preserved. Exact attribution
through errors in external callees still needs richer checker provenance.

Checks in `/home/fin/luna`:

```sh
python3 -B development/check.py --timeout 900 -- test --release -p luna --test derivation
python3 -B development/command.py --timeout 120 -- cargo fmt --all -- --check
git diff --check
```

- Final source suite: 22 passed, exit 0,
  `/tmp/luna-tools-oql1k2z6/result.json`.
- The first run passed 21/22; its location assertion assumed a token-exact
  declaration start, whereas HIR spans include preceding separators. The final
  assertion checks that each note covers its corresponding declaration. Initial
  report: `/tmp/luna-tools-9thanjh6/result.json`.
- Formatting: `/tmp/luna-tools-t_lqgmp9/result.json`; diff check passed.

## Receiver expressions in generated methods

The shared generated-expression copier now preserves `SelfValue` while assigning
its fresh expression owner. Ordinary method checking resolves that owner to the
cloned receiver parameter, including borrowed receivers. It does not capture the
source strategy's parameter environment or fabricate a receiver for static
methods. This closes a gap where receiver signatures cloned successfully but any
body use of lowercase `self` reached the copier's unsupported-expression case.

Checks in `/home/fin/luna` (all exit 0):

```sh
python3 -B development/check.py --timeout 900 -- test --release -p luna --test derivation
python3 -B development/command.py --timeout 120 -- cargo fmt --all -- --check
git diff --check
```

- Source suite: 26 passed, `/tmp/luna-tools-m5_9omfp/result.json`.
  Receiver cases read independent subjects, write the original borrowed subject,
  return a receiver during comptime invocation, and reject self in static methods.
- Formatting: `/tmp/luna-tools-2z65seio/result.json`; diff check passed.

## Closed positional strategy inputs

An explicit selection may now carry an argument list, such as
`derive Flag.use(configured(true)) for Subject`. Each expansion reserves a private
input-group constant and a typed constant for each strategy parameter. The input
group checks bindings in source argument order before producing its Object;
member references resolve to the per-expansion parameter constants, never to
runtime parameters added to generated methods. Ordinary constant admission also
checks inputs that no generated method uses.

Input type expressions are copied with preceding input bindings available, so
literal contextual types and dependencies such as `(T: Type, value: T)` survive.
Members, associated declarations and member prefix attributes use explicit
capture substitutions. Source generation scopes remain separate from destination
method-local binding identities.

This path covers closed positional inputs. Generic request environments, strategy
clauses/contracts, default/named/variadic and specialized parameter matching still
need the general call-instantiation path. General strategy statements and default
strategy registration remain unfinished. Input values inherit ordinary constant
materialization limits rather than bypassing resource or runtime identity checks.

The first source run passed 27/30 tests and exposed that Object entries cannot
use struct field projection (`/tmp/luna-tools-xk55a4bd/result.json`). The internal
`ConstantObjectEntry` HIR node now performs required Object evaluation, bounds
checking, preserved-type checking and ordinary constant materialization. Its dump
is an S-expression with the input expression and ordinal index.

The next run passed 30/33; three runtime-use fixtures lacked a bound Splice
protocol (`/tmp/luna-tools-v9fzqvdt/result.json`). Corrected positive fixtures bind
an explicit bool Splice, and a separate rejection test preserves that boundary.
The final derive suite passed all 34 tests, including independent values,
contextual numeric literals, preceding Type inputs, associated members, prefix
attributes, complete interface matching and unused-input rejection.

Checks in `/home/fin/luna`:

```sh
python3 -B development/check.py --timeout 900 -- test --release -p luna --test derivation
python3 -B development/check.py --timeout 900 -- test --release -p hir --test display
python3 -B development/check.py --timeout 900 -- test --release -p elaboration --lib member_clone
python3 -B development/command.py --timeout 120 -- cargo fmt --all -- --check
git diff --check
```

- Derive: 34 passed, exit 0, `/tmp/luna-tools-lhj4ar6o/result.json`.
- HIR display: 19 passed, exit 0, `/tmp/luna-tools-fpmr2wrg/result.json`.
- Formatting: exit 0, `/tmp/luna-tools-lx7sdedh/result.json`; diff check passed.
- Member clone: 9 passed, exit 0, `/tmp/luna-tools-21rzc0pz/result.json`.
  Explicit input substitutions retain one receiver parameter, independent
  destination identities and unchanged source captures after budget failure.

## Cached input entry extraction

A completed ordinary constant whose cached expression is a flat Object can now
supply an entry by copying that entry alone. Lookup preserves the entry's original
type, checks the requested type and charges the selected expression's copy cost.
It does not repeatedly interpret or copy unrelated entries of the input group.
The first evaluation still checks and materializes the whole input group.

The optimization only reads an existing ordinary-constant cache entry. Associated
values and generic residuals are not admitted through this path. A reconstruction
call or block surrounding an Object retains its environment and uses the original
required-invocation/materialization path. This is an extraction optimization;
other generation and checking costs are not claimed to be linear in input count.

Checks in `/home/fin/luna`:

```sh
python3 -B development/check.py --timeout 900 -- test --release -p luna --test derivation
python3 -B development/check.py --timeout 900 -- test --release -p elaboration --lib object_entry_tests
python3 -B development/command.py --timeout 120 -- cargo fmt --all -- --check
git diff --check
```

- Derive: 35 passed, exit 0, `/tmp/luna-tools-wdd75ew9/result.json`, including
  65 inputs sharing an Object with a 16 KiB string.
- Formatting: exit 0, `/tmp/luna-tools-us8i7tul/result.json`; diff check passed.
- Entry extraction: 3 passed, exit 0, `/tmp/luna-tools-ltvmjx_j/result.json`.
  With a 32-step copy allowance, a Bool entry succeeds beside a 64 KiB string;
  selecting the string correctly exhausts that allowance. Invalid indices and
  exhausted budgets leave the source reusable, and wrapped snapshots fall back.

## Immutable local computations in strategy bodies

Declaration strategies may now contain initialized `let` or `const` bindings
with a single by-value name. The generation input block evaluates these bindings
in source order after explicit inputs. Generated members, associated declarations
and prefix attributes capture each binding through its own reserved constant;
source-local identities never become runtime method parameters.

A strategy with no explicit inputs still gets an evaluation group when it has
local computations. Explicit local annotations are preserved, and inferred local
values retain the Object entry's checked type. Unused bindings are checked too.
Mutability, destructuring, guards, assignment and general control-flow statements
remain outside this path and are rejected. Generic request environments still
require deferred generation; this step retains ordinary constant materialization
limits and does not complete general strategy execution.

Checks in `/home/fin/luna` (all exit 0):

```sh
python3 -B development/check.py --timeout 900 -- test --release -p luna --test derivation
python3 -B development/command.py --timeout 120 -- cargo fmt --all -- --check
git diff --check
```

- Derive suite: 38 passed, `/tmp/luna-tools-17cds165/result.json`.
  New cases cover ordered local computations, independent requests, zero-input
  strategies, inferred and explicit local types, associated types, attributes,
  unused failures and rejection of mutation.
- Formatting: `/tmp/luna-tools-8u6mo2c3/result.json`; diff check passed.

## Export only demanded generation captures

Cloning records the substituted generation values used by members, interface
heads and relocated attributes. Capture analysis follows dependencies in the
annotations of those values before constructing the evaluation group's output.
Only this transitive set crosses the constant materialization boundary; unused
slots carry unit placeholders to preserve reserved identities and ordinal entry
indices. The source computations still execute in order, with their original
types and comptime checks, even when none of their results are exported.

This permits local references used only to compute a captured result to remain
inside the generation invocation. It does not make references legal persistent
captures or relax ordinary runtime Splice requirements. General strategy control
flow and generic generation environments remain unfinished.

Checks in `/home/fin/luna` (exit 0):

```sh
python3 -B development/check.py --timeout 900 -- test --release -p luna --test derivation
python3 -B development/check.py --timeout 900 -- test --release -p elaboration --lib member_clone
python3 -B development/command.py --timeout 120 -- cargo fmt --all -- --check
git diff --check
```

- Derive suite: 40 passed, `/tmp/luna-tools-ifu09g23/result.json`.
  New cases exercise a temporary reference used to compute an exported Boolean
  and a local type needed only by an exported value's annotation. Existing cases
  check unused invalid inputs/locals, interface heads, attributes and Splice.
- Member cloning: 9 passed, `/tmp/luna-tools-k4dltpl4/result.json`, including
  exact per-expansion capture identities and failed-budget recovery.
- Formatting: `/tmp/luna-tools-dp9jn1sh/result.json`; diff check passed.

## Ordered strategy computation statements

Generation groups now retain computation statements in source order rather than
collecting only immutable local initializers. Initialized single-name `var`
bindings, assignments, calls and supported branch/loop forms enter the existing
comptime expression checker and interpreter. Captured results are materialized
after the generation computation finishes; temporary state stays inside that
invocation. Independent derive requests receive independent state.

A strategy containing computations but no input or local capture still gets an
evaluation group, so unused calls cannot bypass phase checks. Assignment to an
immutable binding uses the ordinary diagnostic. Nested definition emission,
destructuring/guarded outer bindings, early strategy returns and generic request
computation environments remain unsupported; this is execution of computations,
not yet control-flow-dependent publication of generated declarations.

Checks in `/home/fin/luna`:

```sh
python3 -B development/check.py --timeout 900 -- test --release -p luna --test derivation
python3 -B development/command.py --timeout 120 -- cargo fmt --all -- --check
git diff --check
```

- Final derive suite: 45 passed, exit 0,
  `/tmp/luna-tools-wibjw7c3/result.json`. Coverage includes ordered mutations,
  per-request isolation, while loops, computation-only strategies, immutable
  assignment rejection, execution-budget failure for unused call results, and
  rejection of conditional member emission before publication.
- Initial runs exposed fixture errors: `()` as a type, the nonexistent `loop`
  source keyword, a standalone fixture without a `void` binding, and an expected
  diagnostic substring. Those fixtures were corrected, not compiler semantics.
- The initial division-by-zero fixture was rejected by safety verification,
  so it did not establish interpreter execution. The final bounded-loop fixture
  explicitly checks exhaustion of the comptime execution budget instead.
- Formatting: exit 0, `/tmp/luna-tools-k1l4lh29/result.json`; diff check passed.

## Capturing strategy pattern outputs

The generation environment now reserves captures per successful pattern binding,
not per outer let statement. An iterative, fuel-bounded pattern traversal records
binding identities in source order. Original declarations still execute once
through ordinary pattern checking and invocation; this collector does not decide
whether a pattern is exhaustive or which branch matches.

Each component is exported with its independently inferred type. An annotation
on a whole struct/list/tuple pattern is not reused as the component type. A plain
by-value root binding retains its explicit annotation and its capture dependency
closure. Captures use HIR identities rather than names, including across local
shadowing; only demanded outputs cross the materialization boundary.

Outer guards, fallback branches and uninitialized declarations still need
additional generation control-flow support. Existing restrictions on nested
member emission, generic environments and persistent captured values remain.

Checks in `/home/fin/luna`:

```sh
python3 -B development/check.py --timeout 900 -- test --release -p luna --test derivation
python3 -B development/command.py --timeout 120 -- cargo fmt --all -- --check
git diff --check
```

- Derive suite: 49 passed, exit 0, `/tmp/luna-tools-m02iifiq/result.json`.
  New coverage includes annotated nested heterogeneous struct outputs, associated
  values and runtime method captures, mutable destructuring, shadowed names,
  independent requests, wildcard phase checking and unused invalid fields.
- The initial build failed because HirId is not ordered; membership deduplication
  now uses a hash set while output order follows the explicit traversal stack.
- Formatting: exit 0, `/tmp/luna-tools-dt6bptmm/result.json`; diff check passed.
- This suite does not establish end-to-end support for every pattern protocol;
  those continue to require their ordinary protocol bindings and checking.
