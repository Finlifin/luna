# Flurry Ecosystem Tools (`eco/`)

This directory hosts editor integrations and syntax tooling for the **Flurry** programming language, working alongside the compiler crates in [`compiler/`](../compiler):

| Component | Path | Description |
|---|---|---|
| **Tree-sitter Grammar** | [`eco/tree-sitter-flurry`](./tree-sitter-flurry) | Complete Tree-sitter grammar, C external scanner (nested `{- ... -}` block comments, raw strings, disambiguated range/variadic dots), and queries (`highlights.scm`, `locals.scm`, `indents.scm`, `folds.scm`, `tags.scm`) for Neovim, Helix, Zed, Emacs, and GitHub. |
| **VS Code Extension** | [`eco/vscode-flurry`](./vscode-flurry) | Zero-dependency Visual Studio Code extension providing TextMate highlighting, language configuration, idiomatic Flurry snippets, and LSP client integration with `luna lsp`. |
| **Language Server (`luna lsp`)** | [`compiler/anar`](../compiler/anar) | Fast, resilient HIR-stage Language Server Protocol (`LSP 3.17`) service inside luna modeled after Zig's `zls`, supporting multi-package + 6-sysroot-package resolution, hover, navigation, references, rename, completion, signature help, symbols, semantic tokens, folding, and formatting. |
| **Source Formatter (`luna fmt`)** | [`compiler/fmt`](../compiler/fmt) | Comment-preserving Wadler-Lindig Flurry code formatter (`luna fmt`, `flurryfmt.fl`, and `--%` directives), shared by CLI workflows and `anar` document formatting. |

## Quick Verification

```bash
# 1. Run the service tests and the single CLI protocol entrypoint
python3 -B development/check.py --timeout 600 -- test -p anar --lib --test lsp_integration -p luna --test cli_lsp

# 2. Run Tree-sitter corpus tests & full library/*.fl parse sweep
cd eco/tree-sitter-flurry && npx tree-sitter test

# 3. Run VS Code extension static & JSON-RPC smoke tests
node eco/vscode-flurry/test/smoke_test.js
```
