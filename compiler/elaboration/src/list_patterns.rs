//! Validate source-selected list observation methods before synthesizing calls.
use nir::program::{Trait, TraitPhase};
use nir::{AssociatedTypeOrigin, DeclarationId, Parameter, Primitive, Sort, Term};

pub struct Protocol {
    pub size: Term,
    pub comptime: bool,
    pub by_value: bool,
    pub length: &'static str,
    pub element: &'static str,
    pub rest: Option<&'static str>,
    pub rest_default: Option<DeclarationId>,
}

pub fn protocol(traits: &[Trait], id: DeclarationId) -> Result<Protocol, String> {
    let runtime = phase_protocol(traits, id, false)?;
    if has_staged_contract(traits, id) {
        if runtime.comptime {
            return Err("dual-phase ListPatternView requires runtime base methods".into());
        }
        phase_protocol(traits, id, true)?;
    }
    Ok(runtime)
}

pub fn for_phase(traits: &[Trait], id: DeclarationId, comptime: bool) -> Result<Protocol, String> {
    let base = protocol(traits, id)?;
    if comptime && has_staged_contract(traits, id) {
        phase_protocol(traits, id, true)
    } else {
        Ok(base)
    }
}

fn has_staged_contract(traits: &[Trait], id: DeclarationId) -> bool {
    traits
        .iter()
        .filter(|item| item.declaration == id)
        .any(|item| {
            item.associated
                .iter()
                .any(|member| member.name == "ElementComptime")
                || item.methods.iter().any(|method| {
                    matches!(
                        method.name.as_str(),
                        "list_pattern_view_get_len_comptime"
                            | "list_pattern_view_get_element_comptime"
                            | "list_pattern_view_rest_comptime"
                    )
                })
        })
}

fn phase_protocol(traits: &[Trait], id: DeclarationId, staged: bool) -> Result<Protocol, String> {
    let (element_name, length_name, get_name, rest_name) = if staged {
        (
            "ElementComptime",
            "list_pattern_view_get_len_comptime",
            "list_pattern_view_get_element_comptime",
            "list_pattern_view_rest_comptime",
        )
    } else {
        (
            "Element",
            "list_pattern_view_get_len",
            "list_pattern_view_get_element",
            "list_pattern_view_rest",
        )
    };
    let mut candidates = traits.iter().filter(|item| item.declaration == id);
    let interface = candidates
        .next()
        .ok_or("ListPatternView binding is not a trait")?;
    if candidates.next().is_some() || !interface.type_parameters.is_empty() {
        return Err(
            "ListPatternView requires one trait identity without explicit parameters".into(),
        );
    }
    let elements = interface
        .associated
        .iter()
        .filter(|item| item.name == element_name)
        .collect::<Vec<_>>();
    let [element] = elements.as_slice() else {
        return Err(format!(
            "ListPatternView requires one associated {element_name}"
        ));
    };
    if element.ty != Term::Primitive(Primitive::Type)
        || (staged && element.phase != TraitPhase::Comptime)
    {
        return Err("ListPatternView Element must be Type-valued".into());
    }
    let lengths = interface
        .methods
        .iter()
        .filter(|method| method.name == length_name)
        .collect::<Vec<_>>();
    let [length] = lengths.as_slice() else {
        return Err("ListPatternView requires one length method".into());
    };
    let Ok(Term::Function { result, .. }) = length.signature.callable_signature() else {
        return Err("ListPatternView length method is not callable".into());
    };
    let size = result.as_ref().clone();
    let comptime = length.signature.is_comptime_callable();
    if staged && !comptime {
        return Err("ListPatternView comptime methods must be comptime callables".into());
    }
    if !matches!(&size, Term::Primitive(p) if *p == Primitive::Usize
        || p.fixed_integer_layout().is_some_and(|(signed,_)| !signed)
        || (comptime && *p == Primitive::Integer))
    {
        return Err(
            "ListPatternView length and index require an unsigned size type or comptime Integer"
                .into(),
        );
    }
    let subject = Term::Rigid {
        declaration: interface.self_parameter,
        sort: Sort::Type,
    };
    let receiver = Parameter {
        sort: Sort::Runtime,
        ty: if comptime {
            subject.clone()
        } else {
            Term::Pointer(Box::new(subject.clone()))
        },
    };
    let element_type = Term::AssociatedType {
        origin: Box::new(AssociatedTypeOrigin {
            interface: id,
            member: element.declaration,
        }),
        arguments: vec![],
        subject: Box::new(subject),
    };
    let index_param = Parameter {
        sort: if staged || interface.comptime_only {
            Sort::Runtime
        } else {
            Sort::ComptimeValue
        },
        ty: size.clone(),
    };
    for (name, parameters, result) in [
        (length_name, vec![receiver.clone()], size.clone()),
        (
            get_name,
            vec![receiver.clone(), index_param.clone()],
            element_type,
        ),
    ] {
        let methods = interface
            .methods
            .iter()
            .filter(|method| method.name == name)
            .collect::<Vec<_>>();
        let [method] = methods.as_slice() else {
            return Err(format!("ListPatternView requires one {name} method"));
        };
        let expected = Term::Function {
            parameters,
            result: Box::new(result),
        };
        if method.signature.is_comptime_callable() != comptime
            || method.signature.callable_signature() != Ok(&expected)
            || method.by_reference_receiver == comptime
            || method.by_value_receiver != comptime
            || (staged && method.phase != TraitPhase::Comptime)
            || !method.type_parameters.is_empty()
            || !method.type_arguments.is_empty()
        {
            return Err(format!("invalid ListPatternView {name} signature"));
        }
    }
    let rest_methods = interface
        .methods
        .iter()
        .filter(|method| method.name == rest_name)
        .collect::<Vec<_>>();
    let (rest, rest_default) = match rest_methods.as_slice() {
        [] => (None, None),
        [method] => {
            let expected_params = vec![receiver, index_param];
            let valid_signature = matches!(
                method.signature.callable_signature(),
                Ok(Term::Function { parameters, .. }) if *parameters == expected_params
            );
            if method.signature.is_comptime_callable() != comptime
                || !valid_signature
                || method.by_reference_receiver == comptime
                || method.by_value_receiver != comptime
                || (staged && method.phase != TraitPhase::Comptime)
                || !method.type_parameters.is_empty()
                || !method.type_arguments.is_empty()
            {
                return Err(format!("invalid ListPatternView {rest_name} signature"));
            }
            (Some(rest_name), method.default)
        }
        _ => {
            return Err(format!(
                "ListPatternView requires at most one {rest_name} method"
            ));
        }
    };
    Ok(Protocol {
        size,
        comptime,
        by_value: comptime,
        length: length_name,
        element: get_name,
        rest,
        rest_default,
    })
}
