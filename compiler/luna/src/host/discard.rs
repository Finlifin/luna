//! Source-level ownership adapter, checked before LLVM lowering.
use super::*;
use nir::program::{Expr, ExprKind as K, Function, SourceRange};
use nir::{DeclarationId, Primitive, Term};

/// Call an entry once and retire its delivered value through the ordinary
/// resource verifier. This adapter never inspects or prints the owned value.
pub fn llvm_ir_discarding_result(
    package: &Package,
    identity: Option<u64>,
    entry: &str,
    fuel: u64,
    protocols: (DeclarationId, DeclarationId),
    inline: bool,
) -> Result<String, String> {
    let expanded;
    let package = if inline {
        expanded = inline_package(package, fuel)?;
        &expanded
    } else {
        package
    };
    let (prepared, selected) = crate::invocation::prepare_entry_in_package(
        package,
        entry,
        FunctionPhase::Runtime,
        identity,
    )?;
    let mut owned = prepared.into_owned();
    let original = owned
        .functions
        .iter()
        .find(|f| f.declaration == selected)
        .ok_or("discard entry is missing")?;
    let Term::Function { parameters, result } = &original.signature else {
        return Err("discard entry is not callable".into());
    };
    if !parameters.is_empty() {
        return Err("discard entry requires a zero-input call plan".into());
    }
    let mut declaration = DeclarationId {
        package: u64::MAX,
        local: u32::MAX,
    };
    while owned.functions.iter().any(|f| f.declaration == declaration) {
        declaration.local = declaration
            .local
            .checked_sub(1)
            .ok_or("host adapter identity space exhausted")?;
    }
    let source = SourceRange { start: 0, end: 1 };
    let call = Expr {
        source,
        ty: (**result).clone(),
        kind: K::Call {
            callee: Box::new(Expr {
                source,
                ty: original.signature.clone(),
                kind: K::Function(selected),
            }),
            arguments: vec![],
            parameter_bindings: vec![],
        },
    };
    let body_type = if **result == Term::Primitive(Primitive::Never) {
        (**result).clone()
    } else {
        Term::Primitive(Primitive::Void)
    };
    let wrapper = Function {
        declaration,
        name: "<host-discard-result>".into(),
        phase: FunctionPhase::Runtime,
        inline: false,
        pure: false,
        is_unsafe: false,
        preconditions: vec![],
        postconditions: vec![],
        result_binding: None,
        entry_snapshots: vec![],
        type_parameters: vec![],
        requirements: vec![],
        capabilities: vec![],
        parameters: vec![],
        defaults: vec![],
        signature: Term::Function {
            parameters: vec![],
            result: Box::new(Term::Primitive(Primitive::Void)),
        },
        body: Expr {
            source: SourceRange { start: 0, end: 2 },
            ty: body_type.clone(),
            kind: K::Block {
                statements: vec![Expr {
                    source,
                    ty: body_type.clone(),
                    kind: K::Discard(Box::new(call)),
                }],
                result: None,
            },
        },
    };
    owned.functions.push(wrapper);
    llvm_ir_declaration(&owned, declaration, fuel, Some(protocols))
}
