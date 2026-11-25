//! A06 `HasHeadShape(ty, decl)` head-shape predicate (`docs/type/propositional-constraints/atoms.md`).
//!
//! Checks only the outermost preserved nominal or normal-form head of `ty`
//! without propagating blindly across semantic equalities (`Eq`).
use nir::{DeclarationId, Term};
use std::fmt;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct HeadShapeQuery {
    pub ty: Term,
    pub head: DeclarationId,
}

impl HeadShapeQuery {
    /// Inspect the structural head of `self.ty` directly without consulting EUF
    /// equations. Open variables and unresolved terms stay `None`.
    pub fn evaluate(&self) -> Option<bool> {
        evaluate_term(&self.ty, self.head)
    }
}

pub(crate) fn evaluate_term(ty: &Term, head: DeclarationId) -> Option<bool> {
    match ty {
        Term::Nominal { declaration, .. } => Some(*declaration == head),
        Term::NormalForm { head: nf_head, .. } => Some(*nf_head == head),
        Term::Rigid { .. }
        | Term::Bound { .. }
        | Term::Hole { .. }
        | Term::AssociatedType { .. }
        | Term::AssociatedTypeCall(_)
        | Term::ClosureContextType
        | Term::ClosureContextValue(_)
        | Term::ContextualClosure { .. }
        | Term::Runtime(_)
        | Term::RuntimeField { .. } => None,
        _ => Some(false),
    }
}

impl fmt::Display for HeadShapeQuery {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "(has-head-shape {} {})", self.ty, self.head)
    }
}
