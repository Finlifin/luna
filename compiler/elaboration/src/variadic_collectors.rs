//! Validate source-selected positional and extended variadic collector protocols.
use nir::program::{Trait, TraitPhase};
use nir::{AssociatedTypeOrigin, DeclarationId, Parameter, Primitive, Sort, Term};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CollectorKind {
    Positional,
    Extended,
}

impl CollectorKind {
    fn names(self) -> (&'static str, &'static str, &'static str, &'static str) {
        match self {
            Self::Positional => (
                "VariadicCollector",
                "variadic_init",
                "variadic_collect",
                "comptime_variadic_collect",
            ),
            Self::Extended => (
                "ExtendedVariadicCollector",
                "extended_variadic_init",
                "extended_variadic_collect",
                "comptime_extended_variadic_collect",
            ),
        }
    }
}

/// Borrow the exact declarations checked in this compilation's package.
pub struct Protocol {
    pub interface: DeclarationId,
    pub tmp: DeclarationId,
    pub init: &'static str,
    pub collect: &'static str,
    pub comptime_collect: &'static str,
}

pub fn protocol(
    traits: &[Trait],
    id: DeclarationId,
    kind: CollectorKind,
) -> Result<Protocol, String> {
    let (label, init_name, collect_name, comptime_collect_name) = kind.names();
    let mut candidates = traits.iter().filter(|item| item.declaration == id);
    let interface = candidates
        .next()
        .ok_or_else(|| format!("{label} binding is not a trait"))?;
    if candidates.next().is_some()
        || !interface.type_parameters.is_empty()
        || interface.comptime_only
    {
        return Err(format!(
            "{label} requires one runtime trait without explicit parameters"
        ));
    }

    let tmps = interface
        .associated
        .iter()
        .filter(|item| item.name == "Tmp")
        .collect::<Vec<_>>();
    let [tmp] = tmps.as_slice() else {
        return Err(format!("{label} requires one associated Tmp"));
    };
    if tmp.ty != Term::Primitive(Primitive::Type) || tmp.phase != TraitPhase::Runtime {
        return Err(format!(
            "{label} Tmp must be a runtime Type-valued associated type"
        ));
    }

    let subject = Term::Rigid {
        declaration: interface.self_parameter,
        sort: Sort::Type,
    };
    let tmp_type = Term::AssociatedType {
        origin: Box::new(AssociatedTypeOrigin {
            interface: id,
            member: tmp.declaration,
        }),
        arguments: vec![],
        subject: Box::new(subject.clone()),
    };

    // Runtime phase: `fn variadic_init() -> Tmp where S`
    let inits = interface
        .methods
        .iter()
        .filter(|method| method.name == init_name)
        .collect::<Vec<_>>();
    let [init] = inits.as_slice() else {
        return Err(format!("{label} requires one {init_name} method"));
    };
    let expected_init = Term::Function {
        parameters: vec![],
        result: Box::new(tmp_type.clone()),
    };
    if init.phase != TraitPhase::Runtime
        || init.signature.is_comptime_callable()
        || init.is_quote
        || init.by_value_receiver
        || init.by_reference_receiver
        || init.type_parameters.len() != 1
        || !init.type_arguments.is_empty()
        || !init.parameter_bindings.is_empty()
        || init.signature.callable_signature() != Ok(&expected_init)
    {
        return Err(format!(
            "{label} requires runtime fn {init_name}() -> Tmp where S"
        ));
    }

    // Runtime phase: `fn variadic_collect(tmp: *Tmp, args: S) -> Self where S`
    let collects = interface
        .methods
        .iter()
        .filter(|method| method.name == collect_name)
        .collect::<Vec<_>>();
    let [collect] = collects.as_slice() else {
        return Err(format!("{label} requires one {collect_name} method"));
    };
    let Some(&collect_s) = collect.type_parameters.first() else {
        return Err(format!(
            "{label} requires runtime fn {collect_name}(tmp: *Tmp, args: S) -> Self where S"
        ));
    };
    let expected_collect = Term::Function {
        parameters: vec![
            Parameter {
                sort: Sort::Runtime,
                ty: Term::Pointer(Box::new(tmp_type)),
            },
            Parameter {
                sort: Sort::Runtime,
                ty: Term::Rigid {
                    declaration: collect_s,
                    sort: Sort::Type,
                },
            },
        ],
        result: Box::new(subject.clone()),
    };
    if collect.phase != TraitPhase::Runtime
        || collect.signature.is_comptime_callable()
        || collect.is_quote
        || collect.by_value_receiver
        || collect.by_reference_receiver
        || collect.type_parameters.len() != 1
        || !collect.type_arguments.is_empty()
        || collect.parameter_bindings.len() != 2
        || collect.signature.callable_signature() != Ok(&expected_collect)
    {
        return Err(format!(
            "{label} requires runtime fn {collect_name}(tmp: *Tmp, args: S) -> Self where S"
        ));
    }

    // Comptime phase: `^attrs.comptime_phase comptime fn comptime_variadic_collect(args: S) -> Self where S`
    let comptime_collects = interface
        .methods
        .iter()
        .filter(|method| method.name == comptime_collect_name)
        .collect::<Vec<_>>();
    let [comptime_collect] = comptime_collects.as_slice() else {
        return Err(format!(
            "{label} requires one {comptime_collect_name} method"
        ));
    };
    let Some(&comptime_s) = comptime_collect.type_parameters.first() else {
        return Err(format!(
            "{label} requires ^attrs.comptime_phase comptime fn {comptime_collect_name}(args: S) -> Self where S"
        ));
    };
    let expected_comptime = Term::Function {
        parameters: vec![Parameter {
            sort: Sort::Runtime,
            ty: Term::Rigid {
                declaration: comptime_s,
                sort: Sort::Type,
            },
        }],
        result: Box::new(subject),
    };
    if comptime_collect.phase != TraitPhase::Comptime
        || !comptime_collect.signature.is_comptime_callable()
        || comptime_collect.is_quote
        || comptime_collect.by_value_receiver
        || comptime_collect.by_reference_receiver
        || comptime_collect.type_parameters.len() != 1
        || !comptime_collect.type_arguments.is_empty()
        || comptime_collect.parameter_bindings.len() != 1
        || comptime_collect.signature.callable_signature() != Ok(&expected_comptime)
    {
        return Err(format!(
            "{label} requires ^attrs.comptime_phase comptime fn {comptime_collect_name}(args: S) -> Self where S"
        ));
    }

    Ok(Protocol {
        interface: id,
        tmp: tmp.declaration,
        init: init_name,
        collect: collect_name,
        comptime_collect: comptime_collect_name,
    })
}
