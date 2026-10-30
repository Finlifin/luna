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
mod function_templates;
mod item;
mod pattern;
pub mod providers;
mod test_templates;

use std::collections::HashMap;

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
use resolve::{
    Binding, BindingKind, Resolution, ResolveError, ResolveResult, Resolver, Rib, RibKind, ScopeId,
    binding::Visibility::Private, ids::AstNodeRef,
};
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
pub fn lower_to_hir<'hir, 'ast, 'resolver: 'ast>(
    ast: &'ast Ast,
    file_id: vfs::FileId,
    arena: &'hir HirArena,
    source_map: &'ast SourceMap,
    diag_ctx: &'ast DiagnosticContext<'ast>,
    package: &'ast mut Package<'hir>,
    resolver: &'resolver mut Resolver<'resolver>,
    file_scope: ScopeId,
) {
    package.reserve_definition_prefix(resolver.next_definition_index());
    let mut ctx = LoweringContext::new(
        ast, file_id, arena, source_map, diag_ctx, package, resolver, file_scope,
    );
    ctx.lower_file_scope(ast.root);
}

/// Mutable context threaded through all lowering functions.
///
/// Owns the in-progress [`Package`] and maintains per-owner HirId
/// allocation state.
pub struct LoweringContext<'hir, 'ast, 'resolver> {
    pub(crate) ast: &'ast Ast,
    pub(crate) arena: &'hir HirArena,
    file_id: vfs::FileId,
    source_map: &'ast SourceMap,
    diag_ctx: &'ast DiagnosticContext<'ast>,

    pub(crate) package: &'ast mut Package<'hir>,

    /// The current owner being lowered.
    pub(crate) current_owner: OwnerId,
    /// Each owner keeps its allocation sequence across nested definitions.
    next_local_ids: HashMap<OwnerId, u32>,
    last_trait_sources: Option<std::sync::Arc<hir::trait_sources::TraitSourcePlan>>,

    /// Early name resolver – performs module-level name lookup.
    pub(crate) resolver: &'resolver mut Resolver<'resolver>,
    pub(crate) ast_ref_to_hir_id: HashMap<AstNodeRef, HirId>,
    pub(crate) expanded_locals: HashMap<(AstNodeRef, u32), HirId>,
    /// Lexically declared output names only; elaboration still verifies the
    /// code's category, subject and successful output types.
    pub(crate) pattern_code_outputs: HashMap<HirId, Vec<hir::common::Ident>>,
    /// The scope that owns the top-level names of the file being lowered.
    pub(crate) file_scope: ScopeId,

    pub(crate) surrouding_ctx: Vec<SurroundingContext>,
    pub(crate) refinement_depth: usize,
    pub(crate) test_template_ports: HashMap<resolve::DefId, Vec<hir::common::Ident>>,
}

impl<'hir, 'ast, 'resolver: 'ast> LoweringContext<'hir, 'ast, 'resolver> {
    fn new(
        ast: &'ast Ast,
        file_id: vfs::FileId,
        arena: &'hir HirArena,
        source_map: &'ast SourceMap,
        diag_ctx: &'ast DiagnosticContext<'ast>,
        package: &'ast mut Package<'hir>,
        resolver: &'resolver mut Resolver<'resolver>,
        file_scope: ScopeId,
    ) -> Self {
        LoweringContext {
            ast,
            file_id,
            arena,
            source_map,
            diag_ctx,
            package,
            current_owner: OwnerId::INVALID,
            next_local_ids: HashMap::new(),
            last_trait_sources: None,
            resolver,
            file_scope,
            ast_ref_to_hir_id: HashMap::default(),
            expanded_locals: HashMap::new(),
            pattern_code_outputs: HashMap::new(),
            surrouding_ctx: Vec::new(),
            refinement_depth: 0,
            test_template_ports: HashMap::new(),
        }
    }

    /// Allocate the next [`HirId`] within the current owner.
    pub(crate) fn next_hir_id(&mut self) -> HirId {
        let next = self.next_local_ids.entry(self.current_owner).or_default();
        let local = ItemLocalId::new(*next);
        *next += 1;
        HirId::new(self.current_owner, local)
    }

    pub(crate) fn ast_node_ref(&self, node: NodeIndex) -> AstNodeRef {
        AstNodeRef::new(self.file_id, node)
    }

    pub(crate) fn local_enter(&mut self, rib_kind: RibKind) -> usize {
        self.resolver
            .ribs_mut()
            .push(Rib::new(rib_kind, ScopeId::INVALID));
        self.resolver.ribs().depth() - 1
    }

    pub(crate) fn local_exit(&mut self, expected_rib_index: usize) {
        while self.resolver.ribs().depth() > expected_rib_index {
            self.resolver.ribs_mut().pop();
        }
    }

    pub(crate) fn resolve(&mut self, id: Ident) -> ResolveResult<Resolution> {
        self.resolver
            .resolve_name(id.name, self.file_scope, id.span)
    }

    pub(crate) fn define_symbol(&mut self, name: Ident, binding_kind: BindingKind) {
        self.resolver
            .ribs_mut()
            .current_mut()
            .expect("define_symbol called with no current rib")
            .define(
                name.name,
                Binding {
                    entry: None,
                    kind: binding_kind,
                    vis: Private,
                },
            );
    }

    pub(crate) fn define_local(
        &mut self,
        name: Ident,
        ast_node: ast::NodeIndex,
        corrresponding_hir_id: HirId,
    ) {
        let ast_node_ref = self.ast_node_ref(ast_node);
        self.define_symbol(name, BindingKind::Local(ast_node_ref));
        self.record_hir_id(ast_node_ref, corrresponding_hir_id);
    }

    pub(crate) fn define_fn_param(&mut self, name: Ident, ast_node: ast::NodeIndex, hir_id: HirId) {
        let ast_node_ref = self.ast_node_ref(ast_node);
        self.define_symbol(name, BindingKind::FnParam(ast_node_ref));
        self.record_hir_id(ast_node_ref, hir_id);
    }

    pub(crate) fn define_loop_label(
        &mut self,
        name: Ident,
        ast_node: ast::NodeIndex,
        hir_id: HirId,
    ) {
        let ast_node_ref = self.ast_node_ref(ast_node);
        self.define_symbol(name, BindingKind::LoopLabel(ast_node_ref));
        self.record_hir_id(ast_node_ref, hir_id);
    }

    /// Initialize a fresh owner's sequence without resetting resumed owners.
    pub(crate) fn ensure_owner_counter(&mut self) {
        self.next_local_ids.entry(self.current_owner).or_default();
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

    pub(crate) fn record_hir_id(&mut self, ast_node: AstNodeRef, hir_id: HirId) {
        self.ast_ref_to_hir_id.insert(ast_node, hir_id);
    }

    pub(crate) fn emit_resolve_error(&self, err: ResolveError) {
        err.emit(self.diag_ctx, self.ast.file_byte_pos);
    }

    pub(crate) fn emit_unsupported_node(&self, name: &str, span: Span) {
        let err = LoweringError::unsupported_node(name, span);
        err.emit(self.diag_ctx, self.ast.file_byte_pos);
    }

    pub(crate) fn emit_malformed(&self, msg: &str, span: Span) {
        let err = LoweringError::malformed_ast(msg, span);
        err.emit(self.diag_ctx, self.ast.file_byte_pos);
    }

    pub(crate) fn emit_unsupported_clause(&self, msg: &str, span: Span) {
        let err = LoweringError::unsupported_clause(msg, span);
        err.emit(self.diag_ctx, self.ast.file_byte_pos);
    }

    pub(crate) fn emit_invalid_parameter(&self, msg: &str, span: Span) {
        let err = LoweringError::invalid_parameter(msg, span);
        err.emit(self.diag_ctx, self.ast.file_byte_pos);
    }

    pub(crate) fn emit_invalid_pattern(&self, msg: &str, span: Span) {
        let err = LoweringError::invalid_pattern(msg, span);
        err.emit(self.diag_ctx, self.ast.file_byte_pos);
    }

    pub(crate) fn emit_invalid_item(&self, msg: &str, span: Span) {
        let err = LoweringError::invalid_item(msg, span);
        err.emit(self.diag_ctx, self.ast.file_byte_pos);
    }

    pub(crate) fn emit_invalid_enum_variant(&self, msg: &str, span: Span) {
        let err = LoweringError::invalid_enum_variant(msg, span);
        err.emit(self.diag_ctx, self.ast.file_byte_pos);
    }

    pub(crate) fn emit_invalid_struct_field(&self, msg: &str, span: Span) {
        let err = LoweringError::invalid_struct_field(msg, span);
        err.emit(self.diag_ctx, self.ast.file_byte_pos);
    }
}

pub(crate) enum SurroundingContext {
    // pure comptime fn(...) => AttributeSetTrue("__flurry_keyword_pure", AttributeSetTrue("__flurry_keyword_comptime", Definition.fn)),
    AttributeSetTrue(Symbol),
    Attribute(HirId),
}
