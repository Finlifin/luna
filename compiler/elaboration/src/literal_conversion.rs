//! Source-bound literal conversion. The suffix is a value index, not a Type name.
use nir::{
    DeclarationId, Parameter, Primitive, Sort, Term,
    program::{MethodContract, Trait},
};

pub fn protocol(traits: &[Trait], id: DeclarationId) -> Result<&MethodContract, String> {
    let mut matches = traits.iter().filter(|t| t.declaration == id);
    let declaration = matches
        .next()
        .ok_or("LiteralInto protocol is not a declared trait")?;
    if matches.next().is_some() {
        return Err("duplicate LiteralInto protocol identity".into());
    }
    if declaration.parameter_types
        != [
            Term::Primitive(Primitive::Symbol),
            Term::Primitive(Primitive::Type),
        ]
        || declaration.type_parameters.len() != 2
    {
        return Err("LiteralInto requires ordered Symbol and Type parameters".into());
    }
    let methods = declaration
        .methods
        .iter()
        .filter(|m| m.name == "literal_into")
        .collect::<Vec<_>>();
    let [method] = methods.as_slice() else {
        return Err("LiteralInto requires one literal_into method".into());
    };
    let Ok(Term::Function { parameters, result }) = method.signature.callable_signature() else {
        return Err("literal_into must be callable".into());
    };
    let expected = Parameter {
        sort: Sort::ComptimeValue,
        ty: Term::Rigid {
            declaration: declaration.self_parameter,
            sort: Sort::Type,
        },
    };
    let result = match result.as_ref() {
        Term::Computation { result, .. } => result.as_ref(),
        result => result,
    };
    if parameters != &[expected]
        || *result
            != (Term::Rigid {
                declaration: declaration.type_parameters[1],
                sort: Sort::Type,
            })
        || method.by_value_receiver
        || method.by_reference_receiver
        || !method.type_parameters.is_empty()
        || !method.type_arguments.is_empty()
        || method.parameter_bindings.len() != 1
    {
        return Err("LiteralInto requires fn literal_into(comptime value: Self) -> Result".into());
    }
    Ok(method)
}
