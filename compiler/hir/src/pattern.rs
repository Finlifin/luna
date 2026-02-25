//! HIR pattern types.

use rustc_span::Span;

use crate::Expr;
use crate::common::{BindingMode, Ident, Path};
use crate::hir_id::HirId;

#[derive(Debug, Clone, PartialEq)]
pub struct Pattern<'hir> {
    pub hir_id: HirId,
    pub kind: PatternKind<'hir>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum PatternKind<'hir> {
    Wild,
    Binding(BindingMode, Ident, Option<&'hir Pattern<'hir>>),
    Const(&'hir Expr<'hir>),
    Tuple(&'hir [Pattern<'hir>]),
    Struct(Path<'hir>, &'hir [FieldPat<'hir>], bool),
    TupleStruct(&'hir Expr<'hir>, &'hir [Pattern<'hir>]),
    Or(&'hir [Pattern<'hir>]),
    Ref(&'hir Pattern<'hir>),
    Path(Path<'hir>),
    Range(
        Option<&'hir super::expr::Expr<'hir>>,
        Option<&'hir super::expr::Expr<'hir>>,
    ),
    Err,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FieldPat<'hir> {
    pub ident: Ident,
    pub pat: Pattern<'hir>,
    pub span: Span,
}
