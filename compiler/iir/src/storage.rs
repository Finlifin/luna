//! Storage-phase validation in concrete runtime bodies, retaining staged inputs.
use nir::program::{ExprKind as K, Function, FunctionPhase, Package};
use nir::{Sort, Term};
use std::collections::HashSet;

pub(super) fn validate(
    function: &Function,
    metadata: &Package,
    resolve: &dyn Fn(&Term, &mut usize) -> Result<Vec<Term>, nir::phases::StorageError>,
    validated_closed: &mut HashSet<Term>,
    fuel: &mut usize,
) -> Result<(), String> {
    if function.phase != FunctionPhase::Runtime {
        return Ok(());
    }
    let Term::Function { parameters, .. } = function.signature.callable_signature() else {
        return Err("IIR runtime function has no callable signature".into());
    };
    let frames = vec![parameters.iter().map(|p| p.sort).collect()];
    let mut validated_in_fn = HashSet::<Term>::new();
    let mut check_storage = |ty: &Term,
                             fuel: &mut usize|
     -> Result<(), nir::phases::StorageError> {
        if validated_closed.contains(ty) || validated_in_fn.contains(ty) {
            *fuel = fuel
                .checked_sub(1)
                .ok_or(nir::phases::StorageError::Budget)?;
            return Ok(());
        }
        nir::phases::validate_runtime_storage_with_nominals(ty, metadata, &frames, resolve, fuel)?;
        if ty.check_scope(&[]).is_ok() {
            validated_closed.insert(ty.clone());
        } else {
            validated_in_fn.insert(ty.clone());
        }
        Ok(())
    };
    for env in &function.env_parameters {
        check_storage(&env.ty, fuel).map_err(|message| {
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
    while let Some(root) = roots.pop() {
        nir::visit::expression_pruned(root, fuel, |expression, fuel| {
            check_storage(&expression.ty, fuel).map_err(|message| {
                format!(
                    "IIR runtime storage in {} at {}: {message}",
                    function.name, expression.source
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
