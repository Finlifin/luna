//! Closed effect-row interpretations belong to model evidence only. They never
//! reorder source rows or choose handler routes. Equal occurrences stay in
//! stable order; only completed distinct labels may commute in this domain.
use super::*;
use nir::effects::Operation;

fn argument(value: &Term, nominal: &BTreeSet<DeclarationId>) -> bool {
    model_applications::closed_argument(value, nominal)
}

pub(super) fn rebuild(
    effects: &[Operation],
    result: &Term,
    nominal: &BTreeSet<DeclarationId>,
    child: &mut impl FnMut(&Term) -> Option<Term>,
) -> Option<Term> {
    let effects = effects
        .iter()
        .map(|operation| {
            let arguments = operation
                .arguments
                .iter()
                .map(&mut *child)
                .collect::<Option<Vec<_>>>()?;
            arguments
                .iter()
                .all(|value| argument(value, nominal))
                .then_some(Operation {
                    declaration: operation.declaration,
                    arguments,
                })
        })
        .collect::<Option<Vec<_>>>()?;
    let result = child(result)?;
    if !closed_type(&result, nominal) {
        return None;
    }
    let mut value = Term::computation(effects, result);
    if let Term::Computation { effects, .. } = &mut value {
        // Stable sorting retains multiplicity and relative same-label order.
        // Every argument has a completed intrinsic interpretation here.
        effects.sort_by_cached_key(ToString::to_string);
    }
    Some(value)
}

pub(super) fn canonical(
    effects: &[Operation],
    result: &Term,
    nominal: &BTreeSet<DeclarationId>,
) -> bool {
    if effects.is_empty()
        || matches!(result, Term::Computation { .. })
        || !closed_type(result, nominal)
    {
        return false;
    }
    let mut previous = None;
    for operation in effects {
        if !operation
            .arguments
            .iter()
            .all(|value| argument(value, nominal))
        {
            return false;
        }
        let key = operation.to_string();
        if previous.as_ref().is_some_and(|previous| previous > &key) {
            return false;
        }
        previous = Some(key);
    }
    true
}
