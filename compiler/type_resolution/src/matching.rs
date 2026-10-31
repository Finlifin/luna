//! Inference proposals, validated by an obligation on the original types.
//! Structural decomposition never treats an opaque function as injective.
mod aliases;
mod effects;
mod error_rows;
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

pub fn propose(
    inference: &mut InferenceContext,
    left: &I,
    right: &I,
    context: &crate::ptc::Context,
    scope: &[Vec<Sort>],
    budget: Budget,
) -> Result<usize, MatchError> {
    let mut remaining = budget;
    propose_metered(inference, left, right, context, scope, &mut remaining)?;
    Ok(budget.steps - remaining.steps)
}

pub(crate) fn propose_metered(
    inference: &mut InferenceContext,
    left: &I,
    right: &I,
    context: &crate::ptc::Context,
    scope: &[Vec<Sort>],
    budget: &mut Budget,
) -> Result<(), MatchError> {
    let mut fuel = budget.steps;
    let result = (|| {
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
        let left = crate::inference_budget::copy_input(left, &mut fuel, budget.max_depth)
            .map_err(|_| MatchError::BudgetExhausted)?;
        let right = crate::inference_budget::copy_input(right, &mut fuel, budget.max_depth)
            .map_err(|_| MatchError::BudgetExhausted)?;
        let mut work = vec![(left, right, 0)];
        while let Some((left, right, depth)) = work.pop() {
            tick(&mut fuel, depth, budget.max_depth)?;
            if left == right {
                continue;
            }
            let left = resolved_shape(inference, &left, &mut fuel)?;
            let right = resolved_shape(inference, &right, &mut fuel)?;
            if left == right {
                continue;
            }
            match (&left, &right) {
                (I::Meta(id), _) => inference
                    .assign_with_limits(*id, right, &mut fuel, budget.max_depth)
                    .map_err(match_error)?,
                (_, I::Meta(id)) => inference
                    .assign_with_limits(*id, left, &mut fuel, budget.max_depth)
                    .map_err(match_error)?,
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
                    let previous = work.len();
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
                    if work.len() == previous
                        && arguments
                            .iter()
                            .any(|argument| !matches!(argument, I::Stable(_)))
                    {
                        aliases::propose(
                            template,
                            actual,
                            arguments,
                            depth + 1,
                            &mut fuel,
                            budget.max_depth,
                            context,
                            scope,
                            &mut work,
                        )?;
                    }
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
    })();
    budget.steps = fuel;
    result
}

fn match_error(error: InferenceError) -> MatchError {
    match error {
        InferenceError::Budget => MatchError::BudgetExhausted,
        error => MatchError::Inference(error),
    }
}

fn resolved_shape(
    inference: &InferenceContext,
    term: &I,
    fuel: &mut usize,
) -> Result<I, MatchError> {
    let resolve = |term: &I, fuel: &mut usize| {
        inference
            .resolve_with_limits(term, fuel, 128)
            .map_err(match_error)
    };
    let mut term = resolve(term, fuel)?;
    if let I::Instantiate { arguments, .. } = &mut term {
        for argument in arguments {
            *fuel = fuel.checked_sub(1).ok_or(MatchError::BudgetExhausted)?;
            *argument = resolve(argument, fuel)?;
        }
    }
    Ok(term)
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
            // on one of those binders cannot escape into a substitution.
            // Value proposals occur only inside a matched type identity; the
            // original enclosing Type obligation still validates the result.
            if *sort == actual.sort() {
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
            if *sort == template.sort() {
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
            Term::ContextualClosure {
                context: a,
                signature: sa,
            },
            Term::ContextualClosure {
                context: b,
                signature: sb,
            },
        ) => {
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
            Term::ContextualClosure {
                context: environment,
                signature,
            },
            actual,
        ) if matches!(
            actual,
            Term::Closure { .. }
                | Term::Function { .. }
                | Term::ComptimeFunction(_)
                | Term::NormalFormFunction(..)
                | Term::UnsafeFunction(_)
        ) =>
        {
            let (fields, body) = match actual {
                Term::Closure {
                    environment,
                    signature,
                } => (environment.as_slice(), signature.as_ref()),
                body => (&[][..], body),
            };
            *fuel = fuel
                .checked_sub(fields.len())
                .ok_or(MatchError::BudgetExhausted)?;
            let supplied = Term::ClosureContextValue(fields.to_vec());
            decompose(
                environment,
                &supplied,
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
            decompose(
                signature,
                body,
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
            template,
            Term::ContextualClosure {
                context: environment,
                signature,
            },
        ) if matches!(
            template,
            Term::Closure { .. }
                | Term::Function { .. }
                | Term::ComptimeFunction(_)
                | Term::NormalFormFunction(..)
                | Term::UnsafeFunction(_)
        ) =>
        {
            let (fields, body) = match template {
                Term::Closure {
                    environment,
                    signature,
                } => (environment.as_slice(), signature.as_ref()),
                body => (&[][..], body),
            };
            *fuel = fuel
                .checked_sub(fields.len())
                .ok_or(MatchError::BudgetExhausted)?;
            let supplied = Term::ClosureContextValue(fields.to_vec());
            decompose(
                &supplied,
                environment,
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
            decompose(
                body,
                signature,
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
        (Term::ClosureContextValue(a), Term::ClosureContextValue(b)) if a.len() == b.len() => {
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
        (
            Term::ErrorResult {
                errors: left,
                result: a,
            },
            Term::ErrorResult {
                errors: right,
                result: b,
            },
        ) => {
            error_rows::labels(
                left,
                a,
                right,
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
            if let Some(actual_arguments) = actual_arguments {
                error_rows::labels(
                    right,
                    b,
                    left,
                    a,
                    actual_arguments,
                    Some(arguments),
                    nesting,
                    depth + 1,
                    fuel,
                    limit,
                    context,
                    scope,
                    work,
                )?;
            }
            // Infer the successful component; row compatibility remains an
            // obligation on the original complete types. Never zip unordered
            // labels or infer their identity from layout positions.
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
        (Term::NormalFormFunction(a, ac), Term::NormalFormFunction(b, bc)) if ac == bc => {
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
        (Term::Optional(a), Term::Optional(b))
        | (Term::Pointer(a), Term::Pointer(b))
        | (Term::ComptimeFunction(a), Term::ComptimeFunction(b))
        | (Term::UnsafeFunction(a), Term::UnsafeFunction(b)) => decompose(
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
        (Term::Record(a), Term::Record(b)) | (Term::RecordValue(a), Term::RecordValue(b))
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
        ) if a == b && left.len() == right.len() => {
            // Syntactic candidate substitutions only. The original application
            // equality remains an obligation; no inverse theorem is asserted.
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
        (
            Term::NominalValue {
                ty: left,
                variant: a,
                fields: af,
            },
            Term::NominalValue {
                ty: right,
                variant: b,
                fields: bf,
            },
        ) if a == b && af.len() == bf.len() => {
            // Immutable payloads can propose ordinary input substitutions.
            // Keep their complete domain and enclosing equality obligation:
            // field inspection is not an inverse theorem for an opaque head.
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
            for (left, right) in af.iter().zip(bf) {
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
        (Term::Tuple(a), Term::Tuple(b)) | (Term::TupleValue(a), Term::TupleValue(b))
            if a.len() == b.len() =>
        {
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
