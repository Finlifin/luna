//! Compile candidate-local pattern control onto the expression worklist.
//! Structural leaves are admitted before any invocation code is published.
use super::*;
use nir::program::Pattern as P;
use std::collections::BTreeSet;

/// Flatten sequential pattern computation onto the ordinary expression worklist.
/// Value and Type structures expose one field at a time. A failure jumps to the candidate's
/// existing scope rollback before any later field observation or computation.
pub(super) fn lower<'a>(
    pattern: &'a P,
    subject: Register,
    condition: Register,
    failed: usize,
    locals: &mut BTreeMap<LocalId, Register>,
    fresh: &mut impl FnMut() -> Register,
    labels: &mut Vec<Option<usize>>,
    fuel: &mut usize,
    source: SourceRange,
) -> Result<Vec<Work<'a>>, Error> {
    enum Task<'a> {
        Pattern(&'a P, Register, usize),
        Code(Work<'a>),
        Expression(&'a Expr, Register),
        Guard(&'a Expr, usize),
    }
    let mut pending = vec![Task::Pattern(pattern, subject, failed)];
    let mut code = Vec::new();
    while let Some(task) = pending.pop() {
        *fuel = fuel.checked_sub(1).ok_or(Error::Budget(source))?;
        if let Task::Pattern(pattern, input, failed) = &task {
            if let Some((shape, fields)) = nir::pattern_structure::View::from_pattern(pattern) {
                *fuel = fuel
                    .checked_sub(fields.len().saturating_add(1))
                    .ok_or(Error::Budget(source))?;
                match shape {
                    nir::pattern_structure::View::AnonymousRecord { names, .. } => {
                        for name in names {
                            *fuel = fuel
                                .checked_sub(name.len().saturating_add(1))
                                .ok_or(Error::Budget(source))?;
                        }
                    }
                    nir::pattern_structure::View::ErrorResult { label: Some(label) } => {
                        nir::type_computations::charge_term(label, fuel)
                            .map_err(|_| Error::Budget(source))?;
                    }
                    _ => {}
                }
                let shape = Arc::new(shape.to_owned());
                for (index, field) in fields.iter().enumerate().rev() {
                    let destination = if matches!(field, P::Wild) {
                        None
                    } else {
                        Some(fresh())
                    };
                    if let Some(destination) = destination {
                        pending.push(Task::Pattern(field, destination, *failed));
                    }
                    pending.push(Task::Code(Work::Emit(
                        source,
                        Op::PatternStructureField {
                            destination,
                            value: *input,
                            shape: shape.clone(),
                            index,
                        },
                    )));
                }
                code.push(Work::Emit(
                    source,
                    Op::CheckPatternStructure {
                        destination: condition,
                        value: *input,
                        shape,
                    },
                ));
                code.push(Work::Emit(
                    source,
                    Op::Branch {
                        condition,
                        when: false,
                        target: *failed,
                    },
                ));
                continue;
            }
        }
        if let Task::Pattern(pattern, input, failed) = &task {
            if let Some((shape, fields)) =
                nir::pattern_structure::TypeStructure::from_pattern(pattern)
            {
                *fuel = fuel
                    .checked_sub(fields.len().saturating_add(1))
                    .ok_or(Error::Budget(source))?;
                let identity = fresh();
                pending.push(Task::Code(Work::Emit(
                    source,
                    Op::FinishTypePattern { identity },
                )));
                for (index, field) in fields.iter().enumerate().rev() {
                    let destination = if matches!(field, P::Wild) {
                        None
                    } else {
                        Some(fresh())
                    };
                    if let Some(destination) = destination {
                        pending.push(Task::Pattern(field, destination, *failed));
                    }
                    pending.push(Task::Code(Work::Emit(
                        source,
                        Op::TypePatternField {
                            identity,
                            destination,
                            value: *input,
                            index,
                        },
                    )));
                }
                code.push(Work::Emit(
                    source,
                    Op::BeginTypePattern {
                        identity,
                        destination: condition,
                        value: *input,
                        shape,
                    },
                ));
                code.push(Work::Emit(
                    source,
                    Op::Branch {
                        condition,
                        when: false,
                        target: *failed,
                    },
                ));
                continue;
            }
        }
        match task {
            Task::Pattern(P::Then { first, value, next }, input, failed) => {
                let transformed = fresh();
                pending.push(Task::Pattern(next, transformed, failed));
                pending.push(Task::Expression(value, transformed));
                pending.push(Task::Pattern(first, input, failed));
            }
            Task::Pattern(
                P::Guard {
                    pattern,
                    condition: guard,
                },
                input,
                failed,
            ) => {
                pending.push(Task::Guard(guard, failed));
                pending.push(Task::Pattern(pattern, input, failed));
            }
            Task::Code(step) => code.push(step),
            Task::Pattern(P::Binding { binding, inner }, input, failed) => {
                let local = *locals.entry(*binding).or_insert_with(&mut *fresh);
                pending.push(Task::Code(Work::Emit(
                    source,
                    Op::BindPattern {
                        local,
                        value: input,
                        observe: false,
                    },
                )));
                pending.push(Task::Pattern(inner, input, failed));
            }
            Task::Pattern(P::Observe { .. } | P::Remap { .. }, input, failed) => {
                let Task::Pattern(pattern, _, _) = task else {
                    unreachable!()
                };
                let rollback = labels.len();
                labels.push(None);
                let done = labels.len();
                labels.push(None);
                pending.push(Task::Code(Work::Label(done)));
                pending.push(Task::Code(Work::Emit(source, Op::Jump(failed))));
                pending.push(Task::Code(Work::EndScope(source)));
                pending.push(Task::Code(Work::Label(rollback)));
                pending.push(Task::Code(Work::Emit(source, Op::Jump(done))));
                pending.push(Task::Code(Work::Emit(source, Op::CommitScope)));
                match pattern {
                    P::Observe { binding, inner } => {
                        let local = *locals.entry(*binding).or_insert_with(&mut *fresh);
                        pending.push(Task::Code(Work::Emit(
                            source,
                            Op::ClearPatternAlias { local },
                        )));
                        pending.push(Task::Pattern(inner, input, rollback));
                        pending.push(Task::Code(Work::Emit(
                            source,
                            Op::BindPattern {
                                local,
                                value: input,
                                observe: true,
                            },
                        )));
                        pending.push(Task::Code(Work::BeginScope(source)));
                    }
                    P::Remap { pattern, bindings } => {
                        *fuel = fuel
                            .checked_sub(bindings.len())
                            .ok_or(Error::Budget(source))?;
                        let pairs = Arc::new(
                            bindings
                                .iter()
                                .map(|(from, to)| {
                                    let from = *locals.entry(*from).or_insert_with(&mut *fresh);
                                    let to = *locals.entry(*to).or_insert_with(&mut *fresh);
                                    (from, to)
                                })
                                .collect(),
                        );
                        pending.push(Task::Code(Work::Emit(
                            source,
                            Op::RemapPattern(Arc::clone(&pairs)),
                        )));
                        pending.push(Task::Pattern(pattern, input, rollback));
                        pending.push(Task::Code(Work::BeginScope(source)));
                        pending.push(Task::Code(Work::Emit(source, Op::CheckPatternRemap(pairs))));
                    }
                    _ => unreachable!(),
                }
            }
            Task::Pattern(P::Or(alternatives), input, failed) => {
                *fuel = fuel
                    .checked_sub(alternatives.len())
                    .ok_or(Error::Budget(source))?;
                let done = labels.len();
                labels.push(None);
                pending.push(Task::Code(Work::Label(done)));
                pending.push(Task::Code(Work::Emit(source, Op::Jump(failed))));
                for alternative in alternatives.iter().rev() {
                    let next = labels.len();
                    labels.push(None);
                    pending.push(Task::Code(Work::EndScope(source)));
                    pending.push(Task::Code(Work::Label(next)));
                    pending.push(Task::Code(Work::Emit(source, Op::Jump(done))));
                    pending.push(Task::Code(Work::Emit(source, Op::CommitScope)));
                    pending.push(Task::Pattern(alternative, input, next));
                    pending.push(Task::Code(Work::BeginScope(source)));
                }
            }
            Task::Expression(value, destination) => code.push(Work::Expression(value, destination)),
            Task::Guard(guard, failed) => {
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
            Task::Pattern(pattern, value, failed) => {
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
