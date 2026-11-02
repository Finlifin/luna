//! Audited classification rules for currently represented closed types.
//! Nominal declarations need descriptors; factory provenance is not a kind.
use nir::{Primitive as P, Term};
use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TypeKind {
    Boolean,
    Symbol,
    Integer,
    SignedInteger,
    UnsignedInteger,
    FloatingPoint,
    Tuple,
    Pointer,
    Function,
}
impl TypeKind {
    pub(super) fn implies(self, other: Self) -> bool {
        self == other
            || matches!(
                (self, other),
                (Self::SignedInteger | Self::UnsignedInteger, Self::Integer)
            )
    }
    pub(super) fn disjoint(self, other: Self) -> bool {
        // Integer overlaps both signedness classes; these do not exhaust all
        // possible integer descriptions (e.g. mathematical Integer).
        !self.implies(other) && !other.implies(self)
    }
    pub(super) fn consequence(self, truth: bool, query: Self) -> Option<bool> {
        if truth && self.implies(query) {
            Some(true)
        } else if (truth && self.disjoint(query)) || (!truth && query.implies(self)) {
            Some(false)
        } else {
            None
        }
    }
    pub fn evaluate(self, ty: &Term) -> Option<bool> {
        let actual = match ty {
            Term::Primitive(P::Bool) => Self::Boolean,
            Term::Primitive(P::Symbol) => Self::Symbol,
            Term::Primitive(P::I8 | P::I16 | P::I32 | P::I64 | P::I128 | P::Isize) => {
                Self::SignedInteger
            }
            Term::Primitive(P::U8 | P::U16 | P::U32 | P::U64 | P::U128 | P::Usize) => {
                Self::UnsignedInteger
            }
            Term::Primitive(P::Integer) => {
                if matches!(self, Self::SignedInteger | Self::UnsignedInteger) {
                    return None;
                }
                Self::Integer
            }
            Term::Primitive(P::Float | P::F16 | P::Bf16 | P::F32 | P::F64) => Self::FloatingPoint,
            Term::Primitive(P::Unit) => return None, // legacy representation
            Term::Primitive(_) => return Some(false),
            Term::Tuple(_) => Self::Tuple,
            Term::Optional(_) => return Some(false),
            Term::Pointer(_) => Self::Pointer,
            Term::Function { .. }
            | Term::ComptimeFunction(_)
            | Term::NormalFormFunction(..)
            | Term::UnsafeFunction(_) => Self::Function,
            // Closure qualification and nominal/newtype descriptors are not
            // inferred by peeking through an environment or representation.
            _ => return None,
        };
        Some(actual.implies(self))
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct KindQuery {
    pub ty: Term,
    pub kind: TypeKind,
}
impl KindQuery {
    pub fn evaluate(&self) -> Option<bool> {
        self.kind.evaluate(&self.ty)
    }
}
pub(super) fn witnesses() -> Vec<Term> {
    vec![
        Term::Primitive(P::Bool),
        Term::Primitive(P::Symbol),
        Term::Primitive(P::U32),
        Term::Primitive(P::I32),
        Term::Primitive(P::F32),
        Term::Primitive(P::U8),
        Term::Primitive(P::U16),
        Term::Primitive(P::U64),
        Term::Primitive(P::U128),
        Term::Primitive(P::Usize),
        Term::Primitive(P::I8),
        Term::Primitive(P::I16),
        Term::Primitive(P::I64),
        Term::Primitive(P::I128),
        Term::Primitive(P::Isize),
        Term::Primitive(P::F16),
        Term::Primitive(P::Bf16),
        Term::Primitive(P::F64),
        Term::Primitive(P::Float),
        Term::Primitive(P::Integer),
        Term::Primitive(P::Char),
        Term::Pointer(Box::new(Term::Primitive(P::Bool))),
        Term::Tuple(vec![Term::Primitive(P::Bool)]),
        Term::Function {
            parameters: vec![],
            result: Box::new(Term::Primitive(P::Bool)),
        },
    ]
}
/// Distinct closed structural candidates, used only to construct models.
/// Width supplies fresh shapes without increasing recursive type depth. Reserve
/// their representation work before allocating; these are not domain axioms.
pub(super) fn structural_witnesses(width: usize, fuel: &mut usize) -> Option<[Term; 3]> {
    let remaining = width
        .checked_add(1)
        .and_then(|work| work.checked_mul(3))
        .and_then(|work| work.checked_add(3))
        .and_then(|work| fuel.checked_sub(work));
    let Some(remaining) = remaining else {
        *fuel = 0;
        return None;
    };
    *fuel = remaining;
    let shape = || Term::Tuple(vec![Term::Primitive(P::Bool); width]);
    Some([
        shape(),
        Term::Pointer(Box::new(shape())),
        Term::Function {
            parameters: vec![],
            result: Box::new(shape()),
        },
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn structural_candidates_reserve_storage_before_construction() {
        for width in [1_000_000, usize::MAX] {
            let mut fuel = 10;
            assert!(structural_witnesses(width, &mut fuel).is_none());
            assert_eq!(fuel, 0);
        }
        let mut fuel = 100;
        let candidates = structural_witnesses(3, &mut fuel).unwrap();
        assert!(fuel < 100);
        for (candidate, kind) in
            candidates
                .iter()
                .zip([TypeKind::Tuple, TypeKind::Pointer, TypeKind::Function])
        {
            assert_eq!(kind.evaluate(candidate), Some(true));
        }
    }
}

impl fmt::Display for TypeKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Boolean => "boolean",
            Self::Symbol => "symbol",
            Self::Integer => "integer",
            Self::SignedInteger => "signed-integer",
            Self::UnsignedInteger => "unsigned-integer",
            Self::FloatingPoint => "floating-point",
            Self::Tuple => "tuple",
            Self::Pointer => "pointer",
            Self::Function => "function",
        })
    }
}
impl fmt::Display for KindQuery {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "(type-kind {} {})", self.ty, self.kind)
    }
}
