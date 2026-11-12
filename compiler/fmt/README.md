# `fmt` / `flurryfmt`

Official source code formatter for the Flurry programming language.

This crate provides:

- The `fmt` library crate (`format_source`, configuration discovery, `--%` directive scanning,
  line-based diff generation, and post-format AST/comment verification).
- The standalone `flurryfmt` CLI binary (`rustfmt`-style ergonomics).
- The shared CLI driver (`fmt::run_cli`) backing `luna fmt`.

## CLI Usage

### `flurryfmt`

```sh
# Format stdin -> stdout (default when no path arguments are given, or with `-`)
flurryfmt < input.fl > output.fl
cat input.fl | flurryfmt --stdin-filepath src/lib.fl

# Format specific files or recursively format `.fl` files in directories
flurryfmt src/main.fl library/meta
flurryfmt .

# Check formatting without modifying files (prints unified line diffs, exits 1 on diff)
flurryfmt --check src/

# Print only file paths that require formatting
flurryfmt --check -l src/

# Create `.bk` backups before overwriting changed files
flurryfmt --backup src/main.fl

# Override configuration options on the command line
flurryfmt --config max_width=80,reorder_imports=false src/
flurryfmt --config ".max_width 80, .reorder_imports false" src/

# Inspect effective or default configuration in `flurryfmt.fl` syntax
flurryfmt --print-config default
flurryfmt --print-config minimal
flurryfmt --print-config current
```

### `luna fmt`

`luna fmt` accepts the exact same flags as `flurryfmt`, except omitting path arguments formats
the current working directory (`.`) recursively instead of reading `stdin` (pass `-` to read
`stdin`):

```sh
luna fmt
luna fmt --check
luna fmt - < snippet.fl
```

### Exit Codes

| Exit Code | Meaning |
| --- | --- |
| `0` | Formatting succeeded (or `--check` found zero formatting differences). |
| `1` | `--check` / `--emit check` detected formatting differences, or a file had a syntax, I/O, or verification error. Files with errors are never modified on disk. |
| `2` | Invalid CLI options, conflicting flags, or invalid `flurryfmt.fl` / `--config` settings. |

## Configuration (`flurryfmt.fl`)

`flurryfmt` searches upward from each target file's directory (or `--stdin-filepath` / current
directory for `stdin`, unless `--config-path` is given) for `flurryfmt.fl`.

Configuration files use Flurry top-level `.key value` configuration syntax:

```flurry
.max_width 100,
.tab_spaces 4,
.newline_style .lf,
.use_small_heuristics .default,
.reorder_imports true,
.normalize_comments true,
.ignore [
    "generated/**",
    "*.gen.fl",
],
```

| Option | Values | Default | Description |
| --- | --- | --- | --- |
| `.max_width` | positive integer | `100` | Maximum target column width per line. |
| `.tab_spaces` | positive integer | `4` | Indentation width in spaces per nesting level. |
| `.newline_style` | `.lf`, `.crlf`, `.auto` | `.lf` | Output line-ending policy. |
| `.use_small_heuristics` | `.default`, `.off`, `.max` | `.default` | Width sub-budgets for single-line calls, constructors, arrays, chains, `if`/`else`, and `let`/`else`. |
| `.reorder_imports` | `true`, `false` | `true` | Sort contiguous `use` groups and comment-free `use a.{b, c}` multi-path items lexicographically. |
| `.normalize_comments` | `true`, `false` | `true` | Normalize `--text` line comments to `-- text` (leaving `--%` directives and `---` banners intact). |
| `.ignore` | `["glob", ...]` | `[]` | Glob patterns (`*`, `**`, `?`) excluded during directory traversal and `--stdin-filepath` checks. Hidden `.` directories and `target` are skipped automatically. |

## Inline `--%` Directives

Standalone `--%` comment lines control formatting locally without affecting runtime or compiler
semantics (`--%` inside strings or trailing inline comments after code are ignored; `--%.skip` is
normalized to `--% .skip`):

- `--% .skip`: Skip formatting the immediately following AST item, statement, field, variant, or
  expression; aligns line 1 to the enclosing indentation while preserving internal layout verbatim.
- `--% .off` / `--% .on`: Disable formatting across a region verbatim (even if the region contains
  unfinished or custom DSL text). An unclosed `--% .off` automatically ends at the closing `}` of
  its enclosing block or at EOF.
- `--% .preserve`: Preserve user-written line breaks and intra-line column alignment on the next
  node while normalizing its outer block indentation and `--` comment spacing.
- Scoped configuration overrides (`--% .max_width 120`, `--% .reorder_imports false`,
  `--% .normalize_comments false`, `--% .use_small_heuristics .max`): Override configuration from
  that point until the end of the enclosing `{ ... }` block (or file).

Unknown or malformed `--%` directives emit a warning on `stderr` (suppressed by `-q`) and leave
the comment line intact.

## Formatting Rules Summary

- **Indentation & Width**: Spaces only (`tab_spaces = 4`), `max_width = 100`, LF line endings.
- **Modifiers**: Attributes (`^attr`) each occupy their own line above the item in user order;
  declaration modifiers canonicalize to
  `pub`/`private` -> `template` -> `pure` -> `quote` -> `comptime` -> `async` -> `inline` ->
  `unsafe`/`spec`/`verified`/`atomic`.
- **`use` Imports**: Contiguous runs of `use` items (partitioned by visibility and `@use` vs `use`,
  split by blank lines or standalone comments) sort lexicographically; single-line multi-paths
  format compactly without inner brace spaces (`use a.{b, c}`).
- **Semicolons**: Omitted when a formatted item or statement ends in `}`, except when preserving a
  block-final `ExprStatement` (`expr;`) in a value block (which yields `void`) or disambiguating an
  open-ended statement followed by a continuation token (`(`, `[`, `{`, `.`, `'`, `?`, `!`, `#`,
  `match`, `do`, `matches`, or an infix/assignment operator). Bare trailing expressions never gain
  a semicolon.
- **Trailing Commas & Mixed Containers**: Vertical comma rule (none on single-line except 1-element
  tuples `(x,)`, always on multi-line including arms, enum variants, and multi-line `use` braces).
  Mixed `struct`/`enum`/`union`/`mod` bodies preserve declaration order and insert one blank line
  on category transitions (`.cfg`/properties, fields/variants, methods/items).
- **Operators, Postfix Chains & Types**: Binary operators always carry spaces on both sides (required
  lexically for `+`, `-`, `*`, `/`, `%`, `<`, `>`) and wrap in Front style (operator leading the
  continuation line). Postfix `.` chains align under a single indent level without pyramid nesting.
  `?T` and `*T` bind tightly; `!E T`, `#Eff T`, `&r T`, `%c T`, and `%{ ... } fn(...)` separate the
  qualifier from the body type with one space. CMTT contextual types format as `< x: T |- R >`,
  `< |- R >`, `< S -| x: T >`, and `< S -|>`; standalone `< expr >` patterns preserve required
  whitespace around `<` and `>`.
- **Self-Verification**: Every formatted result is re-parsed and checked for comment retention and
  canonical AST equivalence before any file is written.
