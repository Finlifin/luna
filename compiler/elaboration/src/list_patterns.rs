//! Validate source-selected list observation methods before synthesizing calls.
use nir::program::Trait;
use nir::{AssociatedTypeOrigin, DeclarationId, Parameter, Primitive, Sort, Term};

pub struct Protocol {
    pub size: Term,
    pub comptime: bool,
}

pub fn protocol(traits: &[Trait], id: DeclarationId) -> Result<Protocol, String> {
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
        .filter(|item| item.name == "Element")
        .collect::<Vec<_>>();
    let [element] = elements.as_slice() else {
        return Err("ListPatternView requires one associated Element".into());
    };
    if element.ty != Term::Primitive(Primitive::Type) {
        return Err("ListPatternView Element must be Type-valued".into());
    }
    let lengths = interface
        .methods
        .iter()
        .filter(|method| method.name == "list_pattern_view_get_len")
        .collect::<Vec<_>>();
    let [length] = lengths.as_slice() else {
        return Err("ListPatternView requires one length method".into());
    };
    let Ok(Term::Function { result, .. }) = length.signature.callable_signature() else {
        return Err("ListPatternView length method is not callable".into());
    };
    let size = result.as_ref().clone();
    let comptime = length.signature.is_comptime_callable();
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
        ty: Term::Pointer(Box::new(subject.clone())),
    };
    let element_type = Term::AssociatedType {
        origin: Box::new(AssociatedTypeOrigin {
            interface: id,
            member: element.declaration,
        }),
        arguments: vec![],
        subject: Box::new(subject),
    };
    for (name, parameters, result) in [
        (
            "list_pattern_view_get_len",
            vec![receiver.clone()],
            size.clone(),
        ),
        (
            "list_pattern_view_get_element",
            vec![
                receiver,
                Parameter {
                    sort: Sort::ComptimeValue,
                    ty: size.clone(),
                },
            ],
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
            || !method.by_reference_receiver
            || method.by_value_receiver
            || !method.type_parameters.is_empty()
            || !method.type_arguments.is_empty()
        {
            return Err(format!("invalid ListPatternView {name} signature"));
        }
    }
    Ok(Protocol { size, comptime })
}
