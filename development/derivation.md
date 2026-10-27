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

Final HIR integration commands in `/home/fin/luna` (exit 0):

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
