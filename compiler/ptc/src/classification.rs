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
            Term::Function { .. } | Term::ComptimeFunction(_) | Term::UnsafeFunction(_) => {
                Self::Function
            }
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
        Term::Primitive(P::Char),
        Term::Pointer(Box::new(Term::Primitive(P::Bool))),
        Term::Tuple(vec![Term::Primitive(P::Bool)]),
        Term::Function {
            parameters: vec![],
            result: Box::new(Term::Primitive(P::Bool)),
        },
    ]
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
