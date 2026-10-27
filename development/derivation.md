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
