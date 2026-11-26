//! Field membership and field-type resolution queries for `Type.predicates.has_field`
//! and `Term::FieldType` over structural records (`Term::Record`) and checked structs.
use crate::DeclarationTheory;
use nir::{NominalKind, Term};
use std::fmt;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct HasFieldQuery {
    pub ty: Term,
    pub field: Term,
}

impl HasFieldQuery {
    pub fn evaluate_in(&self, theory: &DeclarationTheory) -> Option<bool> {
        let Term::Symbol(name) = &self.field else {
            return None;
        };
        has_field_known(theory, &self.ty, name.as_ref())
    }
}

pub(crate) fn has_field_known(theory: &DeclarationTheory, ty: &Term, field: &str) -> Option<bool> {
    match ty {
        Term::Record(fields) => Some(fields.iter().any(|(name, _)| name == field)),
        Term::Nominal { declaration, .. } => {
            if let Some(summary) = theory.struct_fields.get(declaration) {
                Some(summary.fields.iter().any(|(name, _)| name == field))
            } else if matches!(
                theory.constructors.nominal_constructors.get(declaration),
                Some(Some(NominalKind::Enum | NominalKind::Newtype))
            ) {
                Some(false)
            } else {
                None
            }
        }
        Term::Primitive(_)
        | Term::Tuple(_)
        | Term::Function { .. }
        | Term::Pointer(_)
        | Term::Optional(_)
        | Term::ErrorResult { .. }
        | Term::Computation { .. }
        | Term::Unit => Some(false),
        _ => None,
    }
}

pub(crate) fn resolve_field_type(
    theory: &DeclarationTheory,
    ty: &Term,
    field: &str,
) -> Option<Term> {
    match ty {
        Term::Record(fields) => fields
            .iter()
            .find(|(name, _)| name == field)
            .map(|(_, field_ty)| field_ty.clone()),
        Term::Nominal {
            declaration,
            arguments,
        } => {
            let summary = theory.struct_fields.get(declaration)?;
            let (_, raw_ty) = summary.fields.iter().find(|(name, _)| name == field)?;
            if summary.binders.is_empty() {
                return Some(raw_ty.clone());
            }
            raw_ty
                .instantiate_parameters(&summary.binders, arguments)
                .ok()
        }
        _ => None,
    }
}

impl fmt::Display for HasFieldQuery {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "(has-field {} {})", self.ty, self.field)
    }
}
