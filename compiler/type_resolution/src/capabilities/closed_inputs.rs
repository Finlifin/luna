//! Closed-world absence requires closed inputs, possibly under proved caller
//! equalities. Structural views carry equality proofs; opaque heads are never
//! evaluated or inverted to obtain those views.
use super::*;
use crate::implementation::Key;
use crate::ptc::TraitQuery;

impl World<'_> {
    pub(super) fn closed_trait_inputs_metered(
        &self,
        query: &TraitQuery,
        budget: &mut Budget,
    ) -> Result<bool, QueryError> {
        for input in query.inputs() {
            if !closed(input, self.context, budget, 0, &mut HashSet::new())? {
                return Ok(false);
            }
        }
        Ok(true)
    }
}
fn closed(
    ty: &Term,
    context: &Context,
    budget: &mut Budget,
    depth: usize,
    active: &mut HashSet<Term>,
) -> Result<bool, QueryError> {
    if depth >= budget.max_depth.min(64) {
        return Err(QueryError::Budget);
    }
    let key = Key::from_type_metered(ty, budget).map_err(|error| match error {
        crate::implementation::Error::BudgetExhausted => QueryError::Budget,
        error => QueryError::Invalid(format!("closed trait input: {error:?}")),
    })?;
    if !key.is_open() {
        return Ok(true);
    }
    if !active.insert(ty.clone()) {
        return Ok(false);
    }
    let children: Vec<&Term> = match ty {
        Term::Nominal { arguments, .. } | Term::Tuple(arguments) => arguments.iter().collect(),
        Term::OptionalValue { ty, value } => std::iter::once(ty.as_ref())
            .chain(value.as_deref())
            .collect(),
        Term::ErrorValue { ty, label, value } => std::iter::once(ty.as_ref())
            .chain(label.as_deref())
            .chain(std::iter::once(value.as_ref()))
            .collect(),
        Term::NominalValue { ty, fields, .. } => {
            std::iter::once(ty.as_ref()).chain(fields.iter()).collect()
        }
        Term::Record(fields) | Term::RecordValue(fields) => {
            fields.iter().map(|(_, ty)| ty).collect()
        }
        Term::Optional(inner) | Term::Pointer(inner) => vec![inner],
        Term::Function { parameters, result } => parameters
            .iter()
            .map(|p| &p.ty)
            .chain(std::iter::once(result.as_ref()))
            .collect(),
        // A rigid/projection/opaque input needs an actual equality proof to a
        // structural type. A recursive equation does not establish closure.
        _ => {
            let (view, consumed) = context.structural_view_metered(ty, *budget);
            budget.steps -= consumed;
            let result = match view {
                Ok(Some((view, _proof))) if &view != ty => {
                    closed(&view, context, budget, depth + 1, active)
                }
                Ok(_) => Ok(false),
                Err(crate::ptc::UnknownReason::BudgetExhausted) => Err(QueryError::Budget),
                Err(reason) => Err(QueryError::Unresolved(format!(
                    "closed trait input view: {reason:?}"
                ))),
            };
            active.remove(ty);
            return result;
        }
    };
    let result = (|| {
        if children.is_empty() {
            return Ok(false);
        }
        for child in children {
            if !closed(child, context, budget, depth + 1, active)? {
                return Ok(false);
            }
        }
        Ok(true)
    })();
    active.remove(ty);
    result
}
