//! Error rows are sets, not ordered effect occurrences. Proposals below retain
//! the original full equality obligation and never invert normal-form heads.
use super::*;

#[allow(clippy::too_many_arguments)]
pub(super) fn labels(
    left: &[Term],
    left_result: &Term,
    right: &[Term],
    right_result: &Term,
    arguments: &[I],
    actual_arguments: Option<&[I]>,
    nesting: u32,
    depth: usize,
    fuel: &mut usize,
    limit: usize,
    context: &crate::ptc::Context,
    scope: &[Vec<Sort>],
    work: &mut Vec<(I, I, usize)>,
) -> Result<(), MatchError> {
    tick(fuel, depth, limit)?;
    *fuel = fuel
        .checked_sub(left.len().saturating_add(right.len()))
        .ok_or(MatchError::BudgetExhausted)?;
    let known_left = left
        .iter()
        .map(|ty| ground(ty, Some(arguments), nesting, fuel, scope))
        .collect::<Result<Vec<_>, _>>()?;
    let known_right = right
        .iter()
        .map(|ty| ground(ty, actual_arguments, nesting, fuel, scope))
        .collect::<Result<Option<Vec<_>>, _>>()?;
    let Some(known_right) = known_right else {
        return Ok(());
    };
    let unresolved = known_left.iter().filter(|ty| ty.is_none()).count();
    // Set cancellation is justified only when the successful domains are
    // complete and identical: an unknown success domain could absorb labels.
    let fixed_success = match (
        ground(left_result, Some(arguments), nesting, fuel, scope)?,
        ground(right_result, actual_arguments, nesting, fuel, scope)?,
    ) {
        (Some(a), Some(b)) => a == b,
        _ => false,
    };
    for (i, template) in left.iter().enumerate() {
        if known_left[i].is_some() {
            continue;
        }
        let mut candidates = Vec::new();
        for (j, actual) in known_right.iter().enumerate() {
            tick(fuel, depth, limit)?;
            if compatible(template, actual, context)
                && !contains(&candidates, &known_right, actual, fuel)?
            {
                candidates.push(j);
            }
        }
        let selected = if candidates.len() == 1 {
            candidates.first().copied()
        } else if unresolved == 1 && fixed_success {
            let mut uncovered = Vec::new();
            for (j, actual) in known_right.iter().enumerate() {
                tick(fuel, depth, limit)?;
                let mut covered = false;
                for known in known_left.iter().flatten() {
                    tick(fuel, depth, limit)?;
                    if equal(known, actual, fuel)? {
                        covered = true;
                        break;
                    }
                }
                if !covered && !contains(&uncovered, &known_right, actual, fuel)? {
                    uncovered.push(j);
                }
            }
            if uncovered.len() == 1 && candidates.contains(&uncovered[0]) {
                Some(uncovered[0])
            } else {
                None
            }
        } else {
            None
        };
        if let Some(j) = selected {
            decompose(
                template,
                &right[j],
                arguments,
                actual_arguments,
                nesting,
                depth + 1,
                fuel,
                limit,
                context,
                scope,
                work,
            )?;
        }
    }
    Ok(())
}

fn equal(a: &Term, b: &Term, fuel: &mut usize) -> Result<bool, MatchError> {
    crate::associated::charge_projection_copy(a, fuel).map_err(|_| MatchError::BudgetExhausted)?;
    crate::associated::charge_projection_copy(b, fuel).map_err(|_| MatchError::BudgetExhausted)?;
    Ok(a == b)
}
fn contains(
    indices: &[usize],
    values: &[Term],
    actual: &Term,
    fuel: &mut usize,
) -> Result<bool, MatchError> {
    for &index in indices {
        if equal(&values[index], actual, fuel)? {
            return Ok(true);
        }
    }
    Ok(false)
}

fn compatible(template: &Term, actual: &Term, context: &crate::ptc::Context) -> bool {
    match (template, actual) {
        (Term::Nominal { declaration: a, .. }, Term::Nominal { declaration: b, .. })
            if context.is_nominal_constructor(*a) && context.is_nominal_constructor(*b) =>
        {
            a == b
        }
        (Term::Nominal { declaration, .. }, Term::Primitive(_))
            if context.is_nominal_constructor(*declaration) =>
        {
            false
        }
        // Opaque descriptions and caller parameters may still denote this
        // nominal type. Keep them as candidates, even if that leaves ambiguity.
        _ => true,
    }
}

fn ground(
    term: &Term,
    arguments: Option<&[I]>,
    nesting: u32,
    fuel: &mut usize,
    scope: &[Vec<Sort>],
) -> Result<Option<Term>, MatchError> {
    crate::associated::charge_projection_copy(term, fuel)
        .map_err(|_| MatchError::BudgetExhausted)?;
    let Some(component) = extract(term, arguments, nesting, scope)? else {
        return Ok(None);
    };
    let value = match component {
        I::Stable(value) => value,
        I::Meta(_) => return Ok(None),
        I::Instantiate {
            template,
            arguments,
        } => {
            if template.check_scope(&[]).is_ok() {
                template
            } else {
                let Some(arguments) = arguments
                    .into_iter()
                    .map(|value| match value {
                        I::Stable(value) => Some(value),
                        _ => None,
                    })
                    .collect::<Option<Vec<_>>>()
                else {
                    return Ok(None);
                };
                let before = *fuel;
                crate::associated::charge_projection_copy(&template, fuel)
                    .map_err(|_| MatchError::BudgetExhausted)?;
                let size = before - *fuel;
                let before = *fuel;
                for argument in &arguments {
                    crate::associated::charge_projection_copy(argument, fuel)
                        .map_err(|_| MatchError::BudgetExhausted)?;
                }
                let expansion = size
                    .checked_mul((before - *fuel).max(1))
                    .ok_or(MatchError::BudgetExhausted)?;
                *fuel = fuel
                    .checked_sub(expansion)
                    .ok_or(MatchError::BudgetExhausted)?;
                template
                    .substitute(&arguments)
                    .map_err(|e| MatchError::Inference(InferenceError::InvalidSubstitution(e)))?
            }
        }
    };
    crate::associated::charge_projection_copy(&value, fuel)
        .map_err(|_| MatchError::BudgetExhausted)?;
    Ok(Some(value))
}
