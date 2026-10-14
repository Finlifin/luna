//! HIR clause declarations (where-clauses / type constraints).

use rustc_span::Span;

use crate::Expr;
use crate::common::Ident;
use crate::hir_id::HirId;

#[derive(Debug, Clone, PartialEq)]
pub struct ClauseParam<'hir> {
    pub hir_id: HirId,
    /// Resolver identity used by ResolvedClauseParam references.
    pub definition: Option<symbol::DefId>,
    pub name: Ident,
    pub kind: ClauseParamKind<'hir>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ClauseParamKind<'hir> {
    Type(Ident),
    Positional(Ident, &'hir Expr<'hir>),
    Optional(Ident, &'hir Expr<'hir>, &'hir Expr<'hir>),
    Varadic(Ident, &'hir Expr<'hir>),
    TraitBound(Ident, &'hir Expr<'hir>),
    Quote(Box<ClauseParamKind<'hir>>),
}

/// A where-clause constraint on a definition.
///
/// Analogous to rustc's `WherePredicate`, but also covers Flurry-specific
/// contract clauses (`requires`, `ensures`, `decreases`).
#[derive(Debug, Clone, PartialEq)]
pub struct ClauseConstraint<'hir> {
    pub label: Option<Ident>,
    pub hir_id: HirId,
    pub kind: ClauseConstraintKind<'hir>,
    pub span: Span,
}

/// The specific form of a [`ClauseConstraint`].
#[derive(Debug, Clone, PartialEq)]
pub enum ClauseConstraintKind<'hir> {
    /// Pre-condition (assertion that must hold on entry).
    Requires(&'hir Expr<'hir>),
    /// Post-condition (assertion that must hold on exit).
    Ensures(&'hir Expr<'hir>),
    /// Termination metric for recursive functions.
    Decreases(&'hir Expr<'hir>),

    /// TODO
    Outcome,
}
