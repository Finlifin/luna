//! Audited classification rules for currently represented closed types.
//! Nominal declarations need descriptors; factory provenance is not a kind.
pub use nir::TypeKind;
use nir::{Primitive as P, Term};
use std::fmt;

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

impl fmt::Display for KindQuery {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "(type-kind {} {})", self.ty, self.kind)
    }
}
