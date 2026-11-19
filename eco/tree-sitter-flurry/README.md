# tree-sitter-flurry

Tree-sitter grammar and editor queries for the Flurry programming language.

## Overview

`tree-sitter-flurry` implements the Flurry lexical and surface syntax defined in [`docs/basics/lexical.md`](../../docs/basics/lexical.md) and [`docs/grammar/syntax`](../../docs/grammar/syntax), including:

- Line comments (`-- ...`), formatter directives (`--% ...`), and arbitrarily nested block comments (`{- ... -}`).
- Whitespace-sensitive binary operators (`+`, `-`, `*`, `/`, `%`, `<`, `>`) disambiguated from tight generic/normal-form argument brackets (`Array<i32, 2>`, `fn<T>`), unary negation (`-x`), pointer types (`*T`), dereference (`.*`), and closure-qualified types (`%ctx T`).
- Quote, character, and view disambiguation: take-view chains (`value'view`, `.u8'builtin`), character literals (`'a'`, `'\x{1F600}'`), quoted expressions/patterns (`'(...)`, `'<...>`), and macro content blocks (`'{ ... }`).
- Interpolated strings (`"hello ${name}"`) and raw string blocks (`@builtin.str.raw { ... }`).
- Full item declarations (`fn`, `pure comptime fn<...>`, `struct`, `enum`, `union`, `trait`, `assoc`, `impl`, `derive`, `typealias`, `newtype`, `const`, `mod`, `effect`, `effect enum`, `case`, `predicate`, `test`, `use`, `@use`, `define`).
- Verification and type clauses (`where`, `requires`, `ensures`, `decreases`, `outcomes`, `asserts`, `assumes`, `invariant`, `forall`, `exists`, `refines`, CMTT contextual types `< x: T |- R >` and `< P -| x: T >`).
- Expressions and patterns, including structural boundary preservation (`if`, `while`, `for` conditions vs extended `{ ... }` application), BitVec segment patterns (`0b1110(xs: BitVec<4>)`), error/effect/optional propagation and elimination (`?`, `!`, `#`), and `.ref` / `.*` / `.as(...)` / `.dyn(...)` / `.use(...)` / `.await`.

## Directory Layout

- `grammar.js` — Tree-sitter grammar specification.
- `src/scanner.c` — Zero-state C external scanner for nested `{- -}` comments, whitespace-sensitive operators, `'view` / char / quote disambiguation, string interpolation boundaries, and raw blocks.
- `src/parser.c`, `src/grammar.json`, `src/node-types.json`, `src/tree_sitter/` — Generated parser artifacts (`tree-sitter generate`), excluded from git tracking.
- `queries/highlights.scm` — Syntax highlighting captures.
- `queries/locals.scm` — Lexical scope, local definitions, and reference captures.
- `queries/indents.scm` — Indentation queries.
- `queries/folds.scm` — Code folding queries.
- `queries/tags.scm` — Symbol navigation tags.
- `test/corpus/` — Tree-sitter corpus test suite.

## Development Commands

```sh
cd eco/tree-sitter-flurry
tree-sitter generate
tree-sitter test
tree-sitter parse ../../library/builtin/lib.fl
```
