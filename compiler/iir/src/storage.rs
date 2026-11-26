//! Storage-phase validation in concrete runtime bodies, retaining staged inputs.
use nir::program::{Expr, ExprKind as K, Function, FunctionPhase, Package};
use nir::{OrderedTerm, Sort, Term};
use std::collections::BTreeSet;

pub(super) fn validate(
    function: &Function,
    metadata: &Package,
    resolve: &dyn Fn(&Term, &mut usize) -> Result<Vec<Term>, nir::phases::StorageError>,
    validated_closed: &mut BTreeSet<OrderedTerm>,
    fuel: &mut usize,
) -> Result<(), String> {
    if function.phase != FunctionPhase::Runtime {
        return Ok(());
    }
    let Term::Function { parameters, .. } = function.signature.callable_signature() else {
        return Err("IIR runtime function has no callable signature".into());
    };
    let frames = vec![parameters.iter().map(|p| p.sort).collect()];
    let mut validated_in_fn = BTreeSet::<OrderedTerm>::new();
    for env in &function.env_parameters {
        check_storage(
            &env.ty,
            metadata,
            &frames,
            resolve,
            validated_closed,
            &mut validated_in_fn,
            fuel,
        )
        .map_err(|message| {
            format!(
                "IIR runtime environment storage in {} at {}: {message}",
                function.name, function.source
            )
        })?;
    }
    let mut roots = vec![&function.body];
    roots.extend(&function.preconditions);
    roots.extend(&function.postconditions);
    roots.extend(function.runtime_deliveries.values());
    for (index, value) in &function.defaults {
        let parameter = parameters
            .get(*index)
            .ok_or("IIR default parameter is missing")?;
        if parameter.sort == Sort::Runtime {
            roots.push(value);
        }
    }
    validate_expressions_with_local_cache(
        roots,
        &frames,
        metadata,
        resolve,
        validated_closed,
        &mut validated_in_fn,
        &function.name,
        fuel,
    )
}

pub(super) fn validate_expressions(
    roots: Vec<&Expr>,
    frames: &[Vec<Sort>],
    metadata: &Package,
    resolve: &dyn Fn(&Term, &mut usize) -> Result<Vec<Term>, nir::phases::StorageError>,
    validated_closed: &mut BTreeSet<OrderedTerm>,
    owner_name: &str,
    fuel: &mut usize,
) -> Result<(), String> {
    let mut validated_in_owner = BTreeSet::<OrderedTerm>::new();
    validate_expressions_with_local_cache(
        roots,
        frames,
        metadata,
        resolve,
        validated_closed,
        &mut validated_in_owner,
        owner_name,
        fuel,
    )
}

fn check_storage(
    ty: &Term,
    metadata: &Package,
    frames: &[Vec<Sort>],
    resolve: &dyn Fn(&Term, &mut usize) -> Result<Vec<Term>, nir::phases::StorageError>,
    validated_closed: &mut BTreeSet<OrderedTerm>,
    validated_in_owner: &mut BTreeSet<OrderedTerm>,
    fuel: &mut usize,
) -> Result<(), nir::phases::StorageError> {
    let key = OrderedTerm(ty.clone());
    if validated_closed.contains(&key) || validated_in_owner.contains(&key) {
        *fuel = fuel
            .checked_sub(1)
            .ok_or(nir::phases::StorageError::Budget)?;
        return Ok(());
    }
    nir::phases::validate_runtime_storage_with_nominals(ty, metadata, frames, resolve, fuel)?;
    if ty.check_scope(&[]).is_ok() {
        validated_closed.insert(key);
    } else {
        validated_in_owner.insert(key);
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn validate_expressions_with_local_cache(
    mut roots: Vec<&Expr>,
    frames: &[Vec<Sort>],
    metadata: &Package,
    resolve: &dyn Fn(&Term, &mut usize) -> Result<Vec<Term>, nir::phases::StorageError>,
    validated_closed: &mut BTreeSet<OrderedTerm>,
    validated_in_owner: &mut BTreeSet<OrderedTerm>,
    owner_name: &str,
    fuel: &mut usize,
) -> Result<(), String> {
    while let Some(root) = roots.pop() {
        nir::visit::expression_pruned(root, fuel, |expression, fuel| {
            check_storage(
                &expression.ty,
                metadata,
                frames,
                resolve,
                validated_closed,
                validated_in_owner,
                fuel,
            )
            .map_err(|message| {
                format!(
                    "IIR runtime storage in {} at {}: {message}",
                    owner_name, expression.source
                )
            })?;
            match &expression.kind {
                K::Call {
                    callee,
                    arguments,
                    env_arguments,
                    ..
                } => {
                    let Term::Function { parameters, .. } = callee.ty.callable_signature() else {
                        return Err("IIR call has no callable signature".into());
                    };
                    roots.push(callee);
                    for argument in arguments {
                        let parameter = parameters
                            .get(argument.parameter)
                            .ok_or("IIR call argument parameter is missing")?;
                        // A staged argument is checked in its own execution phase.
                        // It is not allocated as runtime storage by this call.
                        if parameter.sort == Sort::Runtime {
                            roots.push(&argument.value);
                        }
                    }
                    roots.extend(env_arguments.iter());
                    Ok(false)
                }
                K::TypeEqual(left, right)
                    if matches!(left.kind, K::TypeValue(_))
                        && matches!(right.kind, K::TypeValue(_)) =>
                {
                    // Immutable descriptions are operands of type observation,
                    // not runtime locals. Their terms are checked by IIR typing.
                    Ok(false)
                }
                K::TypeKind { operand, .. } | K::TypeNotSized { operand }
                    if matches!(operand.kind, K::TypeValue(_)) =>
                {
                    // Immutable descriptions are operands of type observation,
                    // not runtime locals. Their terms are checked by IIR typing.
                    Ok(false)
                }
                K::TypeLayout {
                    query: nir::TypeLayoutQuery::FieldOffset,
                    operand,
                } if matches!(
                    &operand.kind,
                    K::Tuple(items) if items.len() == 2 && matches!(items[0].kind, K::TypeValue(_))
                ) =>
                {
                    if let K::Tuple(items) = &operand.kind
                        && !matches!(items[1].kind, K::Literal(_))
                    {
                        roots.push(&items[1]);
                    }
                    Ok(false)
                }
                K::TypeLayout { operand, .. } if matches!(operand.kind, K::TypeValue(_)) => {
                    // Immutable descriptions are operands of type observation,
                    // not runtime locals. Their terms are checked by IIR typing.
                    Ok(false)
                }
                _ => Ok(true),
            }
        })?;
    }
    Ok(())
}
