//! Owned immutable operands, separate from callable selection and expression trees.
use crate::{Error, copy_error};
use nir::program::{Expr, ExprKind as K, Literal};
use nir::{DeclarationId, Term};
use std::fmt;

#[derive(Debug, PartialEq)]
pub enum Kind {
    Literal(Literal),
    Type(Term),
    Frozen(Term),
    Function(DeclarationId),
}
#[derive(Debug)]
pub struct Constant {
    pub ty: Term,
    pub kind: Kind,
}
impl Constant {
    pub(crate) fn copy(expr: &Expr, fuel: &mut usize) -> Result<Self, Error> {
        nir::copy_budget::charge_substitution(expr, &[], &[], fuel)
            .map_err(|error| copy_error(error, expr.source))?;
        let kind = match &expr.kind {
            K::Literal(value) => Kind::Literal(value.clone()),
            K::TypeValue(value) => Kind::Type(value.clone()),
            K::ComptimeValue(value) => Kind::Frozen(value.clone()),
            K::Function(id) => Kind::Function(*id),
            _ => return Err(Error::Unsupported(expr.source)),
        };
        Ok(Self {
            ty: expr.ty.clone(),
            kind,
        })
    }
}
impl fmt::Display for Constant {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "(typed-constant (type {}) ", self.ty)?;
        match &self.kind {
            Kind::Literal(value) => write!(f, "(literal {value})")?,
            Kind::Type(value) => write!(f, "(type-value {value})")?,
            Kind::Frozen(value) => write!(f, "(frozen-value {value})")?,
            Kind::Function(id) => write!(f, "(function {id})")?,
        }
        f.write_str(")")
    }
}
