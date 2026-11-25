//! Validate source-selected map and struct pattern view protocols before synthesizing calls.
use nir::program::{Trait, TraitPhase};
use nir::{DeclarationId, Parameter, Primitive, Sort, Term};

#[derive(Clone, Copy, Debug)]
pub struct Protocol {
    pub is_map: bool,
    pub comptime: bool,
    pub by_value: bool,
    pub field_type: &'static str,
    pub get: &'static str,
}

pub fn stable_map_protocol(
    traits: &[Trait],
    id: DeclarationId,
    map_pattern: Option<DeclarationId>,
) -> Result<(), String> {
    stable_marker_protocol(
        traits,
        id,
        map_pattern,
        "StableMapPatternView",
        "MapPatternView",
    )
}

pub fn stable_struct_protocol(
    traits: &[Trait],
    id: DeclarationId,
    struct_pattern: Option<DeclarationId>,
) -> Result<(), String> {
    stable_marker_protocol(
        traits,
        id,
        struct_pattern,
        "StableStructPatternView",
        "StructPatternView",
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
    if let Some(super_id) = supertrait
        && !interface
            .parents
            .iter()
            .any(|parent| parent.declaration == super_id && parent.arguments.is_empty())
    {
        return Err(format!("{name} must require {super_name} as a supertrait"));
    }
    Ok(())
}

pub fn map_protocol(traits: &[Trait], id: DeclarationId) -> Result<Protocol, String> {
    let runtime = phase_protocol(traits, id, false, true)?;
    if has_staged_contract(traits, id, true) {
        if runtime.comptime {
            return Err("dual-phase MapPatternView requires runtime base methods".into());
        }
        phase_protocol(traits, id, true, true)?;
    }
    Ok(runtime)
}

pub fn map_for_phase(
    traits: &[Trait],
    id: DeclarationId,
    comptime: bool,
) -> Result<Protocol, String> {
    let base = map_protocol(traits, id)?;
    if comptime && has_staged_contract(traits, id, true) {
        phase_protocol(traits, id, true, true)
    } else {
        Ok(base)
    }
}

pub fn struct_protocol(traits: &[Trait], id: DeclarationId) -> Result<Protocol, String> {
    let runtime = phase_protocol(traits, id, false, false)?;
    if has_staged_contract(traits, id, false) {
        if runtime.comptime {
            return Err("dual-phase StructPatternView requires runtime base methods".into());
        }
        phase_protocol(traits, id, true, false)?;
    }
    Ok(runtime)
}

pub fn struct_for_phase(
    traits: &[Trait],
    id: DeclarationId,
    comptime: bool,
) -> Result<Protocol, String> {
    let base = struct_protocol(traits, id)?;
    if comptime && has_staged_contract(traits, id, false) {
        phase_protocol(traits, id, true, false)
    } else {
        Ok(base)
    }
}

pub fn has_staged_contract(traits: &[Trait], id: DeclarationId, is_map: bool) -> bool {
    let get_comptime = if is_map {
        "map_pattern_view_get_comptime"
    } else {
        "struct_pattern_view_get_comptime"
    };
    traits
        .iter()
        .filter(|item| item.declaration == id)
        .any(|item| {
            item.associated
                .iter()
                .any(|member| member.name == "FieldTypeComptime")
                || item
                    .methods
                    .iter()
                    .any(|method| method.name == get_comptime)
        })
}

fn is_field_type_signature(ty: &Term) -> bool {
    let inner = match ty {
        Term::PureFunction(inner) => inner.as_ref(),
        other => other,
    };
    let Term::ComptimeFunction(sig) = inner else {
        return false;
    };
    let Term::Function { parameters, result } = sig.as_ref() else {
        return false;
    };
    parameters.len() == 1
        && parameters[0].ty == Term::Primitive(Primitive::Symbol)
        && **result == Term::Primitive(Primitive::Type)
}

fn phase_protocol(
    traits: &[Trait],
    id: DeclarationId,
    staged: bool,
    is_map: bool,
) -> Result<Protocol, String> {
    let label = if is_map {
        "MapPatternView"
    } else {
        "StructPatternView"
    };
    let field_type_name = if staged {
        "FieldTypeComptime"
    } else {
        "FieldType"
    };
    let get_name = match (is_map, staged) {
        (true, false) => "map_pattern_view_get",
        (true, true) => "map_pattern_view_get_comptime",
        (false, false) => "struct_pattern_view_get",
        (false, true) => "struct_pattern_view_get_comptime",
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
    let field_types = interface
        .associated
        .iter()
        .filter(|item| item.name == field_type_name)
        .collect::<Vec<_>>();
    let [field_type] = field_types.as_slice() else {
        return Err(format!("{label} requires one associated {field_type_name}"));
    };
    if !is_field_type_signature(&field_type.ty)
        || (staged && field_type.phase != TraitPhase::Comptime)
    {
        return Err(format!(
            "{label} {field_type_name} must have type `pure comptime fn(Symbol) -> Type`"
        ));
    }

    let gets = interface
        .methods
        .iter()
        .filter(|method| method.name == get_name)
        .collect::<Vec<_>>();
    let [get_method] = gets.as_slice() else {
        return Err(format!("{label} requires one {get_name} method"));
    };
    let comptime = get_method.signature.is_comptime_callable();
    if staged && !comptime {
        return Err(format!(
            "{label} comptime methods must be comptime callables"
        ));
    }
    let subject = Term::Rigid {
        declaration: interface.self_parameter,
        sort: Sort::Type,
    };
    let expected_receiver = Parameter {
        sort: Sort::Runtime,
        ty: if comptime {
            subject
        } else {
            Term::Pointer(Box::new(subject))
        },
    };
    let expected_key_param = Parameter {
        sort: if staged || interface.comptime_only {
            Sort::Runtime
        } else {
            Sort::ComptimeValue
        },
        ty: Term::Primitive(Primitive::Symbol),
    };
    let Ok(Term::Function { parameters, result }) = get_method.signature.callable_signature()
    else {
        return Err(format!("{label} {get_name} method is not callable"));
    };
    let valid_result = if is_map {
        matches!(result.as_ref(), Term::Optional(_))
    } else {
        true
    };
    if parameters.as_slice() != [expected_receiver, expected_key_param]
        || !valid_result
        || get_method.by_reference_receiver == comptime
        || get_method.by_value_receiver != comptime
        || (staged && get_method.phase != TraitPhase::Comptime)
        || !get_method.type_parameters.is_empty()
        || !get_method.type_arguments.is_empty()
    {
        return Err(format!("invalid {label} {get_name} signature"));
    }

    Ok(Protocol {
        is_map,
        comptime,
        by_value: comptime,
        field_type: field_type_name,
        get: get_name,
    })
}
