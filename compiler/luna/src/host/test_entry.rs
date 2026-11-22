//! Source test adapters for void and `!E void` test functions.
use super::*;
use nir::program::{
    Expr, ExprKind as K, Function, FunctionPhase, Literal, MatchArm, Pattern, SourceRange,
};
use nir::{DeclarationId, Primitive, Term};

/// Describe one error branch type as canonical S-expression text for failure reports.
pub(crate) fn describe_error_type(package: &Package, ty: &Term) -> String {
    let name = match ty {
        Term::Nominal { declaration, .. } => package
            .enums
            .iter()
            .find(|def| def.declaration == *declaration)
            .map(|def| def.name.as_str())
            .or_else(|| {
                package
                    .structs
                    .iter()
                    .find(|def| def.declaration == *declaration)
                    .map(|def| def.name.as_str())
            })
            .or_else(|| {
                package
                    .newtypes
                    .iter()
                    .find(|def| def.declaration == *declaration)
                    .map(|def| def.name.as_str())
            }),
        _ => None,
    };
    match name {
        Some(name) => format!(
            "(error-type {}) (type {ty})",
            Literal::String(name.to_owned())
        ),
        None => format!("(type {ty})"),
    }
}

/// Describe the error branch at `index` within an `ErrorResult` type.
pub(crate) fn describe_error_branch(package: &Package, ty: &Term, index: usize) -> String {
    if let Term::ErrorResult { errors, .. } = ty
        && let Some(err_ty) = errors.get(index)
    {
        return describe_error_type(package, err_ty);
    }
    format!("(type {ty})")
}

/// Synthesize a `() -> u32` runtime wrapper that evaluates a `!E void` test
/// function, drops any non-`Copy` error payload cleanly, and returns `0` on
/// success or the 1-based error branch tag on failure.
pub(crate) fn synthesize_error_test_adapter(
    package: &mut Package,
    target: DeclarationId,
) -> Result<(DeclarationId, Vec<String>), String> {
    let original = package
        .functions
        .iter()
        .find(|f| f.declaration == target)
        .ok_or("test declaration is missing")?
        .clone();
    let Term::Function { parameters, result } = original.signature.callable_signature() else {
        return Err("test is not callable".into());
    };
    if original.phase != FunctionPhase::Runtime
        || !original.type_parameters.is_empty()
        || !original.parameters.is_empty()
        || !original.env_parameters.is_empty()
        || !parameters.is_empty()
    {
        return Err("test must be a non-generic zero-argument runtime function".into());
    }
    let Term::ErrorResult { errors, result: ok } = result.as_ref() else {
        return Err("test must return void or !E void".into());
    };
    if **ok != Term::Primitive(Primitive::Void) {
        return Err("test must return void or !E void".into());
    }

    let error_descriptions = errors
        .iter()
        .map(|err_ty| describe_error_type(package, err_ty))
        .collect::<Vec<_>>();

    let mut declaration = DeclarationId {
        package: u64::MAX,
        local: u32::MAX,
    };
    while package
        .functions
        .iter()
        .any(|f| f.declaration == declaration)
        || package.tests.contains(&declaration)
    {
        declaration.local = declaration
            .local
            .checked_sub(1)
            .ok_or("host test adapter identity space exhausted")?;
    }

    let mut cursor = 0u32;
    let mut next_source = || {
        let start = cursor;
        cursor += 1;
        SourceRange { start, end: cursor }
    };

    let u32_ty = Term::Primitive(Primitive::U32);
    let callee_expr = Expr {
        source: next_source(),
        ty: original.signature.clone(),
        kind: K::Function(target),
    };
    let call_expr = Expr {
        source: next_source(),
        ty: (**result).clone(),
        kind: K::Call {
            callee: Box::new(callee_expr),
            arguments: vec![],
            env_arguments: vec![],
            parameter_bindings: vec![],
        },
    };

    let mut arms = Vec::with_capacity(errors.len() + 1);
    arms.push(MatchArm {
        pattern: Pattern::ErrorResult {
            label: None,
            inner: Box::new(Pattern::Wild),
        },
        guard: None,
        body: Expr {
            source: next_source(),
            ty: u32_ty.clone(),
            kind: K::Literal(Literal::UnsignedInteger(0)),
        },
    });
    for (index, err_ty) in errors.iter().enumerate() {
        let tag = u32::try_from(index + 1).map_err(|_| "test error branch limit exceeded")?;
        arms.push(MatchArm {
            pattern: Pattern::ErrorResult {
                label: Some(err_ty.clone()),
                inner: Box::new(Pattern::Wild),
            },
            guard: None,
            body: Expr {
                source: next_source(),
                ty: u32_ty.clone(),
                kind: K::Literal(Literal::UnsignedInteger(u128::from(tag))),
            },
        });
    }

    let match_expr = Expr {
        source: next_source(),
        ty: u32_ty.clone(),
        kind: K::Match {
            value: Box::new(call_expr),
            arms,
        },
    };
    let body_source = next_source();
    let wrapper_source = next_source();
    let wrapper = Function {
        interface: nir::program::FunctionDeclaration {
            source: wrapper_source,
            declaration,
            name: format!("<host-test-adapter:{}>", original.name),
            phase: FunctionPhase::Runtime,
            inline: false,
            pure: false,
            is_unsafe: false,
            preconditions: vec![],
            postconditions: vec![],
            result_binding: None,
            result_type_binder: None,
            result_guarantees: vec![],
            entry_snapshots: vec![],
            type_parameters: vec![],
            requirements: vec![],
            capabilities: vec![],
            parameters: vec![],
            env_parameters: vec![],
            defaults: vec![],
            signature: Term::Function {
                parameters: vec![],
                result: Box::new(u32_ty.clone()),
            },
        },
        runtime_deliveries: Default::default(),
        body: Expr {
            source: body_source,
            ty: u32_ty,
            kind: K::Block {
                statements: vec![],
                result: Some(Box::new(match_expr)),
            },
        },
    };
    package.functions.push(wrapper);
    Ok((declaration, error_descriptions))
}

/// Emit standalone LLVM IR for a `void` or `!E void` source test declaration.
pub(crate) fn llvm_ir_test_declaration(
    package: &Package,
    declaration: DeclarationId,
    fuel: u64,
    protocols: Option<(DeclarationId, DeclarationId)>,
) -> Result<String, String> {
    let function = package
        .functions
        .iter()
        .find(|f| f.declaration == declaration)
        .ok_or("test declaration has no executable body")?;
    let Term::Function { parameters, result } = function.signature.callable_signature() else {
        return Err("test must return void or !E void".into());
    };
    if !parameters.is_empty() {
        return Err("test must be a non-generic zero-argument runtime function".into());
    }
    match result.as_ref() {
        Term::Primitive(Primitive::Void) => llvm_ir_declaration_with_entry_mode(
            package,
            declaration,
            fuel,
            protocols,
            llvm::EntryMode::TestVoid,
        ),
        Term::ErrorResult { result: ok, .. } if **ok == Term::Primitive(Primitive::Void) => {
            let mut owned = package.clone();
            let (adapter, error_descriptions) =
                synthesize_error_test_adapter(&mut owned, declaration)?;
            llvm_ir_declaration_with_entry_mode(
                &owned,
                adapter,
                fuel,
                protocols,
                llvm::EntryMode::TestErrorTag { error_descriptions },
            )
        }
        _ => Err("test must return void or !E void".into()),
    }
}
