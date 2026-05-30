//! HIR clause declarations (where-clauses / type constraints).

use rustc_span::Span;

use crate::Expr;
use crate::common::Ident;
use crate::hir_id::HirId;

#[derive(Debug, Clone, PartialEq)]
pub struct ClauseParam<'hir> {
    pub hir_id: HirId,
    pub name: Ident,
    pub kind: ClauseParamKind<'hir>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ClauseParamKind<'hir> {
    Type(Ident),
    Positional(Ident, &'hir Expr<'hir>),
    Optional(Ident, &'hir Expr<'hir>),
    Varadic(Ident, &'hir Expr<'hir>),
    Quote(Ident, &'hir Expr<'hir>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct ClauseConstraint<'hir> {
    pub hir_id: HirId,
    pub kind: ClauseConstraintKind<'hir>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ClauseConstraintKind<'hir> {
    Requires(&'hir Expr<'hir>),
    Ensures(&'hir Expr<'hir>),
    Decreases(&'hir Expr<'hir>),

    /// TODO
    Outcome,
}
