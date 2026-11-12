# `anar` — Flurry Language Server

`anar` is the official Language Server Protocol (LSP 3.17) implementation for the Flurry programming language.

Because Flurry's full semantic pipeline involves dependent normal forms, staged comptime evaluation, algebraic effects, and program verification, `anar` follows the architecture of Zig's `zls`: a fast, resilient **syntax + multi-package early resolution + HIR-stage** language server with zero external runtime dependencies.

## Features

- **Live Diagnostics (`textDocument/publishDiagnostics`)** — Real-time lexical, syntax, module/import resolution, and HIR lowering diagnostics across unsaved in-memory buffers (`Incremental` UTF-16 sync) and sibling package files.
- **Sysroot & Multi-File Package Resolution** — Automatically discovers Flurry's six builtin sysroot packages (`builtin`, `meta`, `build`, `attrs`, `verification`, `std`), wires default preludes (`builtin`, `meta`, `build`, `attrs`, `verification`), and resolves dot-separated paths (`std.io.Writer`, `meta.List`, etc.).
- **Go to Definition / Declaration (`textDocument/definition`, `declaration`)** — Jumps to functions, structs, enums, variants, fields, traits, `impl` methods, clause parameters, module imports, and local bindings, retaining last-known-good navigation while editing broken buffers.
- **Find References & Document Highlights (`textDocument/references`, `documentHighlight`)** — Cross-file reference search and read/write local occurrence highlighting.
- **Hover Documentation (`textDocument/hover`)** — Reconstructs clean Flurry signatures from HIR and renders leading `--` / `{- ... -}` documentation comments.
- **Context-Aware Completions (`textDocument/completion`)** —
  - Dot-completions (`recv.`, `Type.`, `std.io.`) for struct fields, inherent/trait `impl` methods, enum variants, module exports, and Flurry postfix operations (`.ref`, `.*`, `.as`, `.dyn`, `.use`, `.await`).
  - Take-view completions (`'bit_vec`, `'tag_name`, `'old`, `'intrinsic`, `'tuple_enumerate`).
  - `--%` formatter directive completions (`--% .skip`, `--% .off`, `--% .on`, `--% .max_width = ...`).
  - Lexical locals, parameters, clauses, imports, preludes, and Flurry keywords.
- **Signature Help (`textDocument/signatureHelp`)** — Active parameter highlighting across function and method calls.
- **Document & Workspace Symbols (`textDocument/documentSymbol`, `workspace/symbol`)** — Hierarchical outline (`mod`, `struct` fields, `enum` variants, `trait` & `impl` members) with lexical fallback on broken buffers.
- **Safe Rename (`textDocument/prepareRename`, `rename`)** — Cross-file identifier rename across packages with keyword/validity checks.
- **Semantic Tokens & Folding (`textDocument/semanticTokens/full`, `foldingRange`)** — Full lexical + HIR-resolved semantic token highlighting and brace/comment folding.
- **Canonical Formatting (`textDocument/formatting`)** — Direct in-process integration with `fmt` (`luna fmt`), honoring `flurryfmt.fl` and `--%` scoped directives.

## CLI Usage

```bash
# Run as an LSP server over stdio (default):
anar --stdio

# Override the `library/` sysroot path explicitly:
anar --stdio --sysroot /path/to/luna/library

# Run a one-shot batch syntax/resolve/HIR check on a package or file:
anar --check path/to/lib.fl
```

## Editor Setup

- **VSCode**: Use the official extension in [`eco/vscode-flurry`](../../eco/vscode-flurry).
- **Neovim (`nvim-lspconfig` / `vim.lsp.start`)**:
  ```lua
  vim.api.nvim_create_autocmd("FileType", {
    pattern = "flurry",
    callback = function(ev)
      vim.lsp.start({
        name = "anar",
        cmd = { "anar", "--stdio" },
        root_dir = vim.fs.root(ev.buf, { "lib.fl", "main.fl", ".git" }),
      })
    end,
  })
  ```
- **Helix (`languages.toml`)**:
  ```toml
  [language-server.anar]
  command = "anar"
  args = ["--stdio"]

  [[language]]
  name = "flurry"
  scope = "source.flurry"
  file-types = ["fl"]
  roots = ["lib.fl", "main.fl"]
  comment-token = "--"
  block-comment-tokens = { start = "{-", end = "-}" }
  language-servers = ["anar"]
  auto-format = true
  ```
