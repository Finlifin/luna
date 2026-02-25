//! HIR clause declarations (where-clauses / type constraints).

use rustc_span::Span;

use crate::common::Ident;
use crate::hir_id::HirId;
use crate::ty::TraitBound;

#[derive(Debug, Clone, PartialEq)]
pub struct ClauseConstraint<'hir> {
    pub hir_id: HirId,
    pub kind: ClauseConstraintKind<'hir>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ClauseConstraintKind<'hir> {
    Param(Ident),
    Bound(Ident, &'hir [TraitBound<'hir>]),
    Predicate(&'hir super::expr::Expr<'hir>),
}
