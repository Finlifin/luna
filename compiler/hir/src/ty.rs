//! HIR type-related declarations.
//!
//! In Flurry, types are first-class citizens – a type expression is just an
//! [`Expr`](super::expr::Expr) whose value happens to be a type. Therefore
//! there is **no** separate `Ty` struct; all type-position syntax
//! (`Int`, `&T`, `fn(A) -> B`, …) is represented via [`ExprKind`]
//! variants.
//!
//! This module retains only **trait bounds** and **clause parameters**
//! (generic parameters), which are declarative constructs rather than
//! expressions.

use rustc_span::Span;

use crate::common::{Ident, Path};
use crate::hir_id::HirId;

// ── TraitBound ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub struct TraitBound<'hir> {
    pub kind: TraitBoundKind<'hir>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TraitBoundKind<'hir> {
    Trait(Path<'hir>),
}

// ── ClauseParam (generic parameter) ──────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub struct ClauseParam<'hir> {
    pub hir_id: HirId,
    pub ident: Ident,
    pub bounds: &'hir [TraitBound<'hir>],
    pub span: Span,
}
