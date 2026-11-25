//! Source-bound literal conversion. The suffix is a value index, not a Type name.
use nir::{
    DeclarationId, Parameter, Primitive, Sort, Term,
    program::{MethodContract, Trait, TraitPhase},
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
    let expected = Parameter {
        sort: Sort::ComptimeValue,
        ty: Term::Rigid {
            declaration: declaration.self_parameter,
            sort: Sort::Type,
        },
    };
    let expected_result = Term::Rigid {
        declaration: declaration.type_parameters[1],
        sort: Sort::Type,
    };
    let validate_signature = |candidate: &MethodContract,
                              require_comptime: bool|
     -> Result<(), String> {
        let Ok(Term::Function { parameters, result }) = candidate.signature.callable_signature()
        else {
            return Err(format!("{} must be callable", candidate.name));
        };
        let result = match result.as_ref() {
            Term::Computation { result, .. } => result.as_ref(),
            result => result,
        };
        if (require_comptime
            && (candidate.phase != TraitPhase::Comptime
                || !candidate.signature.is_comptime_callable()))
            || parameters != &[expected.clone()]
            || *result != expected_result
            || candidate.by_value_receiver
            || candidate.by_reference_receiver
            || !candidate.type_parameters.is_empty()
            || !candidate.type_arguments.is_empty()
            || candidate.parameter_bindings.len() != 1
        {
            return Err(if require_comptime {
                "LiteralInto requires ^attrs.comptime_phase comptime fn comptime_literal_into(comptime value: Self) -> Result".into()
            } else {
                "LiteralInto requires fn literal_into(comptime value: Self) -> Result".into()
            });
        }
        Ok(())
    };
    validate_signature(method, false)?;
    let comptime_methods = declaration
        .methods
        .iter()
        .filter(|m| m.name == "comptime_literal_into")
        .collect::<Vec<_>>();
    match comptime_methods.as_slice() {
        [] => {}
        [ct_method] => validate_signature(ct_method, true)?,
        _ => return Err("LiteralInto allows at most one comptime_literal_into method".into()),
    }
    Ok(method)
}
