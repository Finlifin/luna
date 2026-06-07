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
mod pattern;
pub mod providers;

pub use error::{LoweringError, LoweringErrorKind};
pub use providers::set_providers;

use ast::{Ast, NodeIndex};
use diagnostic::{DiagnosticContext, FlurryError};
use hir::{
    HirArena, Package,
    body::Body,
    common::{Ident, Symbol},
    hir_id::{BodyId, HirId, ItemLocalId, OwnerId},
};
use resolve::{Resolver, ScopeId};
use rustc_span::{SourceMap, Span};

/// Lower a single file's AST into HIR, appending definitions to `package`.
///
/// This is the main entry point for AST lowering. The caller provides:
///
/// - `ast`          – the parsed AST of the source file.
/// - `arena`        – the HIR arena (owned by `CompilerInstance`).
/// - `source_map`   – for resolving source text from spans.
/// - `diag_ctx`     – for emitting lowering errors and warnings.
/// - `package`      – the HIR package being built (may already contain
///                    items from other files).
/// - `resolver`     – the early name resolver built from the module tree.
/// - `file_scope`   – the scope that owns the top-level definitions of this
///                    file (used as the starting point for name resolution).
pub fn lower_to_hir<'hir>(
    ast: &Ast,
    arena: &'hir HirArena,
    source_map: &SourceMap,
    diag_ctx: &DiagnosticContext<'_>,
    package: &mut Package<'hir>,
    resolver: &Resolver<'_>,
    file_scope: ScopeId,
) {
    let mut ctx = LoweringContext::new(
        ast, arena, source_map, diag_ctx, package, resolver, file_scope,
    );
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

    /// Early name resolver – performs module-level name lookup.
    pub(crate) resolver: &'ast Resolver<'ast>,
    /// The scope that owns the top-level names of the file being lowered.
    pub(crate) file_scope: ScopeId,

    pub(crate) surrouding_ctx: Vec<SurroundingContext>,
}

impl<'hir, 'ast> LoweringContext<'hir, 'ast> {
    fn new(
        ast: &'ast Ast,
        arena: &'hir HirArena,
        source_map: &'ast SourceMap,
        diag_ctx: &'ast DiagnosticContext<'ast>,
        package: &'ast mut Package<'hir>,
        resolver: &'ast Resolver<'ast>,
        file_scope: ScopeId,
    ) -> Self {
        LoweringContext {
            ast,
            arena,
            source_map,
            diag_ctx,
            package,
            current_owner: OwnerId::INVALID,
            next_local_id: 0,
            resolver,
            file_scope,
            surrouding_ctx: Vec::new(),
        }
    }

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

    /// Allocate a [`BodyId`] and insert the body into the package.
    ///
    /// `owner_hir_id` is the [`HirId`] of the node that *owns* the body
    /// (e.g. a function item or a closure expression).
    pub(crate) fn alloc_body(&mut self, owner_hir_id: HirId, body: Body<'hir>) -> BodyId {
        let id = BodyId::new(owner_hir_id);
        self.package.insert_body(id, body);
        id
    }

    /// Get the source text for an AST node's span.
    pub(crate) fn source_text(&self, node: NodeIndex) -> String {
        self.ast
            .source_content(node, self.source_map)
            .unwrap_or_default()
    }

    /// Reconstruct the [`Symbol`] embedded in the children of an `Id` AST
    /// node.  Falls back to source-text interning for nodes that pre-date the
    /// parallel symbol table (e.g. tests with hand-constructed ASTs).
    pub(crate) fn node_to_symbol(&self, node: NodeIndex) -> Symbol {
        if node == 0 {
            return Symbol::invalid();
        }
        let children = self.ast.get_children(node);
        // SAFETY: hi/lo were produced by Symbol::to_raw_parts() in the
        // parser and are valid for the lifetime of the process.
        unsafe { Symbol::from_raw_parts(children[0], children[1]) }
    }

    /// Convert an AST node (expected to be an `Id` or similar leaf) into an
    /// HIR [`Ident`].
    pub(crate) fn node_to_ident(&self, node: NodeIndex) -> Ident {
        let span = self.ast.get_span(node).unwrap_or(Span::default());
        let sym = self.node_to_symbol(node);
        Ident::new(sym, span)
    }

    pub(crate) fn push_surrounding_ctx(&mut self, ctx: SurroundingContext) {
        self.surrouding_ctx.push(ctx);
    }

    pub(crate) fn pop_surrounding_ctx(&mut self) {
        self.surrouding_ctx.pop();
    }

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

pub(crate) enum SurroundingContext {
    // pure comptime fn(...) => AttributeSetTrue("__flurry_keyword_pure", AttributeSetTrue("__flurry_keyword_comptime", Definition.fn)),
    AttributeSetTrue(Symbol),
    Attribute(HirId),
}
