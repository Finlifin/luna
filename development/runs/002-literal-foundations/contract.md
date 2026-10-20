# Literal frontend contract

This bounded milestone advances the active full-language implementation goal. It does not complete elaboration, comptime evaluation, or the whole literal-conversion protocol.

Authority: `docs/basics/literals.md`, `docs/grammar/syntax`, `docs/grammar/index.md`, `docs/compilation/syntax-and-hir.md`. Fixed outcome criteria C11–C14 and process rubric C1–C10 are recorded in `graph.json`; apply the installed graph-engineering-workflow rubric without alteration. Root integrates isolated source-copy deltas, then a fresh grader reruns acceptance; defects route to their file owner, at most two grading rounds.

## Shared interfaces

- Lexer preserves source byte spans and the current TokenKind variants. Parser accepts Int/IntBin/IntOct/IntHex as AST Int in expressions and Real/RealSci as AST Real in expressions and constant patterns. In pattern position radix prefixes retain BitVec*Pattern identity per docs/control-flow/pattern-matching/bitvec.md; their semantic lowering is outside this milestone and diagnoses explicitly.
- HIR `LitKind::Integer(String)` and `LitKind::Float(String)` store the **exact validated numeral spelling**, including radix, underscores, decimal point and exponent. They are syntax literals, not evaluated comptime values. Never parse through i64/f64, round, overflow to infinity or synthesize zero. Dumps use `(integer (spelling "..."))` and `(float (spelling "..."))`. Numeric arithmetic and value canonicalization belong to elaboration/comptime work.
- Single AST Str is a leaf. Concatenated strings use AST StringConcat (MultiChildren) with at least two Str leaf children, spanning the whole group. Only source whitespace (including an empty gap) between tokens permits merging; comments are not whitespace. Decode individual leaves, then concatenate decoded contents. Strings ending in a suffix terminate concatenation before that suffix.
- HIR suffix requests use `ExprKind::LiteralConversion { literal: &'hir Expr<'hir>, suffix: Ident }`, printed as `(literal-conversion (literal ...) (suffix ...))`. No builtin numeric cast, premature name resolution of the suffix, or chosen LiteralInto implementation. Lowering preserves the request for later elaboration. AST uses existing LiteralExtension with literal and Id children. Only the four documented literal kinds can receive a single suffix; a second suffix or suffix on nonliteral is rejected. LiteralExtension in a pattern lowers to a constant expression pattern.
- Strings/chars decode only the documented escapes. `\x{...}` is 1–6 hexadecimal digits and a Unicode scalar; surrogates and values above U+10FFFF diagnose. Do not synthesize a char or silently truncate on invalid input. Alphabetic and space character literals must lex correctly while quote forms and take views retain their tokens.
- Interpolation parsing/elaboration is a later task. Until then `$name` and `${...}` in strings must issue an explicit unsupported-interpolation lowering diagnostic, with no success dump. Escaped `\$` is ordinary text. Bare braces remain ordinary text. Unescaped `$` not beginning an interpolation is preserved.
- Existing basic CLI and HIR display behavior remains the regression baseline. Full-file output must not be published on any lex/parse/lowering failure.

## Work ownership and checks

Exact paths, baselines, isolation, stop conditions and budgets are in `graph.json`. Lexer, HIR/lowering and independent CLI tests run in `/tmp/luna-dev-002/{lex,hir,tests}`; root owns parser and integration. Each snapshot comes from the same manifest, including the previous graph's uncommitted output. Shared manifests/lockfile are root-only; this milestone needs no dependency changes.

Targeted acceptance: `cargo test -p lex -p parse -p hir -p luna --locked --offline --target-dir /tmp/luna-dev-002/target`; `cargo check -p luna --locked --offline` with a task-owned target directory; changed Rust files formatted. Test authors may cover table-driven boundaries generously, without repeatedly running the workspace.

No commit, push, merge, publish, deploy, deletion or external message is planned in this graph. User's standing delegation authorizes implementation choices and graph execution without another design review. Keep prior unrelated changes byte-for-byte intact.
