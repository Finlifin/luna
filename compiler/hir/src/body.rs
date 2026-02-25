//! HIR bodies – the heavy payloads of function/closure definitions.
//!
//! Bodies are stored **separately** from definition descriptors in the
//! package body table, enabling the compiler to inspect signatures without
//! loading full expression trees.

use rustc_span::Span;

pub use crate::hir_id::BodyId;
use crate::hir_id::HirId;
use crate::pattern::Pattern;

#[derive(Debug, Clone, PartialEq)]
pub struct Body<'hir> {
    pub params: &'hir [Param<'hir>],
    pub value: &'hir super::expr::Expr<'hir>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Param<'hir> {
    pub hir_id: HirId,
    pub pat: Pattern<'hir>,
    pub ty: Option<&'hir super::expr::Expr<'hir>>,
    pub span: Span,
}
