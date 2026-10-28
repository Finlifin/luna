//! Specialize known alternatives without changing runtime selection order.
use super::*;

// Recognize data, not arbitrary pure expressions: duplicating a producer could
// change required invocation counts or failures. Canonical representation and
// type consistency are still validated by the ordinary staged-input binder.
fn inert_input(expression: &n::Expr, fuel: &mut usize) -> Result<bool, String> {
    let mut inert = true;
    iir::visit::expression(expression, fuel, |node, _| {
        inert &= matches!(
            node.kind,
            K::TypeValue(_)
                | K::TypeListValue(_)
                | K::Literal(_)
                | K::Unit
                | K::Tuple(_)
                | K::AnonymousRecord { .. }
                | K::Record { .. }
                | K::Variant { .. }
                | K::NewtypeConvert(_)
                | K::TypeListValue(_)
                | K::Function(_)
                | K::FunctionInstance { .. }
        );
        Ok(())
    })?;
    Ok(inert)
}

pub(super) fn scoped_targets_supported(callee: &n::Expr, fuel: &mut usize) -> Result<bool, String> {
    let mut pending = vec![callee];
    while let Some(value) = pending.pop() {
        *fuel = fuel
            .checked_sub(1)
            .ok_or("callable selection inspection budget exhausted")?;
        if value.ty == Term::Primitive(nir::Primitive::Never) {
            continue;
        }
        match &value.kind {
            K::Function(_) | K::FunctionInstance { .. } | K::TraitMethod { .. } => {}
            K::Block {
                result: Some(result),
                ..
            }
            | K::CallPreparation {
                result: Some(result),
                ..
            } => pending.push(result),
            K::If {
                then_branch,
                else_branch: Some(other),
                ..
            } => {
                pending.push(then_branch);
                pending.push(other);
            }
            K::Match { arms, .. } if !arms.is_empty() => {
                pending.extend(arms.iter().map(|arm| &arm.body))
            }
            _ => return Ok(false),
        }
    }
    Ok(true)
}

/// Complete each source input once, before duplicating a runtime selection.
/// Neither selection conditions nor ordinary runtime arguments are invoked.
pub(super) fn complete_inputs<C>(
    expression: &mut n::Expr,
    context: &mut C,
    fuel: &mut usize,
    mut invoke: impl FnMut(&mut C, &n::Expr, &mut usize) -> Result<n::Expr, String>,
) -> Result<(), String> {
    let K::Call {
        callee,
        arguments,
        parameter_bindings,
    } = &expression.kind
    else {
        return Ok(());
    };
    if !matches!(
        callee.kind,
        K::If { .. } | K::Match { .. } | K::Block { .. } | K::CallPreparation { .. }
    ) || !parameter_bindings.is_empty()
        || arguments.iter().any(|a| a.is_default)
    {
        return Ok(());
    }
    let Term::Function { parameters, .. } = callee.ty.callable_signature() else {
        return Ok(());
    };
    if !parameters.iter().any(|p| p.sort != nir::Sort::Runtime)
        || !scoped_targets_supported(callee, fuel)?
    {
        return Ok(());
    }
    let mut completed = Vec::new();
    for (index, argument) in arguments.iter().enumerate() {
        let parameter = parameters
            .get(argument.parameter)
            .ok_or("conditional staged argument index is invalid")?;
        if parameter.sort == nir::Sort::Runtime || inert_input(&argument.value, fuel)? {
            continue;
        }
        let value = invoke(context, &argument.value, fuel)?;
        if value.ty != argument.value.ty || !inert_input(&value, fuel)? {
            return Err(
                "conditional staged input did not freeze as immutable data of its checked type"
                    .into(),
            );
        }
        completed.push((index, value));
    }
    let K::Call { arguments, .. } = &mut expression.kind else {
        unreachable!()
    };
    for (index, value) in completed {
        arguments[index].value = value;
    }
    Ok(())
}

fn target(expression: &n::Expr) -> bool {
    // The caller has already charged a full expression traversal. Use an
    // explicit stack so deeply nested selection does not recurse on the host.
    let mut pending = vec![expression];
    while let Some(expression) = pending.pop() {
        match &expression.kind {
            K::Block {
                statements,
                result: Some(result),
            } if statements.is_empty() => pending.push(result),
            K::If {
                then_branch,
                else_branch: Some(else_branch),
                ..
            } => {
                pending.push(then_branch);
                pending.push(else_branch);
            }
            K::Function(_) | K::FunctionInstance { .. } | K::TraitMethod { .. } => {}
            _ => return false,
        }
    }
    true
}

fn unwrap_target(mut expression: Box<n::Expr>) -> Box<n::Expr> {
    loop {
        match expression.kind {
            K::Block {
                result: Some(result),
                ..
            } => expression = result,
            _ => return expression,
        }
    }
}

impl Builder<'_> {
    pub(super) fn resolve_conditional_callable(
        expression: &mut n::Expr,
        include_runtime: bool,
        next: &mut u32,
        sources: &mut Vec<n::ExpansionSource>,
        fuel: &mut usize,
    ) -> Result<(), String> {
        let K::Call {
            callee,
            arguments,
            parameter_bindings,
        } = &expression.kind
        else {
            return Ok(());
        };
        if matches!(
            callee.kind,
            K::Match { .. } | K::Block { .. } | K::CallPreparation { .. }
        ) {
            return resolve_scoped_callable(expression, include_runtime, next, sources, fuel);
        }
        let K::If {
            then_branch,
            else_branch: Some(else_branch),
            ..
        } = &callee.kind
        else {
            return Ok(());
        };
        let Term::Function { parameters, .. } = callee.ty.callable_signature() else {
            return Ok(());
        };
        if (!include_runtime && !parameters.iter().any(|p| p.sort != nir::Sort::Runtime))
            || !parameter_bindings.is_empty()
            || arguments.iter().any(|a| a.is_default)
        {
            return Ok(());
        }
        // Only immutable, already-completed data inputs may be duplicated into
        // alternative instances. Computed staged producers must not execute for
        // the unselected target. Runtime arguments execute in the chosen arm.
        for argument in arguments {
            let Some(parameter) = parameters.get(argument.parameter) else {
                return Err("conditional callable argument index is invalid".into());
            };
            if parameter.sort != nir::Sort::Runtime && !inert_input(&argument.value, fuel)? {
                return Ok(());
            }
        }
        // Charge traversal before looking through scope wrappers or copying.
        iir::visit::expression(expression, fuel, |expr, fuel| {
            nir::type_computations::charge_term(&expr.ty, fuel).map_err(|e| e.to_string())
        })?;
        if !target(then_branch) || !target(else_branch) {
            return resolve_scoped_callable(expression, include_runtime, next, sources, fuel);
        }
        crate::associated::charge_projection_copy(&expression.ty, fuel)?;
        let mut copied = Box::new(n::Expr {
            source: expression.source,
            ty: expression.ty.clone(),
            kind: K::Call {
                callee: unwrap_target(then_branch.clone()),
                arguments: arguments.clone(),
                parameter_bindings: parameter_bindings.clone(),
            },
        });
        let source_end = sources.len();
        let previous_next = *next;
        if let Err(error) = nir::freshening::copied_expression(&mut copied, next, sources, fuel) {
            sources.truncate(source_end);
            *next = previous_next;
            return Err(error);
        }
        let K::Call {
            callee,
            arguments,
            parameter_bindings,
        } = std::mem::replace(&mut expression.kind, K::Unit)
        else {
            unreachable!()
        };
        let K::If {
            condition,
            else_branch: Some(else_branch),
            ..
        } = callee.kind
        else {
            unreachable!()
        };
        let branch = |callee, arguments, parameter_bindings| {
            Box::new(n::Expr {
                source: expression.source,
                ty: expression.ty.clone(),
                kind: K::Call {
                    callee: unwrap_target(callee),
                    arguments,
                    parameter_bindings,
                },
            })
        };
        expression.kind = K::If {
            condition,
            then_branch: copied,
            else_branch: Some(branch(else_branch, arguments, parameter_bindings)),
        };
        Ok(())
    }
}

// A selector's subject and branch locals must be retired before ordinary arguments
// begin. Moving the call directly into each arm would extend their lifetimes.
// First compute a private target tag using the original selection tree; then
// dispatch the call outside that tree's resource scopes.
fn resolve_scoped_callable(
    expression: &mut n::Expr,
    include_runtime: bool,
    next: &mut u32,
    sources: &mut Vec<n::ExpansionSource>,
    fuel: &mut usize,
) -> Result<(), String> {
    let K::Call {
        callee,
        arguments,
        parameter_bindings,
    } = &expression.kind
    else {
        unreachable!()
    };
    let Term::Function { parameters, .. } = callee.ty.callable_signature() else {
        return Ok(());
    };
    if (!include_runtime && !parameters.iter().any(|p| p.sort != nir::Sort::Runtime))
        || !parameter_bindings.is_empty()
        || arguments.iter().any(|a| a.is_default)
    {
        return Ok(());
    }
    for argument in arguments {
        let parameter = parameters
            .get(argument.parameter)
            .ok_or("matched callable argument index is invalid")?;
        if parameter.sort != nir::Sort::Runtime && !inert_input(&argument.value, fuel)? {
            return Ok(());
        }
    }
    iir::visit::expression(expression, fuel, |expr, fuel| {
        nir::type_computations::charge_term(&expr.ty, fuel).map_err(|e| e.to_string())
    })?;
    if !scoped_targets_supported(callee, fuel)? {
        return Ok(());
    }
    let source_end = sources.len();
    let previous_next = *next;
    let result = (|| {
        let tag_type = Term::Primitive(nir::Primitive::U64);
        let tag = n::LocalId {
            owner: u32::MAX,
            index: *next,
        };
        *next = next
            .checked_add(1)
            .ok_or("matched callable identity space exhausted")?;
        let mut selection = callee.as_ref().clone();
        let mut targets = Vec::new();
        tag_targets(&mut selection, &mut targets, &tag_type, fuel, 0)?;
        let last = targets
            .len()
            .checked_sub(1)
            .ok_or("matched callable has no completing target")?;
        let mut arms = Vec::new();
        for (index, target) in targets.into_iter().enumerate() {
            for argument in arguments {
                iir::visit::expression(&argument.value, fuel, |expr, fuel| {
                    nir::type_computations::charge_term(&expr.ty, fuel).map_err(|e| e.to_string())
                })?;
            }
            let mut body = n::Expr {
                source: expression.source,
                ty: expression.ty.clone(),
                kind: K::Call {
                    callee: Box::new(target),
                    arguments: arguments.clone(),
                    parameter_bindings: vec![],
                },
            };
            nir::freshening::copied_expression(&mut body, next, sources, fuel)?;
            arms.push(n::MatchArm {
                pattern: if index == last {
                    P::Wild
                } else {
                    P::Literal(n::Literal::UnsignedInteger(index as u128))
                },
                guard: None,
                body,
            });
        }
        *fuel = fuel
            .checked_sub(4)
            .ok_or("matched callable source budget exhausted")?;
        let mut fresh_source = || -> Result<n::SourceRange, String> {
            let index = u32::try_from(sources.len())
                .map_err(|_| "matched callable source space exhausted")?;
            sources.push(n::ExpansionSource {
                original: expression.source,
                site: expression.source,
            });
            Ok(n::SourceRange::generated(index))
        };
        let block_source = fresh_source()?;
        let binding_source = fresh_source()?;
        let match_source = fresh_source()?;
        let tag_source = fresh_source()?;
        let mut candidate = n::Expr {
            source: block_source,
            ty: expression.ty.clone(),
            kind: K::Block {
                statements: vec![n::Expr {
                    source: binding_source,
                    ty: Term::Primitive(nir::Primitive::Void),
                    kind: K::Let {
                        binding: tag,
                        value: Box::new(selection),
                    },
                }],
                result: Some(Box::new(n::Expr {
                    source: match_source,
                    ty: expression.ty.clone(),
                    kind: K::Match {
                        value: Box::new(n::Expr {
                            source: tag_source,
                            ty: tag_type,
                            kind: K::Local(tag),
                        }),
                        arms,
                    },
                })),
            },
        };
        nir::freshening::copied_expression(&mut candidate, next, sources, fuel)?;
        Ok(candidate)
    })();
    match result {
        Ok(candidate) => {
            *expression = candidate;
            Ok(())
        }
        Err(error) => {
            sources.truncate(source_end);
            *next = previous_next;
            Err(error)
        }
    }
}

pub(super) fn tag_targets(
    value: &mut n::Expr,
    targets: &mut Vec<n::Expr>,
    tag_type: &Term,
    fuel: &mut usize,
    depth: usize,
) -> Result<(), String> {
    *fuel = fuel
        .checked_sub(1)
        .ok_or("matched callable selection budget exhausted")?;
    if depth >= 128 {
        return Err("matched callable selection depth exhausted".into());
    }
    if value.ty == Term::Primitive(nir::Primitive::Never) {
        return Ok(());
    }
    match &mut value.kind {
        K::Block {
            result: Some(result),
            ..
        }
        | K::CallPreparation {
            result: Some(result),
            ..
        } => tag_targets(result, targets, tag_type, fuel, depth + 1)?,
        K::If {
            then_branch,
            else_branch: Some(other),
            ..
        } => {
            tag_targets(then_branch, targets, tag_type, fuel, depth + 1)?;
            tag_targets(other, targets, tag_type, fuel, depth + 1)?;
        }
        K::Match { arms, .. } => {
            for arm in arms {
                tag_targets(&mut arm.body, targets, tag_type, fuel, depth + 1)?;
            }
        }
        K::Function(_) | K::FunctionInstance { .. } | K::TraitMethod { .. } => {
            let tag = n::Expr {
                source: value.source,
                ty: tag_type.clone(),
                kind: K::Literal(n::Literal::UnsignedInteger(targets.len() as u128)),
            };
            targets.push(std::mem::replace(value, tag));
        }
        _ => unreachable!(),
    }
    value.ty = tag_type.clone();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inert_inputs_reject_nested_producers_and_charge_budget() {
        let boolean = Term::Primitive(nir::Primitive::Bool);
        let source = n::SourceRange { start: 0, end: 1 };
        let value = n::Expr {
            source,
            ty: boolean.clone(),
            kind: K::Literal(n::Literal::Bool(true)),
        };
        let mut tuple = n::Expr {
            source,
            ty: Term::Tuple(vec![boolean.clone()]),
            kind: K::Tuple(vec![value.clone()]),
        };
        assert!(inert_input(&tuple, &mut 100).unwrap());
        assert!(inert_input(&tuple, &mut 0).is_err());
        tuple.kind = K::Tuple(vec![n::Expr {
            source,
            ty: boolean.clone(),
            kind: K::Call {
                callee: Box::new(n::Expr {
                    source,
                    ty: Term::Function {
                        parameters: vec![],
                        result: Box::new(boolean),
                    },
                    kind: K::Function(Id {
                        package: 0,
                        local: 1,
                    }),
                }),
                arguments: vec![],
                parameter_bindings: vec![],
            },
        }]);
        let mut fuel = 100;
        assert!(!inert_input(&tuple, &mut fuel).unwrap());
        assert!(fuel < 100);
    }
    #[test]
    fn computed_inputs_execute_once_in_source_order_without_running_selection_or_runtime_inputs() {
        let boolean = Term::Primitive(nir::Primitive::Bool);
        let producer = |id: u32| n::Expr {
            source: n::SourceRange {
                start: id,
                end: id + 1,
            },
            ty: boolean.clone(),
            kind: K::Call {
                callee: Box::new(n::Expr {
                    source: n::SourceRange {
                        start: id,
                        end: id + 1,
                    },
                    ty: Term::Function {
                        parameters: vec![],
                        result: Box::new(boolean.clone()),
                    },
                    kind: K::Function(Id {
                        package: 0,
                        local: id,
                    }),
                }),
                arguments: vec![],
                parameter_bindings: vec![],
            },
        };
        let signature = Term::Function {
            parameters: vec![
                nir::Parameter {
                    sort: nir::Sort::ComptimeValue,
                    ty: boolean.clone(),
                },
                nir::Parameter {
                    sort: nir::Sort::Runtime,
                    ty: boolean.clone(),
                },
                nir::Parameter {
                    sort: nir::Sort::ComptimeValue,
                    ty: boolean.clone(),
                },
            ],
            result: Box::new(boolean.clone()),
        };
        let target = n::Expr {
            source: n::SourceRange { start: 50, end: 51 },
            ty: signature.clone(),
            kind: K::Function(Id {
                package: 0,
                local: 50,
            }),
        };
        let original = n::Expr {
            source: n::SourceRange { start: 0, end: 100 },
            ty: boolean.clone(),
            kind: K::Call {
                callee: Box::new(n::Expr {
                    source: n::SourceRange { start: 50, end: 60 },
                    ty: signature,
                    kind: K::If {
                        condition: Box::new(producer(9)),
                        then_branch: Box::new(target.clone()),
                        else_branch: Some(Box::new(target)),
                    },
                }),
                arguments: [2, 1, 0]
                    .into_iter()
                    .map(|parameter| n::CallArgument {
                        passing: Default::default(),
                        parameter,
                        is_default: false,
                        value: producer(parameter as u32),
                    })
                    .collect(),
                parameter_bindings: vec![],
            },
        };
        let mut calls = Vec::new();
        let mut candidate = original.clone();
        complete_inputs(&mut candidate, &mut calls, &mut 1000, |calls, input, _| {
            calls.push(input.source.start);
            if input.source.start == 0 {
                return Err("producer failed".into());
            }
            Ok(n::Expr {
                kind: K::Literal(n::Literal::Bool(true)),
                ..input.clone()
            })
        })
        .unwrap_err();
        assert_eq!(calls, [2, 0]);
        assert_eq!(candidate, original, "failed preparation published a prefix");
        calls.clear();
        for _ in 0..2 {
            complete_inputs(&mut candidate, &mut calls, &mut 1000, |calls, input, _| {
                calls.push(input.source.start);
                Ok(n::Expr {
                    kind: K::Literal(n::Literal::Bool(true)),
                    ..input.clone()
                })
            })
            .unwrap();
        }
        assert_eq!(calls, [2, 0]);
        Builder::resolve_conditional_callable(
            &mut candidate,
            false,
            &mut 100,
            &mut Vec::new(),
            &mut 10_000,
        )
        .unwrap();
        let mut producer_ids = Vec::new();
        iir::visit::expression(&candidate, &mut 1000, |expr, _| {
            if let K::Call { callee, .. } = &expr.kind {
                if let K::Function(id) = callee.kind {
                    producer_ids.push(id.local);
                }
            }
            Ok(())
        })
        .unwrap();
        assert_eq!(producer_ids.iter().filter(|&&id| id == 9).count(), 1);
        assert_eq!(producer_ids.iter().filter(|&&id| id == 1).count(), 2);
        assert!(!producer_ids.contains(&0) && !producer_ids.contains(&2));
    }
}

// Evaluate a template selection at its original binding point. Only the tag
// survives at runtime; later calls dispatch to statically specialized targets.
pub(super) fn freeze_template_selection(
    value: &n::Expr,
    next: &mut u32,
    sources: &mut Vec<n::ExpansionSource>,
    fuel: &mut usize,
) -> Result<Option<(n::Expr, n::Expr)>, String> {
    if !scoped_targets_supported(value, fuel)? {
        return Ok(None);
    }
    let mut remaining = *fuel as u64;
    nir::copy_budget::charge_expression(value, &mut remaining)
        .map_err(|_| "template selection copy budget exhausted")?;
    *fuel = remaining as usize;
    let tag_type = Term::Primitive(nir::Primitive::U64);
    let tag = n::LocalId {
        owner: u32::MAX,
        index: *next,
    };
    *next = next
        .checked_add(1)
        .ok_or("template selection identity space exhausted")?;
    let mut selection = value.clone();
    let mut targets = Vec::new();
    tag_targets(&mut selection, &mut targets, &tag_type, fuel, 0)?;
    let Some(last) = targets.len().checked_sub(1) else {
        return Ok(None);
    };
    let index =
        u32::try_from(sources.len()).map_err(|_| "template selection source space exhausted")?;
    sources.push(n::ExpansionSource {
        original: value.source,
        site: value.source,
    });
    let binding_source = n::SourceRange::generated(index);
    let dispatch = n::Expr {
        source: value.source,
        ty: value.ty.clone(),
        kind: K::Match {
            value: Box::new(n::Expr {
                source: value.source,
                ty: tag_type,
                kind: K::Local(tag),
            }),
            arms: targets
                .into_iter()
                .enumerate()
                .map(|(index, body)| n::MatchArm {
                    pattern: if index == last {
                        P::Wild
                    } else {
                        P::Literal(n::Literal::UnsignedInteger(index as u128))
                    },
                    guard: None,
                    body,
                })
                .collect(),
        },
    };
    Ok(Some((
        n::Expr {
            source: binding_source,
            ty: Term::Primitive(nir::Primitive::Void),
            kind: K::Let {
                binding: tag,
                value: Box::new(selection),
            },
        },
        dispatch,
    )))
}
