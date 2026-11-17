# Flurry for Visual Studio Code (`vscode-flurry`)

Official Visual Studio Code language extension for the **Flurry** programming language (`.fl`), powered by `luna lsp` and its internal [`anar`](../../compiler/anar) service.

## Features

- **Lexical & Semantic Highlighting**:
  - Complete TextMate grammar covering `--` line comments, `--%` formatter directives, nested `{- ... -}` block comments, raw strings (`r#"..."#`), byte/cstr literals, radix numbers (`0x`, `0o`, `0b`), `'label` / `.sym'builtin` suffixes, `%meta` splices, `^attr` prefix attributes, and phase/resource modifiers.
  - Full semantic tokens from `anar` (`namespace`, `struct`, `enum`, `interface`, `typeParameter`, `parameter`, `variable`, `property`, `enumMember`, `function`, `method`, `macro`, `keyword`, `modifier`).
- **Diagnostics**: Live syntax, early-resolution, and HIR lowering diagnostics across multi-file packages and the 6 Flurry sysroot packages (`builtin`, `meta`, `build`, `attrs`, `verification`, `std`).
- **Navigation & Refactoring**:
  - Go to Definition / Go to Declaration (including cross-file modules, `use` path segments, struct fields, enum variants, `impl` methods, and sysroot declarations)
  - Find All References & Document Highlights (read/write classification)
  - Document Outline (`Ctrl+Shift+O`) & Workspace Symbol Search (`Ctrl+T`)
  - Safe Rename (`F2` with `prepareRename` validation)
- **Authoring Assistance**:
  - Hover signatures & extracted `---` / `{- ... -}` documentation comments
  - Context-aware Completion (`.` member/module projection, `'` builtin/label suffix, `%` splice, and lexical/prelude scope bindings)
  - Signature Help (`(`, `,`)
  - Folding Ranges & Idiomatic Flurry Snippets
- **Code Formatting**: Built-in `Format Document` (`Shift+Alt+F`) backed by `compiler/fmt` (`flurryfmt.toml` discovery and `--%` directive support).

## Zero External Dependencies

`vscode-flurry` uses a self-contained Node.js JSON-RPC 2.0 `Content-Length` client (`src/lsp_client.js`) and requires **no `npm install` step** to run or package.

## Development & Local Usage

1. Build `luna`:
   ```bash
   python3 -B development/cargo.py build -p luna
   ```
2. Launch VS Code with this extension loaded:
   ```bash
   code --extensionDevelopmentPath=/path/to/luna/eco/vscode-flurry /path/to/luna
   ```
3. Or symlink/copy `eco/vscode-flurry` into `~/.vscode/extensions/vscode-flurry-0.1.0`.

## Extension Settings

| Setting | Default | Description |
|---|---|---|
| `flurry.anar.enable` | `true` | Enable the `anar` language server. |
| `flurry.anar.path` | `""` | Explicit path to `luna`, launched as `luna lsp --stdio` (auto-discovers workspace builds, the shared dev cache and `PATH` when empty). |
| `flurry.anar.sysroot` | `""` | Explicit path to Flurry's `library/` directory (auto-discovered when empty). |
| `flurry.anar.extraArgs` | `[]` | Additional CLI flags passed to `luna lsp`. |
| `flurry.format.enable` | `true` | Enable document formatting through `anar`. |
| `flurry.trace.server` | `"off"` | Trace JSON-RPC traffic (`"off"`, `"messages"`, `"verbose"`) in the output channel. |

## Verification

```bash
LUNA_BIN=/tmp/luna-dev-002/target/debug/luna node eco/vscode-flurry/test/smoke_test.js
```

Set `LUNA_BIN` to the executable built for the current checkout when validating
the live protocol. An invalid explicit path fails rather than falling back to an
older workspace binary. Without it, the smoke test discovers a binary and skips
the live portion if none exists.

The `flurry.anar.*` setting names are retained for the service configuration.
If `flurry.anar.path` previously pointed to a standalone `anar`, change it to
`luna` or clear it for discovery. No second executable is launched. Navigation
and diagnostics use syntax and HIR evidence, not the full elaboration pipeline.
