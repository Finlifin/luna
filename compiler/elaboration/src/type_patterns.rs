//! Validate the source-selected `TypePatternView` protocol contract.
use nir::code_interfaces::MethodSignature;
use nir::program::Trait;
use nir::{AssociatedTypeOrigin, DeclarationId, Parameter, Primitive, Sort, Term};

/// Validate `TypePatternView` before elaboration dispatches custom type patterns.
pub fn protocol(traits: &[Trait], id: DeclarationId) -> Result<(), String> {
    let mut candidates = traits.iter().filter(|item| item.declaration == id);
    let interface = candidates
        .next()
        .ok_or_else(|| "TypePatternView binding is not a trait".to_owned())?;
    if candidates.next().is_some() || !interface.type_parameters.is_empty() {
        return Err(
            "TypePatternView requires one trait identity without explicit parameters".into(),
        );
    }
    let args_members = interface
        .associated
        .iter()
        .filter(|member| member.name == "Args")
        .collect::<Vec<_>>();
    let [args_member] = args_members.as_slice() else {
        return Err("TypePatternView requires one associated Args: Type member".into());
    };
    if args_member.ty != Term::Primitive(Primitive::Type) {
        return Err("TypePatternView Args associated member must have type Type".into());
    }

    let destructure_methods = interface
        .methods
        .iter()
        .filter(|method| method.name == "type_pattern_view_destructure")
        .collect::<Vec<_>>();
    let [method] = destructure_methods.as_slice() else {
        return Err("TypePatternView requires one type_pattern_view_destructure method".into());
    };
    if method.by_value_receiver
        || method.by_reference_receiver
        || !method.type_parameters.is_empty()
        || !method.type_arguments.is_empty()
    {
        return Err("invalid TypePatternView type_pattern_view_destructure signature".into());
    }
    let MethodSignature::Callable(sig_term) = &method.signature else {
        return Err(
            "TypePatternView type_pattern_view_destructure must be a callable method".into(),
        );
    };
    if !sig_term.is_pure_callable() || !sig_term.is_comptime_callable() {
        return Err(
            "TypePatternView type_pattern_view_destructure must be a pure comptime function".into(),
        );
    }
    let Ok(Term::Function { parameters, result }) = method.signature.callable_signature() else {
        return Err("TypePatternView type_pattern_view_destructure is not callable".into());
    };
    let expected_param = Parameter {
        sort: Sort::Runtime,
        ty: Term::Primitive(Primitive::Type),
    };
    let subject = Term::Rigid {
        declaration: interface.self_parameter,
        sort: Sort::Type,
    };
    let expected_args = Term::AssociatedType {
        origin: Box::new(AssociatedTypeOrigin {
            interface: id,
            member: args_member.declaration,
        }),
        arguments: vec![],
        subject: Box::new(subject),
    };
    let expected_result = Term::Optional(Box::new(expected_args));
    if parameters.as_slice() != [expected_param] || result.as_ref() != &expected_result {
        return Err("invalid TypePatternView type_pattern_view_destructure signature".into());
    }
    Ok(())
}
