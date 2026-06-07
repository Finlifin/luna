//! HIR let-declaration node.

use crate::common::Ident;
use crate::{Expr, HirId};
use rustc_span::Span;

/// A variable binding introduced by a `let` statement or destructuring pattern.
#[derive(Debug, Clone, PartialEq)]
pub struct LetDecl<'hir> {
    pub hir_id: HirId,
    pub name: Ident,
    pub ty: Option<&'hir Expr<'hir>>,
    pub init: Option<&'hir Expr<'hir>>,
    pub span: Span,
}
