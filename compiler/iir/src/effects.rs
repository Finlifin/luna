//! Concrete direct-effect environments. Requests retain explicit occurrence
//! slots; callable preparation is checked in the caller's lexical environment.
use crate::{Package, visit};
use nir::effects::{Abi, Operation};
use nir::program::{Expr, ExprKind as K, Function, FunctionPhase, Pattern};
use nir::{Primitive, Sort, Term};

fn spend(fuel: &mut usize) -> Result<(), String> {
    *fuel = fuel
        .checked_sub(1)
        .ok_or("IIR effect validation budget exhausted")?;
    Ok(())
}
fn compatible(actual: &Term, expected: &Term) -> bool {
    actual == expected || *actual == Term::Primitive(Primitive::Never)
}
fn row_result(ty: &Term) -> (&[Operation], &Term) {
    if let Term::Computation { effects, result } = ty {
        (effects, result)
    } else {
        (&[], ty)
    }
}
// Function values may advertise an effectful result. A computation is not yet
// a storable data value in the executable reference representation.
fn data_type(package: &Package, ty: &Term, fuel: &mut usize, depth: usize) -> Result<(), String> {
    spend(fuel)?;
    if depth >= 128 {
        return Err("IIR effect type depth exhausted".into());
    }
    match ty {
        Term::Computation { .. } => {
            return Err("IIR stored computation values require value/evidence lowering".into());
        }
        Term::Function { parameters, result } => {
            for p in parameters {
                data_type(package, &p.ty, fuel, depth + 1)?;
            }
            data_type(package, row_result(result).1, fuel, depth + 1)?;
        }
        Term::ComptimeFunction(inner) | Term::Pointer(inner) => {
            data_type(package, inner, fuel, depth + 1)?
        }
        Term::Tuple(_) | Term::Record(_) => {
            if let Term::Tuple(items) = ty {
                for item in items {
                    data_type(package, item, fuel, depth + 1)?;
                }
            }
            if let Term::Record(items) = ty {
                for (_, item) in items {
                    data_type(package, item, fuel, depth + 1)?;
                }
            }
        }
        Term::Closure {
            environment,
            signature,
        } => {
            for item in environment {
                data_type(package, item, fuel, depth + 1)?;
            }
            data_type(package, signature, fuel, depth + 1)?;
        }
        Term::NormalForm { head, .. } => {
            // Only an explicitly registered Tuple head grants a field layout.
            // Other normal-form arguments remain opaque type metadata.
            if let Some(fields) = package
                .normal_forms
                .iter()
                .find(|definition| definition.declaration == *head)
                .and_then(|definition| definition.tuple_elements(ty))
            {
                for field in fields {
                    data_type(package, field, fuel, depth + 1)?;
                }
            }
        }
        _ => {}
    }
    Ok(())
}

pub(crate) fn validate(
    package: &Package,
    function: &Function,
    fuel: &mut usize,
) -> Result<(), String> {
    data_type(package, &function.signature, fuel, 0)?;
    let Term::Function { result, .. } = function.signature.callable_signature() else {
        return Err("IIR effect function signature missing".into());
    };
    let (row, _) = row_result(result);
    let runtime = function.phase == FunctionPhase::Runtime;
    expression(package, &function.body, row, None, runtime, true, fuel, 0)?;
    for value in function
        .preconditions
        .iter()
        .chain(&function.postconditions)
        .chain(function.defaults.iter().map(|(_, value)| value))
    {
        expression(package, value, &[], None, runtime, false, fuel, 0)?;
    }
    Ok(())
}
fn operation(
    package: &Package,
    op: &Operation,
    signature: &Term,
    fuel: &mut usize,
) -> Result<(), String> {
    for arg in &op.arguments {
        if arg.sort() != Sort::Type {
            return Err("IIR effect label argument is not a Type".into());
        }
        package.validate_type(arg, fuel)?;
    }
    package.validate_type(signature, fuel)?;
    let Term::Function { parameters, result } = signature else {
        return Err("IIR effect signature is not a function".into());
    };
    for parameter in parameters {
        if parameter.sort != Sort::Runtime || parameter.ty.check_scope(&[]).is_err() {
            return Err("IIR dependent effect input requires identity lowering".into());
        }
        data_type(package, &parameter.ty, fuel, 0)?;
    }
    result
        .check_scope(&[])
        .map_err(|_| "IIR dependent effect result requires identity lowering")?;
    data_type(package, result, fuel, 0)
}
fn pattern(
    package: &Package,
    p: &Pattern,
    row: &[Operation],
    resume: Option<&Term>,
    runtime: bool,
    fuel: &mut usize,
    depth: usize,
) -> Result<(), String> {
    spend(fuel)?;
    if depth >= 128 {
        return Err("IIR effect pattern depth exhausted".into());
    }
    match p {
        Pattern::Then { first, value, next } => {
            pattern(package, first, row, resume, runtime, fuel, depth + 1)?;
            expression(package, value, row, resume, runtime, false, fuel, depth + 1)?;
            pattern(package, next, row, resume, runtime, fuel, depth + 1)?;
        }
        Pattern::Guard {
            pattern: p,
            condition,
        } => {
            pattern(package, p, row, resume, runtime, fuel, depth + 1)?;
            expression(
                package,
                condition,
                row,
                resume,
                runtime,
                false,
                fuel,
                depth + 1,
            )?;
        }
        Pattern::Binding { inner, .. } | Pattern::Remap { pattern: inner, .. } => {
            pattern(package, inner, row, resume, runtime, fuel, depth + 1)?
        }
        Pattern::Tuple(items)
        | Pattern::TypeTuple(items)
        | Pattern::Or(items)
        | Pattern::Record { fields: items, .. }
        | Pattern::AnonymousRecord { fields: items, .. }
        | Pattern::Variant { fields: items, .. }
        | Pattern::NormalForm {
            arguments: items, ..
        } => {
            for item in items {
                pattern(package, item, row, resume, runtime, fuel, depth + 1)?;
            }
        }
        _ => {}
    }
    Ok(())
}
fn expression(
    package: &Package,
    root: &Expr,
    row: &[Operation],
    resume: Option<&Term>,
    runtime: bool,
    computation: bool,
    fuel: &mut usize,
    depth: usize,
) -> Result<(), String> {
    if depth >= 128 {
        return Err("IIR effect environment depth exhausted".into());
    }
    visit::expression_pruned(root, fuel, |expr, fuel| {
        let allowed = computation && std::ptr::eq(expr, root);
        if matches!(expr.ty, Term::Computation { .. }) && !allowed {
            return Err("IIR computation requires an explicit row mapping".into());
        }
        if !matches!(expr.ty, Term::Computation { .. }) {
            data_type(package, &expr.ty, fuel, depth)?;
        }
        if !runtime
            && matches!(
                expr.kind,
                K::EffectHandle { .. }
                    | K::EffectRun { .. }
                    | K::EffectComputation { .. }
                    | K::EffectRequest { .. }
                    | K::EffectResume(_)
            )
        {
            return Err("IIR comptime effects are not supported".into());
        }
        match &expr.kind {
            K::EffectComputation { effects, body } => {
                if effects != row
                    || row_result(&expr.ty).0 != effects
                    || !compatible(&body.ty, row_result(&expr.ty).1)
                {
                    return Err("IIR computation environment/result mismatch".into());
                }
                expression(package, body, row, resume, runtime, false, fuel, depth + 1)?;
                Ok(false)
            }
            K::EffectRun {
                computation: body,
                slots,
            } => {
                let Term::Computation { effects, result } = &body.ty else {
                    return Err("IIR effect-run input is not a computation".into());
                };
                if effects.len() != slots.len() || !compatible(result, &expr.ty) {
                    return Err("IIR residual computation mapping is not supported".into());
                }
                for (i, (op, slot)) in effects.iter().zip(slots).enumerate() {
                    spend(fuel)?;
                    if row.get(*slot) != Some(op) {
                        return Err("IIR effect mapping selects the wrong occurrence".into());
                    }
                    for j in 0..i {
                        spend(fuel)?;
                        if slots[j] == *slot || (effects[j] == *op && slots[j] >= *slot) {
                            return Err(
                                "IIR effect mapping duplicates or reverses an occurrence".into()
                            );
                        }
                    }
                }
                mapped(
                    package,
                    body,
                    row,
                    effects,
                    resume,
                    runtime,
                    fuel,
                    depth + 1,
                )?;
                Ok(false)
            }
            K::EffectHandle {
                computation: body,
                handlers,
                outer,
            } => {
                if outer != row {
                    return Err("IIR handler outer environment mismatch".into());
                }
                let mut installed = Vec::new();
                for handler in handlers {
                    if !matches!(handler.abi, Abi::Direct | Abi::DirectPassing) {
                        return Err("IIR non-direct effects require control ABI lowering".into());
                    }
                    operation(package, &handler.operation, &handler.signature, fuel)?;
                    for prior in &installed {
                        spend(fuel)?;
                        if prior == &handler.operation {
                            return Err("IIR duplicate handler operation group".into());
                        }
                    }
                    installed.push(handler.operation.clone());
                    let Term::Function { parameters, result } = &handler.signature else {
                        unreachable!()
                    };
                    if handler.abi == Abi::DirectPassing && !parameters.is_empty() {
                        return Err("IIR direct passing operation has parameters".into());
                    }
                    if let Some(slot) = handler.fallback {
                        if outer.get(slot) != Some(&handler.operation) {
                            return Err("IIR handler fallback selects the wrong occurrence".into());
                        }
                    }
                    for arm in &handler.arms {
                        if !compatible(&arm.body.ty, result) {
                            return Err("IIR handler branch result mismatch".into());
                        }
                        pattern(
                            package,
                            &arm.pattern,
                            outer,
                            Some(result),
                            runtime,
                            fuel,
                            depth + 1,
                        )?;
                        if let Some(guard) = &arm.guard {
                            expression(
                                package,
                                guard,
                                outer,
                                Some(result),
                                runtime,
                                false,
                                fuel,
                                depth + 1,
                            )?;
                        }
                        expression(
                            package,
                            &arm.body,
                            outer,
                            Some(result),
                            runtime,
                            false,
                            fuel,
                            depth + 1,
                        )?;
                    }
                }
                installed.extend_from_slice(outer);
                if row_result(&body.ty).0 != installed
                    || !compatible(row_result(&body.ty).1, &expr.ty)
                {
                    return Err("IIR handler computation/result mismatch".into());
                }
                expression(
                    package,
                    body,
                    &installed,
                    resume,
                    runtime,
                    true,
                    fuel,
                    depth + 1,
                )?;
                Ok(false)
            }
            K::EffectRequest {
                operation: op,
                abi,
                allowed_index,
                signature,
                arguments,
            } => {
                if !matches!(abi, Abi::Direct | Abi::DirectPassing) {
                    return Err("IIR non-direct effects require control ABI lowering".into());
                }
                operation(package, op, signature, fuel)?;
                if row.get(*allowed_index) != Some(op) {
                    return Err("IIR effect request has no matching occurrence".into());
                }
                let Term::Function { parameters, result } = signature.as_ref() else {
                    unreachable!()
                };
                if *abi == Abi::DirectPassing && !parameters.is_empty() {
                    return Err("IIR direct passing operation has parameters".into());
                }
                if parameters.len() != arguments.len()
                    || !compatible(result, &expr.ty)
                    || parameters
                        .iter()
                        .zip(arguments)
                        .any(|(p, a)| !compatible(&a.ty, &p.ty))
                {
                    return Err("IIR effect request signature mismatch".into());
                }
                Ok(true)
            }
            K::EffectResume(value) => {
                if !resume.is_some_and(|ty| compatible(&value.ty, ty))
                    || expr.ty != Term::Primitive(Primitive::Never)
                {
                    return Err("IIR resume has no matching lexical branch result".into());
                }
                Ok(true)
            }
            K::Closure { captures, body, .. } => {
                for capture in captures {
                    data_type(package, &capture.ty, fuel, depth + 1)?;
                    if let Some(value) = &capture.value {
                        expression(package, value, row, resume, runtime, false, fuel, depth + 1)?;
                    }
                }
                let Term::Function { result, .. } = expr.ty.callable_signature() else {
                    return Err("IIR effect closure signature missing".into());
                };
                let own_row = row_result(result).0;
                expression(
                    package,
                    body,
                    own_row,
                    None,
                    !expr.ty.is_comptime_callable(),
                    true,
                    fuel,
                    depth + 1,
                )?;
                Ok(false)
            }
            _ if matches!(expr.ty, Term::Computation { .. }) => {
                Err("IIR computation requires an explicit environment boundary".into())
            }
            _ => Ok(true),
        }
    })
}
fn mapped(
    package: &Package,
    expr: &Expr,
    caller: &[Operation],
    callee: &[Operation],
    resume: Option<&Term>,
    runtime: bool,
    fuel: &mut usize,
    depth: usize,
) -> Result<(), String> {
    spend(fuel)?;
    if depth >= 128 {
        return Err("IIR mapped computation depth exhausted".into());
    }
    if !matches!(&expr.ty, Term::Computation { effects, .. } if effects == callee) {
        return Err("IIR mapped computation row mismatch".into());
    }
    match &expr.kind {
        K::Call {
            callee: target,
            arguments,
            ..
        } => {
            let Term::Function { result, .. } = target.ty.callable_signature() else {
                return Err("IIR effect callee signature missing".into());
            };
            if row_result(result).0 != callee || result.as_ref() != &expr.ty {
                return Err("IIR effect call result row mismatch".into());
            }
            expression(
                package,
                target,
                caller,
                resume,
                runtime,
                false,
                fuel,
                depth + 1,
            )?;
            for argument in arguments {
                expression(
                    package,
                    &argument.value,
                    caller,
                    resume,
                    runtime,
                    false,
                    fuel,
                    depth + 1,
                )?;
            }
            Ok(())
        }
        K::Block { statements, result } | K::CallPreparation { statements, result } => {
            let result = result
                .as_deref()
                .ok_or("IIR computation block has no result")?;
            if result.ty != expr.ty {
                return Err("IIR computation block result mismatch".into());
            }
            for statement in statements {
                expression(
                    package,
                    statement,
                    caller,
                    resume,
                    runtime,
                    false,
                    fuel,
                    depth + 1,
                )?;
            }
            mapped(
                package,
                result,
                caller,
                callee,
                resume,
                runtime,
                fuel,
                depth + 1,
            )
        }
        K::If {
            condition,
            then_branch,
            else_branch,
        } => {
            let else_branch = else_branch
                .as_deref()
                .ok_or("IIR computation conditional has no fallback")?;
            if then_branch.ty != expr.ty || else_branch.ty != expr.ty {
                return Err("IIR computation conditional result mismatch".into());
            }
            expression(
                package,
                condition,
                caller,
                resume,
                runtime,
                false,
                fuel,
                depth + 1,
            )?;
            mapped(
                package,
                then_branch,
                caller,
                callee,
                resume,
                runtime,
                fuel,
                depth + 1,
            )?;
            mapped(
                package,
                else_branch,
                caller,
                callee,
                resume,
                runtime,
                fuel,
                depth + 1,
            )
        }
        K::Match { value, arms } => {
            expression(
                package,
                value,
                caller,
                resume,
                runtime,
                false,
                fuel,
                depth + 1,
            )?;
            for arm in arms {
                if arm.body.ty != expr.ty {
                    return Err("IIR computation match result mismatch".into());
                }
                pattern(
                    package,
                    &arm.pattern,
                    caller,
                    resume,
                    runtime,
                    fuel,
                    depth + 1,
                )?;
                if let Some(guard) = &arm.guard {
                    expression(
                        package,
                        guard,
                        caller,
                        resume,
                        runtime,
                        false,
                        fuel,
                        depth + 1,
                    )?;
                }
                mapped(
                    package,
                    &arm.body,
                    caller,
                    callee,
                    resume,
                    runtime,
                    fuel,
                    depth + 1,
                )?;
            }
            Ok(())
        }
        K::EffectComputation { .. } => expression(
            package,
            expr,
            callee,
            resume,
            runtime,
            true,
            fuel,
            depth + 1,
        ),
        _ => Err("IIR computation value requires execution/evidence lowering".into()),
    }
}

pub(crate) fn validate_contracts(package: &Package, fuel: &mut usize) -> Result<(), String> {
    let mut contracts: std::collections::HashMap<Operation, (Abi, Term)> =
        std::collections::HashMap::new();
    for instance in &package.functions {
        visit::expressions(&instance.function, fuel, |expr, fuel| {
            let mut insert = |op: &Operation, abi: Abi, signature: &Term| -> Result<(), String> {
                spend(fuel)?;
                if let Some((prior_abi, prior)) = contracts.get(op) {
                    if *prior_abi != abi || prior != signature {
                        return Err(
                            "IIR effect occurrence has conflicting operation contracts".into()
                        );
                    }
                } else {
                    operation(package, op, signature, fuel)?;
                    contracts.insert(op.clone(), (abi, signature.clone()));
                }
                Ok(())
            };
            match &expr.kind {
                K::EffectRequest {
                    operation,
                    abi,
                    signature,
                    ..
                } => insert(operation, *abi, signature)?,
                K::EffectHandle { handlers, .. } => {
                    for handler in handlers {
                        insert(&handler.operation, handler.abi, &handler.signature)?;
                    }
                }
                _ => {}
            }
            Ok(())
        })?;
    }
    Ok(())
}

#[cfg(test)]
mod tuple_tests {
    use super::*;
    use nir::DeclarationId;
    use nir::normal_forms::{Head, Kind, Parameter, Role};
    use nir::program::LocalId;

    #[test]
    fn tuple_storage_checks_fields_without_unfolding_opaque_heads() {
        let id = DeclarationId {
            package: 1,
            local: 0,
        };
        let mut package = Package {
            normal_forms: vec![Head {
                declaration: id,
                role: Role::Tuple,
                parameters: vec![Parameter {
                    declaration: DeclarationId {
                        package: 1,
                        local: 1,
                    },
                    binding: LocalId { owner: 0, index: 0 },
                    kind: Kind::TypeList,
                    ty: Term::Nominal {
                        declaration: DeclarationId {
                            package: 1,
                            local: 2,
                        },
                        arguments: vec![Term::Primitive(Primitive::Type)],
                    },
                }],
                requirements: vec![],
            }],
            ..Default::default()
        };
        let tuple = |field| Term::NormalForm {
            head: id,
            sort: Sort::Type,
            arguments: vec![Term::TypeListValue(vec![field])],
        };
        let scalar = Term::Primitive(Primitive::Bool);
        let computation = Term::Computation {
            effects: vec![],
            result: Box::new(scalar.clone()),
        };
        let stored = tuple(tuple(computation.clone()));
        assert!(
            data_type(&package, &stored, &mut 100, 0)
                .unwrap_err()
                .contains("stored computation")
        );
        let callable = tuple(Term::Function {
            parameters: vec![],
            result: Box::new(computation),
        });
        data_type(&package, &callable, &mut 100, 0).unwrap();
        let valid = tuple(tuple(scalar));
        assert!(data_type(&package, &valid, &mut 1, 0).is_err());
        data_type(&package, &valid, &mut 100, 0).unwrap();
        assert!(data_type(&package, &valid, &mut 100, 128).is_err());
        package.normal_forms[0].role = Role::Opaque;
        // A lookalike application's arguments do not define stored fields.
        data_type(&package, &stored, &mut 100, 0).unwrap();
    }
}
