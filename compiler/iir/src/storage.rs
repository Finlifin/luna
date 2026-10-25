//! Storage-phase validation in concrete runtime bodies, retaining staged inputs.
use nir::program::{ExprKind as K, Function, FunctionPhase, Package};
use nir::{Sort, Term};

pub(super) fn validate(
    function: &Function,
    metadata: &Package,
    resolve: &dyn Fn(&Term, &mut usize) -> Option<Vec<Term>>,
    fuel: &mut usize,
) -> Result<(), String> {
    if function.phase != FunctionPhase::Runtime {
        return Ok(());
    }
    let Term::Function { parameters, .. } = function.signature.callable_signature() else {
        return Err("IIR runtime function has no callable signature".into());
    };
    let frames = vec![parameters.iter().map(|p| p.sort).collect()];
    let mut roots = vec![&function.body];
    roots.extend(&function.preconditions);
    roots.extend(&function.postconditions);
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
            nir::phases::validate_runtime_storage_with_nominals(
                &expression.ty,
                metadata,
                &frames,
                resolve,
                fuel,
            )
            .map_err(|message| {
                format!(
                    "IIR runtime storage in {} at {}: {message}",
                    function.name, expression.source
                )
            })?;
            match &expression.kind {
                K::Call {
                    callee, arguments, ..
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
                _ => Ok(true),
            }
        })?;
    }
    Ok(())
}
