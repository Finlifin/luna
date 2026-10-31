//! Recover structural views only through established regional equalities.
//! Candidate requirements are not assumptions, and satisfiable models are not
//! authorization to choose an input type or invert an opaque function.
use super::*;
use crate::ptc::{Formula, Outcome};

#[allow(clippy::too_many_arguments)]
pub(super) fn propose(
    template: &Term,
    actual: &Term,
    arguments: &[I],
    depth: usize,
    fuel: &mut usize,
    limit: usize,
    context: &MatchContext<'_>,
    scope: &[Vec<Sort>],
    work: &mut Vec<(I, I, usize)>,
) -> Result<(), MatchError> {
    let mut budget = Budget {
        steps: *fuel,
        max_depth: limit,
    };
    let candidates = context.mentioned_types_metered(&mut budget);
    *fuel = budget.steps;
    let candidates = candidates.map_err(|_| MatchError::BudgetExhausted)?;
    for shape in candidates {
        for term in [shape, actual] {
            nir::type_computations::charge_term_with_depth(term, fuel, limit)
                .map_err(|_| MatchError::BudgetExhausted)?;
        }
        if shape.sort() != Sort::Type || shape == actual {
            continue;
        }
        let mut proposals = Vec::new();
        // Screening a shape must not recursively search regional
        // aliases. This bounds alias cycles without dropping facts.
        let direct = MatchContext {
            theory: context.theory,
            alias_views: false,
        };
        match decompose(
            template,
            shape,
            arguments,
            None,
            0,
            depth,
            fuel,
            limit,
            &direct,
            scope,
            &mut proposals,
        ) {
            Err(MatchError::BudgetExhausted) => {
                return Err(MatchError::BudgetExhausted);
            }
            Err(MatchError::Inference(InferenceError::Budget)) => {
                return Err(MatchError::BudgetExhausted);
            }
            Err(_) => continue,
            Ok(()) => {}
        }
        if proposals.is_empty() {
            continue;
        }
        let mut budget = Budget {
            steps: *fuel,
            max_depth: limit,
        };
        let outcome =
            context.prove_metered(Formula::equal(actual.clone(), shape.clone()), &mut budget);
        *fuel = budget.steps;
        if matches!(outcome, Outcome::Entailed(_)) {
            work.extend(proposals);
            return Ok(());
        }
    }
    Ok(())
}
