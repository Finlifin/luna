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
        );
        Ok(())
    })?;
    Ok(inert)
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
        if !parameters.iter().any(|p| p.sort != nir::Sort::Runtime)
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
        iir::visit::expression(expression, fuel, |_, _| Ok(()))?;
        if !target(then_branch) || !target(else_branch) {
            return Ok(());
        }
        crate::associated::charge_projection_copy(&expression.ty, fuel)?;
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
            then_branch,
            else_branch: Some(else_branch),
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
            then_branch: branch(then_branch, arguments.clone(), parameter_bindings.clone()),
            else_branch: Some(branch(else_branch, arguments, parameter_bindings)),
        };
        Ok(())
    }
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
}
