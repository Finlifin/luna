use crate::common::Ident;
use crate::{Expr, HirId};
use rustc_span::Span;

/// 由 let 语句或模式解构声明的变量。
#[derive(Debug, Clone, PartialEq)]
pub struct LetDecl<'hir> {
    pub hir_id: HirId,
    pub name: Ident,
    pub ty: Option<&'hir Expr<'hir>>,
    pub init: Option<&'hir Expr<'hir>>,
    pub span: Span,
}
