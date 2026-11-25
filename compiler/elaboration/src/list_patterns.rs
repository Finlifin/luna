//! Validate source-selected list observation methods before synthesizing calls.
use nir::program::{Trait, TraitPhase};
use nir::{AssociatedTypeOrigin, DeclarationId, Parameter, Primitive, Sort, Term};

pub struct Protocol {
    pub size: Term,
    pub comptime: bool,
    pub by_value: bool,
    pub endless: bool,
    pub length: Option<&'static str>,
    pub element: &'static str,
    pub rest: Option<&'static str>,
    pub rest_default: Option<DeclarationId>,
}

pub fn stable_protocol(
    traits: &[Trait],
    id: DeclarationId,
    list_pattern: Option<DeclarationId>,
) -> Result<(), String> {
    stable_marker_protocol(
        traits,
        id,
        list_pattern,
        "StableListPatternView",
        "ListPatternView",
    )
}

pub fn stable_endless_protocol(
    traits: &[Trait],
    id: DeclarationId,
    endless_list_pattern: Option<DeclarationId>,
) -> Result<(), String> {
    stable_marker_protocol(
        traits,
        id,
        endless_list_pattern,
        "StableListPatternViewEndless",
        "ListPatternViewEndless",
    )
}

fn stable_marker_protocol(
    traits: &[Trait],
    id: DeclarationId,
    supertrait: Option<DeclarationId>,
    name: &str,
    super_name: &str,
) -> Result<(), String> {
    let mut candidates = traits.iter().filter(|item| item.declaration == id);
    let interface = candidates
        .next()
        .ok_or_else(|| format!("{name} binding is not a trait"))?;
    if candidates.next().is_some()
        || !interface.type_parameters.is_empty()
        || interface.comptime_only
    {
        return Err(format!(
            "{name} requires one runtime trait without explicit parameters"
        ));
    }
    if !interface.associated.is_empty() || !interface.methods.is_empty() {
        return Err(format!(
            "{name} marker trait must not declare extra members"
        ));
    }
    if let Some(list_id) = supertrait
        && !interface
            .parents
            .iter()
            .any(|parent| parent.declaration == list_id && parent.arguments.is_empty())
    {
        return Err(format!("{name} must require {super_name} as a supertrait"));
    }
    Ok(())
}

pub fn protocol(traits: &[Trait], id: DeclarationId) -> Result<Protocol, String> {
    let runtime = phase_protocol(traits, id, false, false)?;
    if has_staged_contract(traits, id) {
        if runtime.comptime {
            return Err("dual-phase ListPatternView requires runtime base methods".into());
        }
        phase_protocol(traits, id, true, false)?;
    }
    Ok(runtime)
}

pub fn for_phase(traits: &[Trait], id: DeclarationId, comptime: bool) -> Result<Protocol, String> {
    let base = protocol(traits, id)?;
    if comptime && has_staged_contract(traits, id) {
        phase_protocol(traits, id, true, false)
    } else {
        Ok(base)
    }
}

pub fn endless_protocol(traits: &[Trait], id: DeclarationId) -> Result<Protocol, String> {
    let runtime = phase_protocol(traits, id, false, true)?;
    if has_staged_contract(traits, id) {
        if runtime.comptime {
            return Err("dual-phase ListPatternViewEndless requires runtime base methods".into());
        }
        phase_protocol(traits, id, true, true)?;
    }
    Ok(runtime)
}

pub fn endless_for_phase(
    traits: &[Trait],
    id: DeclarationId,
    comptime: bool,
) -> Result<Protocol, String> {
    let base = endless_protocol(traits, id)?;
    if comptime && has_staged_contract(traits, id) {
        phase_protocol(traits, id, true, true)
    } else {
        Ok(base)
    }
}

pub fn has_staged_contract(traits: &[Trait], id: DeclarationId) -> bool {
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

fn phase_protocol(
    traits: &[Trait],
    id: DeclarationId,
    staged: bool,
    endless: bool,
) -> Result<Protocol, String> {
    let label = if endless {
        "ListPatternViewEndless"
    } else {
        "ListPatternView"
    };
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
        .ok_or_else(|| format!("{label} binding is not a trait"))?;
    if candidates.next().is_some() || !interface.type_parameters.is_empty() {
        return Err(format!(
            "{label} requires one trait identity without explicit parameters"
        ));
    }
    let elements = interface
        .associated
        .iter()
        .filter(|item| item.name == element_name)
        .collect::<Vec<_>>();
    let [element] = elements.as_slice() else {
        return Err(format!("{label} requires one associated {element_name}"));
    };
    if element.ty != Term::Primitive(Primitive::Type)
        || (staged && element.phase != TraitPhase::Comptime)
    {
        return Err(format!("{label} Element must be Type-valued"));
    }
    let (size, comptime) = if endless {
        if interface
            .methods
            .iter()
            .any(|method| method.name == length_name)
        {
            return Err(format!("{label} must not declare {length_name}"));
        }
        let gets = interface
            .methods
            .iter()
            .filter(|method| method.name == get_name)
            .collect::<Vec<_>>();
        let [get] = gets.as_slice() else {
            return Err(format!("{label} requires one {get_name} method"));
        };
        let Ok(Term::Function { parameters, .. }) = get.signature.callable_signature() else {
            return Err(format!("{label} {get_name} method is not callable"));
        };
        let Some(index_param) = parameters.get(1) else {
            return Err(format!("invalid {label} {get_name} signature"));
        };
        (index_param.ty.clone(), get.signature.is_comptime_callable())
    } else {
        let lengths = interface
            .methods
            .iter()
            .filter(|method| method.name == length_name)
            .collect::<Vec<_>>();
        let [length] = lengths.as_slice() else {
            return Err(format!("{label} requires one length method"));
        };
        let Ok(Term::Function { result, .. }) = length.signature.callable_signature() else {
            return Err(format!("{label} length method is not callable"));
        };
        (
            result.as_ref().clone(),
            length.signature.is_comptime_callable(),
        )
    };
    if staged && !comptime {
        return Err(format!(
            "{label} comptime methods must be comptime callables"
        ));
    }
    if !matches!(&size, Term::Primitive(p) if *p == Primitive::Usize
        || p.fixed_integer_layout().is_some_and(|(signed,_)| !signed)
        || (comptime && *p == Primitive::Integer))
    {
        return Err(format!(
            "{label} length and index require an unsigned size type or comptime Integer"
        ));
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
    let mut required_methods = Vec::new();
    if !endless {
        required_methods.push((length_name, vec![receiver.clone()], size.clone()));
    }
    required_methods.push((
        get_name,
        vec![receiver.clone(), index_param.clone()],
        element_type,
    ));
    for (name, parameters, result) in required_methods {
        let methods = interface
            .methods
            .iter()
            .filter(|method| method.name == name)
            .collect::<Vec<_>>();
        let [method] = methods.as_slice() else {
            return Err(format!("{label} requires one {name} method"));
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
            return Err(format!("invalid {label} {name} signature"));
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
                return Err(format!("invalid {label} {rest_name} signature"));
            }
            (Some(rest_name), method.default)
        }
        _ => {
            return Err(format!("{label} requires at most one {rest_name} method"));
        }
    };
    Ok(Protocol {
        size,
        comptime,
        by_value: comptime,
        endless,
        length: (!endless).then_some(length_name),
        element: get_name,
        rest,
        rest_default,
    })
}
