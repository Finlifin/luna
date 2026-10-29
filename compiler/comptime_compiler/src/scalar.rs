//! Operand-free descriptors shared by lowering and primitive execution.
use nir::Term;
use nir::program::{Expr, ExprKind as K, FloatOp, IntegerOp};
use std::fmt;

#[derive(Clone, Copy, Debug)]
pub enum ScalarKind {
    Integer(Option<IntegerOp>),
    Compare(IntegerOp),
    Float(Option<FloatOp>),
}
#[derive(Clone, Copy, Debug)]
pub struct Scalar<'a> {
    pub kind: ScalarKind,
    pub left: &'a Term,
    pub right: Option<&'a Term>,
    pub result: &'a Term,
}
impl<'a> Scalar<'a> {
    pub fn expression(expr: &'a Expr) -> Option<(Self, &'a Expr, Option<&'a Expr>)> {
        let (kind, left, right) = match &expr.kind {
            K::IntegerBinary { op, left, right } => (
                ScalarKind::Integer(Some(*op)),
                left.as_ref(),
                Some(right.as_ref()),
            ),
            K::FloatNegate(value) => (ScalarKind::Float(None), value.as_ref(), None),
            K::FloatBinary { op, left, right } => (
                ScalarKind::Float(Some(*op)),
                left.as_ref(),
                Some(right.as_ref()),
            ),
            K::IntegerNegate(value) => (ScalarKind::Integer(None), value.as_ref(), None),
            K::ScalarEqual(left, right) => (
                ScalarKind::Compare(IntegerOp::Equal),
                left.as_ref(),
                Some(right.as_ref()),
            ),
            K::ScalarLess(left, right) => (
                ScalarKind::Compare(IntegerOp::Less),
                left.as_ref(),
                Some(right.as_ref()),
            ),
            K::ScalarLessEqual(left, right) => (
                ScalarKind::Compare(IntegerOp::LessEqual),
                left.as_ref(),
                Some(right.as_ref()),
            ),
            K::ScalarGreater(left, right) => (
                ScalarKind::Compare(IntegerOp::Greater),
                left.as_ref(),
                Some(right.as_ref()),
            ),
            K::ScalarGreaterEqual(left, right) => (
                ScalarKind::Compare(IntegerOp::GreaterEqual),
                left.as_ref(),
                Some(right.as_ref()),
            ),
            _ => return None,
        };
        Some((
            Self {
                kind,
                left: &left.ty,
                right: right.map(|value| &value.ty),
                result: &expr.ty,
            },
            left,
            right,
        ))
    }
}
impl fmt::Display for Scalar<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.kind {
            ScalarKind::Float(None) => write!(f, "(float negate")?,
            ScalarKind::Float(Some(op)) => write!(f, "(float {op}")?,
            ScalarKind::Integer(None) => write!(f, "(integer negate")?,
            ScalarKind::Integer(Some(op)) => write!(f, "(integer {op}")?,
            ScalarKind::Compare(op) => write!(f, "(compare {op}")?,
        }
        write!(f, " (inputs {}", self.left)?;
        if let Some(right) = self.right {
            write!(f, " {right}")?;
        }
        write!(f, ") (result {}))", self.result)
    }
}
