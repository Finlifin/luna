//! Apartness of retained immutable inputs to admitted nominal constructors.
//! This does not expose program equality or make opaque factories injective.
use super::{Sort, Term, UnknownReason};

pub(super) fn distinct(left: &Term, right: &Term, fuel: &mut usize) -> Result<bool, UnknownReason> {
    if left.sort() != Sort::ComptimeValue || right.sort() != Sort::ComptimeValue {
        return Ok(false);
    }
    // Reserve traversal and comparison of the complete retained terms first.
    // No result is published if their structure exceeds the query's budget.
    nir::type_computations::charge_term(left, fuel)
        .and_then(|()| nir::type_computations::charge_term(right, fuel))
        .map_err(|_| UnknownReason::BudgetExhausted)?;
    let mut pending = vec![(left, right)];
    while let Some((left, right)) = pending.pop() {
        if left.scalar_index_domain().is_some()
            && right.scalar_index_domain().is_some()
            && super::model_applications::fixed_argument(left)
            && super::model_applications::fixed_argument(right)
        {
            if left != right {
                return Ok(true);
            }
            continue;
        }
        let (left, right) = match (left, right) {
            (Term::TupleValue(left), Term::TupleValue(right)) => (left, right),
            (
                Term::NominalValue {
                    ty: a,
                    variant: av,
                    fields: af,
                },
                Term::NominalValue {
                    ty: b,
                    variant: bv,
                    fields: bf,
                },
            ) if a == b => {
                // Both values have the same retained domain. Their completed
                // payload shape and tag remain part of the index identity,
                // even when that domain is an opaque Type application.
                if av != bv {
                    return Ok(true);
                }
                (af, bf)
            }
            _ => continue,
        };
        if left.len() != right.len() {
            return Ok(true);
        }
        pending.extend(left.iter().zip(right));
    }
    // Different Type syntax, pending stores, floats and other value forms do
    // not supply an inequality proof through this operation.
    Ok(false)
}
