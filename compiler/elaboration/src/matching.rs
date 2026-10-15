//! Inference proposals, validated by an obligation on the original types.
//! Structural decomposition never treats an opaque function as injective.
mod effects;
use crate::ptc::Budget;
use crate::{InferenceContext, InferenceError, InferenceTerm as I};
use nir::{Sort, Term};

#[derive(Debug, PartialEq, Eq)]
pub enum MatchError {
    Inference(InferenceError),
    BudgetExhausted,
}
impl From<InferenceError> for MatchError {
    fn from(value: InferenceError) -> Self {
        Self::Inference(value)
    }
}

pub(crate) fn propose(
    inference: &mut InferenceContext,
    left: &I,
    right: &I,
    context: &crate::ptc::Context,
    scope: &[Vec<Sort>],
    budget: Budget,
) -> Result<(), MatchError> {
    for term in [left, right] {
        let actual = inference.sort(term)?;
        if actual != Sort::Type {
            return Err(InferenceError::WrongSort {
                expected: Sort::Type,
                actual,
            }
            .into());
        }
    }
    let mut work = vec![(left.clone(), right.clone(), 0)];
    let mut fuel = budget.steps;
    while let Some((left, right, depth)) = work.pop() {
        tick(&mut fuel, depth, budget.max_depth)?;
        let left = inference.resolve(&left)?;
        let right = inference.resolve(&right)?;
        if left == right {
            continue;
        }
        match (&left, &right) {
            (I::Meta(id), _) => inference.assign(*id, right)?,
            (_, I::Meta(id)) => inference.assign(*id, left)?,
            (
                I::Instantiate {
                    template,
                    arguments,
                },
                I::Stable(actual),
            )
            | (
                I::Stable(actual),
                I::Instantiate {
                    template,
                    arguments,
                },
            ) => {
                decompose(
                    template,
                    actual,
                    arguments,
                    None,
                    0,
                    depth + 1,
                    &mut fuel,
                    budget.max_depth,
                    context,
                    scope,
                    &mut work,
                )?;
            }
            (
                I::Instantiate {
                    template: left,
                    arguments: la,
                },
                I::Instantiate {
                    template: right,
                    arguments: ra,
                },
            ) => {
                decompose(
                    left,
                    right,
                    la,
                    Some(ra),
                    0,
                    depth + 1,
                    &mut fuel,
                    budget.max_depth,
                    context,
                    scope,
                    &mut work,
                )?;
            }
            // Stable unequal terms need PTC; opaque heads are never inverted.
            _ => {}
        }
    }
    Ok(())
}
fn tick(fuel: &mut usize, depth: usize, limit: usize) -> Result<(), MatchError> {
    if depth >= limit.min(128) {
        return Err(MatchError::BudgetExhausted);
    }
    *fuel = fuel.checked_sub(1).ok_or(MatchError::BudgetExhausted)?;
    Ok(())
}
#[allow(clippy::too_many_arguments)]
fn decompose(
    template: &Term,
    actual: &Term,
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
    match (template, actual) {
        (
            Term::Bound {
                depth: binder,
                index,
                sort,
            },
            _,
        ) if *binder == nesting => {
            // Strip local binders before exporting a component. A dependency
            // on one of those binders cannot escape into a Type substitution.
            if *sort == Sort::Type && actual.sort() == Sort::Type {
                if let Some(actual) = extract(actual, actual_arguments, nesting, scope)? {
                    let argument = arguments.get(*index).ok_or_else(|| {
                        MatchError::Inference(InferenceError::InvalidSubstitution(
                            nir::TypeError::MissingArgument(*index),
                        ))
                    })?;
                    work.push((argument.clone(), actual, depth + 1));
                }
            }
        }
        (
            _,
            Term::Bound {
                depth: binder,
                index,
                sort,
            },
        ) if actual_arguments.is_some() && *binder == nesting => {
            if *sort == Sort::Type && template.sort() == Sort::Type {
                if let Some(template) = extract(template, Some(arguments), nesting, scope)? {
                    let argument = actual_arguments.unwrap().get(*index).ok_or_else(|| {
                        MatchError::Inference(InferenceError::InvalidSubstitution(
                            nir::TypeError::MissingArgument(*index),
                        ))
                    })?;
                    work.push((template, argument.clone(), depth + 1));
                }
            }
        }
        (
            Term::Computation {
                effects: a,
                result: ar,
            },
            Term::Computation {
                effects: b,
                result: br,
            },
        ) => {
            effects::computations(
                a,
                ar,
                b,
                br,
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
        (Term::Pointer(a), Term::Pointer(b))
        | (Term::ComptimeFunction(a), Term::ComptimeFunction(b)) => decompose(
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
        )?,
        (Term::Record(a), Term::Record(b))
            if a.len() == b.len() && a.iter().zip(b).all(|((a, _), (b, _))| a == b) =>
        {
            for ((_, a), (_, b)) in a.iter().zip(b) {
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
        (
            Term::NormalForm {
                head: a,
                arguments: left,
                sort: Sort::Type,
            },
            Term::NormalForm {
                head: b,
                arguments: right,
                sort: Sort::Type,
            },
        ) if a == b && context.is_tuple_head(*a) => {
            // Syntactic candidate substitutions only. The original application
            // equality remains an obligation; no inverse theorem is asserted.
            if let ([Term::TypeListValue(left)], [Term::TypeListValue(right)]) =
                (left.as_slice(), right.as_slice())
            {
                if left.len() == right.len() {
                    for (left, right) in left.iter().zip(right) {
                        decompose(
                            left,
                            right,
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
            }
        }
        (Term::Tuple(a), Term::Tuple(b)) if a.len() == b.len() => {
            for (a, b) in a.iter().zip(b) {
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
        (
            Term::Function {
                parameters: a,
                result: ar,
            },
            Term::Function {
                parameters: b,
                result: br,
            },
        ) if a.len() == b.len() && a.iter().zip(b).all(|(a, b)| a.sort == b.sort) => {
            for (a, b) in a.iter().zip(b) {
                decompose(
                    &a.ty,
                    &b.ty,
                    arguments,
                    actual_arguments,
                    nesting + 1,
                    depth + 1,
                    fuel,
                    limit,
                    context,
                    scope,
                    work,
                )?;
            }
            decompose(
                ar,
                br,
                arguments,
                actual_arguments,
                nesting + 1,
                depth + 1,
                fuel,
                limit,
                context,
                scope,
                work,
            )?;
        }
        (
            Term::Closure {
                environment: a,
                signature: sa,
            },
            Term::Closure {
                environment: b,
                signature: sb,
            },
        ) if a.len() == b.len() => {
            for (a, b) in a.iter().zip(b) {
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
            decompose(
                sa,
                sb,
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
        (
            Term::Nominal {
                declaration: a,
                arguments: aa,
            },
            Term::Nominal {
                declaration: b,
                arguments: ba,
            },
        ) if a == b && aa.len() == ba.len() && context.is_nominal_constructor(*a) => {
            for (left, right) in aa.iter().zip(ba) {
                decompose(
                    left,
                    right,
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
        // Unregistered nominal and normal-form arguments have no injectivity rule.
        // Shape mismatch is checked using the original obligation, not guessed.
        _ => {}
    }
    Ok(())
}

// Each removed frame is an actual local binder, not the substitution frame.
// Missing local arguments mean a dependent component cannot be extracted.
fn extract(
    term: &Term,
    arguments: Option<&[I]>,
    nesting: u32,
    scope: &[Vec<Sort>],
) -> Result<Option<I>, MatchError> {
    let mut term = term.clone();
    for _ in 0..nesting {
        match term.substitute(&[]) {
            Ok(value) => term = value,
            Err(nir::TypeError::MissingArgument(_)) => return Ok(None),
            Err(error) => return Err(InferenceError::InvalidSubstitution(error).into()),
        }
    }
    if let (
        Some(arguments),
        Term::Bound {
            depth: 0, index, ..
        },
    ) = (arguments, &term)
    {
        return arguments.get(*index).cloned().map(Some).ok_or_else(|| {
            InferenceError::InvalidSubstitution(nir::TypeError::MissingArgument(*index)).into()
        });
    }
    Ok(Some(match arguments {
        Some(arguments) => I::Instantiate {
            template: term,
            arguments: arguments.to_vec(),
        },
        None => {
            if term.check_scope(scope).is_err() {
                return Ok(None);
            }
            I::Stable(term)
        }
    }))
}
