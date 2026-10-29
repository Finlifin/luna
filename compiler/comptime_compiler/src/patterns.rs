//! Admit structural patterns before publishing any invocation code. Patterns
//! containing executable expressions need their own suspended VM continuations.
use super::*;
use nir::program::Pattern as P;
use std::collections::BTreeSet;

pub(super) fn copy(
    pattern: &P,
    fuel: &mut usize,
    source: SourceRange,
) -> Result<(Arc<P>, Vec<LocalId>), Error> {
    let mut budget = *fuel as u64;
    nir::copy_budget::charge_pattern_rewrite(pattern, &mut budget).map_err(
        |error| match error {
            // The owned pattern still uses a recursive representation. Refuse
            // that carrier before cloning; the tree matcher retains its own
            // depth diagnostic. Representation depth is not execution fuel.
            nir::copy_budget::CopyBudgetExhausted::Depth => Error::Unsupported(source),
            nir::copy_budget::CopyBudgetExhausted::Steps => Error::Budget(source),
        },
    )?;
    *fuel = budget as usize;
    let mut work = vec![pattern];
    let mut bindings = BTreeSet::new();
    while let Some(pattern) = work.pop() {
        *fuel = fuel.checked_sub(1).ok_or(Error::Budget(source))?;
        match pattern {
            P::Then { .. } | P::Guard { .. } | P::CodeEvidence { .. } => {
                return Err(Error::Unsupported(source));
            }
            P::Binding { binding, inner } | P::Observe { binding, inner } => {
                bindings.insert(*binding);
                work.push(inner);
            }
            P::Remap {
                pattern,
                bindings: pairs,
            } => {
                for (a, b) in pairs {
                    bindings.insert(*a);
                    bindings.insert(*b);
                }
                work.push(pattern);
            }
            P::OptionalPresent(inner) | P::ErrorResult { inner, .. } => work.push(inner),
            P::NominalType { fields, .. }
            | P::AnonymousRecord { fields, .. }
            | P::Record { fields, .. }
            | P::Tuple(fields)
            | P::TypeTuple(fields)
            | P::Variant { fields, .. }
            | P::Or(fields)
            | P::NormalForm {
                arguments: fields, ..
            } => work.extend(fields),
            P::Wild | P::Unit | P::OptionalAbsent | P::Literal(_) => {}
        }
    }
    Ok((Arc::new(pattern.clone()), bindings.into_iter().collect()))
}
