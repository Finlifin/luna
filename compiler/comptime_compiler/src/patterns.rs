//! Admit structural patterns before publishing any invocation code. Patterns
//! Sequential computations use the expression worklist and invocation continuations.
use super::*;
use nir::program::Pattern as P;
use std::collections::BTreeSet;

/// Flatten sequential pattern computation onto the ordinary expression worklist.
/// Structural subpatterns remain atomic; a failure jumps to the candidate's
/// existing scope rollback before any later computation can execute.
pub(super) fn lower<'a>(
    pattern: &'a P,
    subject: Register,
    condition: Register,
    failed: usize,
    locals: &mut BTreeMap<LocalId, Register>,
    fresh: &mut impl FnMut() -> Register,
    fuel: &mut usize,
    source: SourceRange,
) -> Result<Vec<Work<'a>>, Error> {
    enum Task<'a> {
        Pattern(&'a P, Register),
        Expression(&'a Expr, Register),
        Guard(&'a Expr),
    }
    let mut pending = vec![Task::Pattern(pattern, subject)];
    let mut code = Vec::new();
    while let Some(task) = pending.pop() {
        *fuel = fuel.checked_sub(1).ok_or(Error::Budget(source))?;
        match task {
            Task::Pattern(P::Then { first, value, next }, input) => {
                let transformed = fresh();
                pending.push(Task::Pattern(next, transformed));
                pending.push(Task::Expression(value, transformed));
                pending.push(Task::Pattern(first, input));
            }
            Task::Pattern(
                P::Guard {
                    pattern,
                    condition: guard,
                },
                input,
            ) => {
                pending.push(Task::Guard(guard));
                pending.push(Task::Pattern(pattern, input));
            }
            Task::Expression(value, destination) => code.push(Work::Expression(value, destination)),
            Task::Guard(guard) => {
                code.push(Work::Expression(guard, condition));
                code.push(Work::Emit(
                    guard.source,
                    Op::Branch {
                        condition,
                        when: false,
                        target: failed,
                    },
                ));
            }
            Task::Pattern(pattern, value) => {
                let (pattern, bindings) = copy(pattern, fuel, source)?;
                let bindings = bindings
                    .into_iter()
                    .map(|id| (id, *locals.entry(id).or_insert_with(&mut *fresh)))
                    .collect();
                code.push(Work::Emit(
                    source,
                    Op::MatchPattern {
                        destination: condition,
                        value,
                        pattern,
                        bindings,
                    },
                ));
                code.push(Work::Emit(
                    source,
                    Op::Branch {
                        condition,
                        when: false,
                        target: failed,
                    },
                ));
            }
        }
    }
    Ok(code)
}

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
