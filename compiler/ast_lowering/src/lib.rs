//! # AST Lowering
//!
//! This crate transforms Flurry's flat, index-based **AST** (produced by
//! the parser) into the arena-allocated **HIR** (High-level Intermediate
//! Representation) consumed by type checking and later passes.
//!
//! ## Architecture
//!
//! ```text
//!   AST  (flat arrays, NodeKind + children indices)
//!    │
//!    ▼  ast_lowering::lower_to_hir()
//!   HIR  (arena-allocated, owner-based, &'hir references)
//! ```
//!
//! The lowering is driven by [`LoweringContext`], which holds references to
//! the AST, the HIR arena, the source map, and the diagnostic context.
//!
//! ## Key design points
//!
//! 1. **Clause declarations as generic parameters** — In Flurry, clause
//!    declarations (`<T, U : Show, V :- Iterator>`) serve as generic
//!    parameters. During lowering they are split into [`ClauseParam`]s
//!    (the type parameter declarations) and [`ClauseConstraint`]s (the
//!    where-clause bounds).
//!
//! 2. **Types are first-class** — Type expressions like `Int`, `*T`, and
//!    `fn(A) -> B` are lowered as regular HIR expressions (`ExprKind`
//!    variants) rather than a separate type AST.
//!
//! 3. **Error recovery** — Every lowering function has a fallback that
//!    emits a diagnostic through the project's [`DiagnosticContext`] and
//!    produces a recovery node (`ExprKind::Invalid`, `PatternKind::Err`,
//!    `ItemKind::Err`).

mod clause;
mod error;
mod expr;
mod item;
mod path;
mod pattern;

pub use error::{LoweringError, LoweringErrorKind};

use ast::{Ast, NodeIndex};
use diagnostic::{DiagnosticContext, FlurryError};
use hir::{
    HirArena, Package,
    body::Body,
    common::{Ident, Symbol},
    hir_id::{BodyId, HirId, ItemLocalId, OwnerId},
};
use rustc_span::{SourceMap, Span};

// ── Public API ───────────────────────────────────────────────────────────────

/// Lower a single file's AST into HIR, appending definitions to `package`.
///
/// This is the main entry point for AST lowering. The caller provides:
///
/// - `ast`        – the parsed AST of the source file.
/// - `arena`      – the HIR arena (owned by `CompilerInstance`).
/// - `source_map` – for resolving source text from spans.
/// - `diag_ctx`   – for emitting lowering errors and warnings.
/// - `package`    – the HIR package being built (may already contain
///                  items from other files).
pub fn lower_to_hir<'hir>(
    ast: &Ast,
    arena: &'hir HirArena,
    source_map: &SourceMap,
    diag_ctx: &DiagnosticContext<'_>,
    package: &mut Package<'hir>,
) {
    let mut ctx = LoweringContext::new(ast, arena, source_map, diag_ctx, package);
    ctx.lower_file_scope(ast.root);
}

/// Mutable context threaded through all lowering functions.
///
/// Owns the in-progress [`Package`] and maintains per-owner HirId
/// allocation state.
pub struct LoweringContext<'hir, 'ast> {
    pub(crate) ast: &'ast Ast,
    pub(crate) arena: &'hir HirArena,
    source_map: &'ast SourceMap,
    diag_ctx: &'ast DiagnosticContext<'ast>,

    pub(crate) package: &'ast mut Package<'hir>,

    /// The current owner being lowered.
    pub(crate) current_owner: OwnerId,
    /// Next `ItemLocalId` within the current owner.
    next_local_id: u32,
}

impl<'hir, 'ast> LoweringContext<'hir, 'ast> {
    fn new(
        ast: &'ast Ast,
        arena: &'hir HirArena,
        source_map: &'ast SourceMap,
        diag_ctx: &'ast DiagnosticContext<'ast>,
        package: &'ast mut Package<'hir>,
    ) -> Self {
        LoweringContext {
            ast,
            arena,
            source_map,
            diag_ctx,
            package,
            current_owner: OwnerId::INVALID,
            next_local_id: 0,
        }
    }

    // ── HirId allocation ─────────────────────────────────────────────────

    /// Allocate the next [`HirId`] within the current owner.
    pub(crate) fn next_hir_id(&mut self) -> HirId {
        let local = ItemLocalId::new(self.next_local_id);
        self.next_local_id += 1;
        HirId::new(self.current_owner, local)
    }

    /// Reset the local id counter (called when switching owners).
    pub(crate) fn reset_hir_id_counter(&mut self) {
        self.next_local_id = 0;
    }

    // ── Body allocation ──────────────────────────────────────────────────

    /// Allocate a [`BodyId`] and insert the body into the package.
    ///
    /// `owner_hir_id` is the [`HirId`] of the node that *owns* the body
    /// (e.g. a function item or a closure expression).
    pub(crate) fn alloc_body(&mut self, owner_hir_id: HirId, body: Body<'hir>) -> BodyId {
        let id = BodyId::new(owner_hir_id);
        self.package.insert_body(id, body);
        id
    }

    // ── Source text helpers ───────────────────────────────────────────────

    /// Get the source text for an AST node's span.
    pub(crate) fn source_text(&self, node: NodeIndex) -> String {
        self.ast
            .source_content(node, self.source_map)
            .unwrap_or_default()
    }

    /// Convert an AST node (expected to be an `Id` or similar leaf) into an
    /// HIR [`Ident`].
    pub(crate) fn node_to_ident(&self, node: NodeIndex) -> Ident {
        let span = self.ast.get_span(node).unwrap_or(Span::default());
        if node == 0 {
            return Ident::new(Symbol::intern(""), span);
        }
        let text = self.source_text(node);
        Ident::new(Symbol::intern(&text), span)
    }

    // ── Diagnostic helpers (delegate to DiagnosticContext) ────────────────

    pub(crate) fn emit_unsupported_node(&self, name: &str, span: Span) {
        let err = LoweringError::unsupported_node(name, span);
        err.emit(self.diag_ctx, rustc_span::BytePos(0));
    }

    pub(crate) fn emit_malformed(&self, msg: &str, span: Span) {
        let err = LoweringError::malformed_ast(msg, span);
        err.emit(self.diag_ctx, rustc_span::BytePos(0));
    }

    pub(crate) fn emit_unsupported_clause(&self, msg: &str, span: Span) {
        let err = LoweringError::unsupported_clause(msg, span);
        err.emit(self.diag_ctx, rustc_span::BytePos(0));
    }

    pub(crate) fn emit_invalid_parameter(&self, msg: &str, span: Span) {
        let err = LoweringError::invalid_parameter(msg, span);
        err.emit(self.diag_ctx, rustc_span::BytePos(0));
    }

    pub(crate) fn emit_invalid_pattern(&self, msg: &str, span: Span) {
        let err = LoweringError::invalid_pattern(msg, span);
        err.emit(self.diag_ctx, rustc_span::BytePos(0));
    }

    pub(crate) fn emit_invalid_item(&self, msg: &str, span: Span) {
        let err = LoweringError::invalid_item(msg, span);
        err.emit(self.diag_ctx, rustc_span::BytePos(0));
    }

    pub(crate) fn emit_invalid_enum_variant(&self, msg: &str, span: Span) {
        let err = LoweringError::invalid_enum_variant(msg, span);
        err.emit(self.diag_ctx, rustc_span::BytePos(0));
    }

    pub(crate) fn emit_invalid_struct_field(&self, msg: &str, span: Span) {
        let err = LoweringError::invalid_struct_field(msg, span);
        err.emit(self.diag_ctx, rustc_span::BytePos(0));
    }
}
