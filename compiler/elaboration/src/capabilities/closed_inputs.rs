//! Closed-world absence requires closed inputs, possibly under proved caller
//! equalities. Structural views carry equality proofs; opaque heads are never
//! evaluated or inverted to obtain those views.
use super::*;
use crate::implementation::Key;
use crate::ptc::TraitQuery;

impl World<'_> {
    pub(super) fn closed_trait_inputs(&self, query: &TraitQuery, budget: Budget) -> bool {
        let allowance = Budget {
            steps: budget.steps / query.arguments.len().saturating_add(1),
            ..budget
        };
        query
            .inputs()
            .all(|input| closed(input, self.context, allowance, 0, &mut HashSet::new()))
    }
}
fn closed(
    ty: &Term,
    context: &Context,
    budget: Budget,
    depth: usize,
    active: &mut HashSet<Term>,
) -> bool {
    if depth >= budget.max_depth.min(64) || budget.steps < 4 {
        return false;
    }
    let allowance = Budget {
        steps: budget.steps / 4,
        ..budget
    };
    let Ok(key) = Key::from_type(ty, allowance) else {
        return false;
    };
    if !key.is_open() {
        return true;
    }
    if !active.insert(ty.clone()) {
        return false;
    }
    let children: Vec<&Term> = match ty {
        Term::Nominal { arguments, .. } | Term::Tuple(arguments) => arguments.iter().collect(),
        Term::NominalValue {ty,fields,..} => std::iter::once(ty.as_ref()).chain(fields.iter()).collect(),
        Term::Record(fields) | Term::RecordValue(fields) => fields.iter().map(|(_, ty)| ty).collect(),
        Term::Pointer(inner) => vec![inner],
        Term::Function { parameters, result } => parameters
            .iter()
            .map(|p| &p.ty)
            .chain(std::iter::once(result.as_ref()))
            .collect(),
        // A rigid/projection/opaque input needs an actual equality proof to a
        // structural type. A recursive equation does not establish closure.
        _ => {
            let result = match context.structural_view(ty, allowance) {
                Ok(Some((view, _proof))) if &view != ty => {
                    closed(&view, context, allowance, depth + 1, active)
                }
                _ => false,
            };
            active.remove(ty);
            return result;
        }
    };
    let allowance = Budget {
        steps: allowance.steps / children.len().max(1),
        ..allowance
    };
    let result = !children.is_empty()
        && children
            .into_iter()
            .all(|child| closed(child, context, allowance, depth + 1, active));
    active.remove(ty);
    result
}
