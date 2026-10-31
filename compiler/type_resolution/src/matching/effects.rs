//! Propose substitutions only for unambiguous operation correspondences.
//! The original complete computation-type equality remains an obligation;
//! this is not an injectivity rule for normal forms or an executable row map.
use super::*;
use nir::effects::Operation;

#[allow(clippy::too_many_arguments)]
pub(super) fn computations(
    left: &[Operation],
    left_result: &Term,
    right: &[Operation],
    right_result: &Term,
    arguments: &[I],
    actual_arguments: Option<&[I]>,
    nesting: u32,
    depth: usize,
    fuel: &mut usize,
    limit: usize,
    context: &MatchContext<'_>,
    scope: &[Vec<Sort>],
    work: &mut Vec<(I, I, usize)>,
) -> Result<(), MatchError> {
    tick(fuel, depth, limit)?;
    if left.len().saturating_add(right.len()) > *fuel {
        return Err(MatchError::BudgetExhausted);
    }
    *fuel -= left.len() + right.len();
    for operation in left.iter().chain(right) {
        for argument in &operation.arguments {
            crate::associated::charge_projection_copy(argument, fuel)
                .map_err(|_| MatchError::BudgetExhausted)?;
        }
    }
    let mut used_left = vec![false; left.len()];
    let mut used_right = vec![false; right.len()];
    // Cancel already-ground identical instances first. For example, matching
    // [echo<T>, echo<bool>] against [echo<char>, echo<bool>] leaves T = char.
    for (i, a) in left.iter().enumerate() {
        let Some(a) = ground_operation(a, Some(arguments), nesting, fuel, scope)? else {
            continue;
        };
        for (j, b) in right.iter().enumerate() {
            tick(fuel, depth, limit)?;
            if used_right[j] {
                continue;
            }
            let Some(b) = ground_operation(b, actual_arguments, nesting, fuel, scope)? else {
                continue;
            };
            if a == b {
                used_left[i] = true;
                used_right[j] = true;
                break;
            }
        }
    }
    // No positional zip: unrelated labels may commute. A remaining declaration
    // must have one candidate, or all candidates must propose the same inputs.
    for (i, a) in left.iter().enumerate() {
        if used_left[i] {
            continue;
        }
        let mut candidates = Vec::new();
        for (j, b) in right.iter().enumerate() {
            tick(fuel, depth, limit)?;
            if !used_right[j]
                && a.declaration == b.declaration
                && a.arguments.len() == b.arguments.len()
            {
                candidates.push(j);
            }
        }
        let Some(&j) = candidates.first() else {
            continue;
        };
        let mut equivalent_candidates = true;
        for &other in &candidates[1..] {
            tick(fuel, depth, limit)?;
            for argument in right[j].arguments.iter().chain(&right[other].arguments) {
                crate::associated::charge_projection_copy(argument, fuel)
                    .map_err(|_| MatchError::BudgetExhausted)?;
            }
            if right[j] != right[other] {
                equivalent_candidates = false;
                break;
            }
        }
        // Multiple differing candidates need additional type information;
        // choosing the first would silently choose a generic instantiation.
        if !equivalent_candidates {
            continue;
        }
        used_left[i] = true;
        used_right[j] = true;
        for (a, b) in a.arguments.iter().zip(&right[j].arguments) {
            decompose(
                a,
                b,
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
    let unmatched_left: Vec<_> = left
        .iter()
        .zip(&used_left)
        .filter_map(|(op, used)| (!used).then_some(op))
        .collect();
    let unmatched_right: Vec<_> = right
        .iter()
        .zip(&used_right)
        .filter_map(|(op, used)| (!used).then_some(op))
        .collect();
    // Flattened prefixes can leave effects inside an inferred result Type.
    // Export the whole remainder rather than discarding its allowed row.
    if unmatched_left.is_empty() {
        let residual = residual(&unmatched_right, right_result, fuel)?;
        decompose(
            left_result,
            &residual,
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
    } else if unmatched_right.is_empty() && actual_arguments.is_some() {
        let residual = residual(&unmatched_left, left_result, fuel)?;
        decompose(
            &residual,
            right_result,
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
    } else if left.len() == right.len() {
        // Result information remains useful even while label pairing waits.
        decompose(
            left_result,
            right_result,
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
    Ok(())
}

fn residual(
    operations: &[&Operation],
    result: &Term,
    fuel: &mut usize,
) -> Result<Term, MatchError> {
    crate::associated::charge_projection_copy(result, fuel)
        .map_err(|_| MatchError::BudgetExhausted)?;
    for operation in operations {
        *fuel = fuel.checked_sub(1).ok_or(MatchError::BudgetExhausted)?;
        for argument in &operation.arguments {
            crate::associated::charge_projection_copy(argument, fuel)
                .map_err(|_| MatchError::BudgetExhausted)?;
        }
    }
    Ok(Term::computation(
        operations.iter().map(|op| (*op).clone()).collect(),
        result.clone(),
    ))
}

fn ground_operation(
    operation: &Operation,
    arguments: Option<&[I]>,
    nesting: u32,
    fuel: &mut usize,
    scope: &[Vec<Sort>],
) -> Result<Option<Operation>, MatchError> {
    let mut values = Vec::new();
    for term in &operation.arguments {
        crate::associated::charge_projection_copy(term, fuel)
            .map_err(|_| MatchError::BudgetExhausted)?;
        let Some(component) = extract(term, arguments, nesting, scope)? else {
            return Ok(None);
        };
        let value = match component {
            I::Stable(value) => value,
            I::Instantiate {
                template,
                arguments,
            } => {
                if template.check_scope(&[]).is_ok() {
                    template
                } else {
                    let Some(arguments) = arguments
                        .into_iter()
                        .map(|arg| {
                            if let I::Stable(value) = arg {
                                Some(value)
                            } else {
                                None
                            }
                        })
                        .collect::<Option<Vec<_>>>()
                    else {
                        return Ok(None);
                    };
                    let before = *fuel;
                    crate::associated::charge_projection_copy(&template, fuel)
                        .map_err(|_| MatchError::BudgetExhausted)?;
                    let template_cost = before - *fuel;
                    let before = *fuel;
                    for argument in &arguments {
                        crate::associated::charge_projection_copy(argument, fuel)
                            .map_err(|_| MatchError::BudgetExhausted)?;
                    }
                    let expansion = template_cost
                        .checked_mul((before - *fuel).max(1))
                        .ok_or(MatchError::BudgetExhausted)?;
                    *fuel = fuel
                        .checked_sub(expansion)
                        .ok_or(MatchError::BudgetExhausted)?;
                    template.substitute(&arguments).map_err(|error| {
                        MatchError::Inference(InferenceError::InvalidSubstitution(error))
                    })?
                }
            }
            I::Meta(_) => return Ok(None),
        };
        crate::associated::charge_projection_copy(&value, fuel)
            .map_err(|_| MatchError::BudgetExhausted)?;
        values.push(value);
    }
    Ok(Some(Operation {
        declaration: operation.declaration,
        arguments: values,
    }))
}
