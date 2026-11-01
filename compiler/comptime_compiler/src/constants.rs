//! Owned immutable operands, separate from callable selection and expression trees.
use crate::{Error, copy_error};
use nir::program::{Expr, ExprKind as K, Literal};
use nir::{DeclarationId, Term};
use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;

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
// Keys borrow admitted source operands only during compilation. The emitted
// code owns the shared payloads and never retains these source references.
#[derive(PartialEq, Eq, Hash)]
enum Input<'a> {
    Literal(&'a Literal),
    Type(&'a Term),
    Frozen(&'a Term),
    Function(DeclarationId),
}
#[derive(Default)]
pub(crate) struct Pool<'a> {
    entries: HashMap<(&'a Term, Input<'a>), Arc<Constant>>,
}
impl<'a> Pool<'a> {
    pub(crate) fn get(&mut self, expr: &'a Expr, fuel: &mut usize) -> Result<Arc<Constant>, Error> {
        // Hashing and equality inspect payloads even on hits. Preserve bounded
        // traversal and depth admission before inspecting or copying a key.
        nir::copy_budget::charge_substitution(expr, &[], &[], fuel)
            .map_err(|error| copy_error(error, expr.source))?;
        let input = match &expr.kind {
            K::Literal(value) => Input::Literal(value),
            K::TypeValue(value) => Input::Type(value),
            K::ComptimeValue(value) => Input::Frozen(value),
            K::Function(id) => Input::Function(*id),
            _ => return Err(Error::Unsupported(expr.source)),
        };
        let key = (&expr.ty, input);
        if let Some(value) = self.entries.get(&key) {
            return Ok(value.clone());
        }
        let kind = match &key.1 {
            Input::Literal(value) => Kind::Literal((*value).clone()),
            Input::Type(value) => Kind::Type((*value).clone()),
            Input::Frozen(value) => Kind::Frozen((*value).clone()),
            Input::Function(id) => Kind::Function(*id),
        };
        let value = Arc::new(Constant {
            ty: expr.ty.clone(),
            kind,
        });
        self.entries.insert(key, value.clone());
        Ok(value)
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
