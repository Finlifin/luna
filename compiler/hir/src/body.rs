//! HIR bodies – the heavy payloads of function/closure definitions.
//!
//! Bodies are stored **separately** from definition descriptors in the
//! package body table, enabling the compiler to inspect signatures without
//! loading full expression trees.

use rustc_span::Span;

pub use crate::hir_id::BodyId;
use crate::hir_id::HirId;
use crate::{Ident, common::FnParam};

#[derive(Debug, Clone, PartialEq)]
pub struct Body<'hir> {
    pub params: &'hir [BodyParam<'hir>],
    pub value: &'hir super::expr::Expr<'hir>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BodyParam<'hir> {
    pub hir_id: HirId,
    pub name: Ident,
    pub ty: crate::common::TyParam<'hir>,
    pub span: Span,
}

impl<'hir> BodyParam<'hir> {
    pub fn from_fn_param(fn_param: &FnParam<'hir>) -> Self {
        Self {
            hir_id: fn_param.hir_id,
            name: fn_param.name.clone(),
            ty: fn_param.ty.clone(),
            span: fn_param.span,
        }
    }
}
