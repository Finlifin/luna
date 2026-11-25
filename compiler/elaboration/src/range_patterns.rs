//! Validate source-selected range endpoint and range pattern view protocols.
use nir::program::{Enum, Trait, TraitPhase, VariantShape};
use nir::{AssociatedTypeOrigin, DeclarationId, Parameter, Primitive, Sort, Term};
use std::collections::BTreeMap;

pub struct Protocol {
    pub comptime: bool,
    pub by_value: bool,
    pub bound: &'static str,
    pub contains: &'static str,
}

/// Validate `RangeEnd<T>` enum structure and return variant indices
/// `(unbounded, inclusive, exclusive)`.
pub fn validate_range_end(
    enums: &BTreeMap<DeclarationId, Enum>,
    id: DeclarationId,
) -> Result<(usize, usize, usize), String> {
    let def = enums
        .get(&id)
        .ok_or_else(|| "RangeEnd binding is not an enum".to_string())?;
    if def.is_intrinsic || def.type_parameters.len() != 1 || def.variants.len() != 3 {
        return Err(
            "RangeEnd must be a non-intrinsic enum with one type parameter and three variants"
                .into(),
        );
    }
    let param = Term::Rigid {
        declaration: def.type_parameters[0],
        sort: Sort::Type,
    };
    let mut unbounded = None;
    let mut inclusive = None;
    let mut exclusive = None;
    for (idx, variant) in def.variants.iter().enumerate() {
        match variant.name.as_str() {
            "unbounded"
                if matches!(variant.shape, VariantShape::Unit) && variant.fields.is_empty() =>
            {
                unbounded = Some(idx);
            }
            "inclusive"
                if matches!(variant.shape, VariantShape::Tuple)
                    && variant.fields.as_slice() == std::slice::from_ref(&param) =>
            {
                inclusive = Some(idx);
            }
            "exclusive"
                if matches!(variant.shape, VariantShape::Tuple)
                    && variant.fields.as_slice() == std::slice::from_ref(&param) =>
            {
                exclusive = Some(idx);
            }
            _ => {
                return Err(format!(
                    "RangeEnd variant `{}` does not match unbounded/inclusive(T)/exclusive(T)",
                    variant.name
                ));
            }
        }
    }
    match (unbounded, inclusive, exclusive) {
        (Some(u), Some(i), Some(e)) => Ok((u, i, e)),
        _ => Err("RangeEnd must declare unbounded, inclusive(T), and exclusive(T)".into()),
    }
}

pub fn stable_protocol(
    traits: &[Trait],
    id: DeclarationId,
    range_pattern: Option<DeclarationId>,
) -> Result<(), String> {
    let mut candidates = traits.iter().filter(|item| item.declaration == id);
    let interface = candidates
        .next()
        .ok_or_else(|| "StableRangePatternView binding is not a trait".to_string())?;
    if candidates.next().is_some()
        || !interface.type_parameters.is_empty()
        || interface.comptime_only
    {
        return Err(
            "StableRangePatternView requires one runtime trait without explicit parameters".into(),
        );
    }
    if !interface.associated.is_empty() || !interface.methods.is_empty() {
        return Err("StableRangePatternView marker trait must not declare extra members".into());
    }
    if let Some(range_id) = range_pattern
        && !interface
            .parents
            .iter()
            .any(|parent| parent.declaration == range_id && parent.arguments.is_empty())
    {
        return Err("StableRangePatternView must require RangePatternView as a supertrait".into());
    }
    Ok(())
}

pub fn has_staged_contract(traits: &[Trait], id: DeclarationId) -> bool {
    traits
        .iter()
        .filter(|item| item.declaration == id)
        .any(|item| {
            item.associated
                .iter()
                .any(|member| member.name == "BoundComptime")
                || item
                    .methods
                    .iter()
                    .any(|method| method.name == "range_pattern_view_contains_comptime")
        })
}

pub fn protocol(
    traits: &[Trait],
    id: DeclarationId,
    range_end: Option<DeclarationId>,
) -> Result<Protocol, String> {
    let runtime = phase_protocol(traits, id, range_end, false)?;
    if has_staged_contract(traits, id) {
        if runtime.comptime {
            return Err("dual-phase RangePatternView requires runtime base methods".into());
        }
        phase_protocol(traits, id, range_end, true)?;
    }
    Ok(runtime)
}

pub fn for_phase(
    traits: &[Trait],
    id: DeclarationId,
    range_end: Option<DeclarationId>,
    comptime: bool,
) -> Result<Protocol, String> {
    let base = protocol(traits, id, range_end)?;
    if comptime && has_staged_contract(traits, id) {
        phase_protocol(traits, id, range_end, true)
    } else {
        Ok(base)
    }
}

fn phase_protocol(
    traits: &[Trait],
    id: DeclarationId,
    range_end: Option<DeclarationId>,
    staged: bool,
) -> Result<Protocol, String> {
    let label = "RangePatternView";
    let (bound_name, contains_name) = if staged {
        ("BoundComptime", "range_pattern_view_contains_comptime")
    } else {
        ("Bound", "range_pattern_view_contains")
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
    let bounds = interface
        .associated
        .iter()
        .filter(|item| item.name == bound_name)
        .collect::<Vec<_>>();
    let [bound] = bounds.as_slice() else {
        return Err(format!("{label} requires one associated {bound_name}"));
    };
    if bound.ty != Term::Primitive(Primitive::Type)
        || (staged && bound.phase != TraitPhase::Comptime)
    {
        return Err(format!("{label} {bound_name} must be Type-valued"));
    }
    let methods = interface
        .methods
        .iter()
        .filter(|method| method.name == contains_name)
        .collect::<Vec<_>>();
    let [method] = methods.as_slice() else {
        return Err(format!("{label} requires one {contains_name} method"));
    };
    let comptime = method.signature.is_comptime_callable();
    if staged && !comptime {
        return Err(format!(
            "{label} comptime methods must be comptime callables"
        ));
    }
    let range_end_id =
        range_end.ok_or_else(|| format!("{label} binding requires a RangeEnd binding"))?;
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
    let bound_type = Term::AssociatedType {
        origin: Box::new(AssociatedTypeOrigin {
            interface: id,
            member: bound.declaration,
        }),
        arguments: vec![],
        subject: Box::new(subject),
    };
    let end_param = Parameter {
        sort: Sort::Runtime,
        ty: Term::Nominal {
            declaration: range_end_id,
            arguments: vec![bound_type],
        },
    };
    let expected = Term::Function {
        parameters: vec![receiver, end_param.clone(), end_param],
        result: Box::new(Term::Primitive(Primitive::Bool)),
    };
    if method.signature.callable_signature() != Ok(&expected)
        || method.by_reference_receiver == comptime
        || method.by_value_receiver != comptime
        || (staged && method.phase != TraitPhase::Comptime)
        || !method.type_parameters.is_empty()
        || !method.type_arguments.is_empty()
    {
        return Err(format!("invalid {label} {contains_name} signature"));
    }
    Ok(Protocol {
        comptime,
        by_value: comptime,
        bound: bound_name,
        contains: contains_name,
    })
}
