//! `anar` — Official HIR-stage Language Server for the Flurry programming language.
//!
//! Inspired by Zig's `zls`, `anar` combines Flurry's lexer, parser, multi-package early
//! name resolver (`resolve`), HIR lowering (`ast_lowering`), and canonical formatter (`fmt`)
//! to deliver responsive IDE features (diagnostics, go-to-definition, find-references, hover,
//! completions, signature help, rename, outline/workspace symbols, semantic tokens, folding,
//! and formatting) without invoking heavy comptime/elaboration passes.

pub mod analysis;
pub mod cli;
pub mod index;
pub mod json;
pub mod position;
pub mod rpc;
pub mod server;

pub use analysis::{LspDiagnostic, PackageSnapshot, WorkspaceAnalyzer};
pub use index::{Occurrence, SymbolIndex, SymbolInfo, SymbolKey};
pub use json::{JsonParseError, JsonValue};
pub use position::{LineIndex, LspPosition, LspRange, path_to_uri, uri_to_path};
pub use rpc::{RequestId, RpcError, RpcMessage};
pub use server::{ANAR_VERSION, LanguageServer};
