//! Validate ConcatPatternView and StableConcatPatternView protocol contracts.
use nir::program::{Trait, TraitPhase};
use nir::{AssociatedTypeOrigin, DeclarationId, Parameter, Primitive, Sort, Term};

#[derive(Clone, Debug)]
pub struct Protocol {
    pub comptime: bool,
    pub by_value: bool,
    pub part_assoc: DeclarationId,
    pub rest_assoc: DeclarationId,
    pub strip_prefix: &'static str,
    pub strip_suffix: &'static str,
}

pub fn stable_protocol(
    traits: &[Trait],
    id: DeclarationId,
    concat_pattern: Option<DeclarationId>,
) -> Result<(), String> {
    let mut candidates = traits.iter().filter(|item| item.declaration == id);
    let interface = candidates
        .next()
        .ok_or_else(|| "StableConcatPatternView binding is not a trait".to_string())?;
    if candidates.next().is_some()
        || !interface.type_parameters.is_empty()
        || interface.comptime_only
    {
        return Err(
            "StableConcatPatternView requires one runtime trait without explicit parameters".into(),
        );
    }
    if !interface.associated.is_empty() || !interface.methods.is_empty() {
        return Err("StableConcatPatternView marker trait must not declare extra members".into());
    }
    if let Some(concat_id) = concat_pattern
        && !interface
            .parents
            .iter()
            .any(|parent| parent.declaration == concat_id && parent.arguments.is_empty())
    {
        return Err(
            "StableConcatPatternView must require ConcatPatternView as a supertrait".into(),
        );
    }
    Ok(())
}

pub fn protocol(traits: &[Trait], id: DeclarationId) -> Result<Protocol, String> {
    let runtime = phase_protocol(traits, id, false)?;
    if has_staged_contract(traits, id) {
        if runtime.comptime {
            return Err("dual-phase ConcatPatternView requires runtime base methods".into());
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

pub fn has_staged_contract(traits: &[Trait], id: DeclarationId) -> bool {
    traits
        .iter()
        .filter(|item| item.declaration == id)
        .any(|item| {
            item.associated
                .iter()
                .any(|member| member.name == "PartComptime" || member.name == "RestComptime")
                || item.methods.iter().any(|method| {
                    matches!(
                        method.name.as_str(),
                        "concat_pattern_view_strip_prefix_comptime"
                            | "concat_pattern_view_strip_suffix_comptime"
                    )
                })
        })
}

fn phase_protocol(traits: &[Trait], id: DeclarationId, staged: bool) -> Result<Protocol, String> {
    let label = "ConcatPatternView";
    let (part_name, rest_name, prefix_name, suffix_name) = if staged {
        (
            "PartComptime",
            "RestComptime",
            "concat_pattern_view_strip_prefix_comptime",
            "concat_pattern_view_strip_suffix_comptime",
        )
    } else {
        (
            "Part",
            "Rest",
            "concat_pattern_view_strip_prefix",
            "concat_pattern_view_strip_suffix",
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

    let parts = interface
        .associated
        .iter()
        .filter(|item| item.name == part_name)
        .collect::<Vec<_>>();
    let [part] = parts.as_slice() else {
        return Err(format!("{label} requires one associated {part_name}"));
    };
    if part.ty != Term::Primitive(Primitive::Type) || (staged && part.phase != TraitPhase::Comptime)
    {
        return Err(format!("{label} {part_name} must be Type-valued"));
    }

    let rests = interface
        .associated
        .iter()
        .filter(|item| item.name == rest_name)
        .collect::<Vec<_>>();
    let [rest] = rests.as_slice() else {
        return Err(format!("{label} requires one associated {rest_name}"));
    };
    if rest.ty != Term::Primitive(Primitive::Type) || (staged && rest.phase != TraitPhase::Comptime)
    {
        return Err(format!("{label} {rest_name} must be Type-valued"));
    }

    let prefixes = interface
        .methods
        .iter()
        .filter(|method| method.name == prefix_name)
        .collect::<Vec<_>>();
    let [prefix_method] = prefixes.as_slice() else {
        return Err(format!("{label} requires one {prefix_name} method"));
    };
    let comptime = prefix_method.signature.is_comptime_callable();
    if staged && !comptime {
        return Err(format!(
            "{label} comptime methods must be comptime callables"
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
    let part_type = Term::AssociatedType {
        origin: Box::new(AssociatedTypeOrigin {
            interface: id,
            member: part.declaration,
        }),
        arguments: vec![],
        subject: Box::new(subject.clone()),
    };
    let rest_type = Term::AssociatedType {
        origin: Box::new(AssociatedTypeOrigin {
            interface: id,
            member: rest.declaration,
        }),
        arguments: vec![],
        subject: Box::new(subject),
    };
    let part_param = Parameter {
        sort: Sort::Runtime,
        ty: part_type,
    };
    let expected = Term::Function {
        parameters: vec![receiver, part_param],
        result: Box::new(Term::Optional(Box::new(rest_type))),
    };

    for (name, expected_name) in [(prefix_name, prefix_name), (suffix_name, suffix_name)] {
        let methods = interface
            .methods
            .iter()
            .filter(|method| method.name == expected_name)
            .collect::<Vec<_>>();
        let [method] = methods.as_slice() else {
            return Err(format!("{label} requires one {name} method"));
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

    Ok(Protocol {
        comptime,
        by_value: comptime,
        part_assoc: part.declaration,
        rest_assoc: rest.declaration,
        strip_prefix: prefix_name,
        strip_suffix: suffix_name,
    })
}
