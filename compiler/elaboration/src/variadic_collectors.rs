//! Validate source-selected positional, fallible, and extended variadic collector protocols.
use nir::program::{Trait, TraitPhase};
use nir::{
    AssociatedTypeCall, AssociatedTypeOrigin, DeclarationId, Parameter, Primitive, Sort, Term,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CollectorKind {
    Positional,
    TryPositional,
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
            Self::TryPositional => (
                "TryVariadicCollector",
                "try_variadic_init",
                "try_variadic_collect",
                "comptime_try_variadic_collect",
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
    pub err: Option<DeclarationId>,
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
    let expected_tmp_ty = Term::NormalFormFunction(
        Box::new(Term::Function {
            parameters: vec![Parameter {
                sort: Sort::Type,
                ty: Term::Primitive(Primitive::Type),
            }],
            result: Box::new(Term::Primitive(Primitive::Type)),
        }),
        None,
    );
    if tmp.ty != expected_tmp_ty || tmp.phase != TraitPhase::Runtime {
        return Err(format!(
            "{label} Tmp must have type for<S: Type> Type in runtime phase"
        ));
    }

    let err = if kind == CollectorKind::TryPositional {
        let errs = interface
            .associated
            .iter()
            .filter(|item| item.name == "Err")
            .collect::<Vec<_>>();
        let [err] = errs.as_slice() else {
            return Err(format!("{label} requires one associated Err"));
        };
        if err.ty != Term::Primitive(Primitive::Type) || err.phase != TraitPhase::Runtime {
            return Err(format!(
                "{label} Err must be a runtime Type-valued associated type"
            ));
        }
        Some(err.declaration)
    } else {
        None
    };

    let subject = Term::Rigid {
        declaration: interface.self_parameter,
        sort: Sort::Type,
    };
    let tmp_type = |param: DeclarationId| {
        Term::AssociatedTypeCall(Box::new(AssociatedTypeCall {
            origin: AssociatedTypeOrigin {
                interface: id,
                member: tmp.declaration,
            },
            arguments: vec![],
            subject: subject.clone(),
            call_arguments: vec![Term::Rigid {
                declaration: param,
                sort: Sort::Type,
            }],
        }))
    };
    let result_type = match err {
        Some(err_decl) => Term::ErrorResult {
            errors: vec![Term::AssociatedType {
                origin: Box::new(AssociatedTypeOrigin {
                    interface: id,
                    member: err_decl,
                }),
                arguments: vec![],
                subject: Box::new(subject.clone()),
            }],
            result: Box::new(subject.clone()),
        },
        None => subject.clone(),
    };

    // Runtime phase: `fn variadic_init() -> Tmp<S> where S`
    let inits = interface
        .methods
        .iter()
        .filter(|method| method.name == init_name)
        .collect::<Vec<_>>();
    let [init] = inits.as_slice() else {
        return Err(format!("{label} requires one {init_name} method"));
    };
    let Some(&init_s) = init.type_parameters.first() else {
        return Err(format!(
            "{label} requires runtime fn {init_name}() -> Tmp<S> where S"
        ));
    };
    let expected_init = Term::Function {
        parameters: vec![],
        result: Box::new(tmp_type(init_s)),
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
            "{label} requires runtime fn {init_name}() -> Tmp<S> where S"
        ));
    }

    // Runtime phase: `fn variadic_collect(tmp: *Tmp<S>, args: S) -> Self (or !Err Self) where S`
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
            "{label} requires runtime fn {collect_name}(tmp: *Tmp<S>, args: S) where S"
        ));
    };
    let expected_collect = Term::Function {
        parameters: vec![
            Parameter {
                sort: Sort::Runtime,
                ty: Term::Pointer(Box::new(tmp_type(collect_s))),
            },
            Parameter {
                sort: Sort::Runtime,
                ty: Term::Rigid {
                    declaration: collect_s,
                    sort: Sort::Type,
                },
            },
        ],
        result: Box::new(result_type.clone()),
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
            "{label} requires runtime fn {collect_name}(tmp: *Tmp<S>, args: S) where S"
        ));
    }

    // Comptime phase: `^attrs.comptime_phase comptime fn comptime_variadic_collect(args: S) -> Self (or !Err Self) where S`
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
            "{label} requires ^attrs.comptime_phase comptime fn {comptime_collect_name}(args: S) where S"
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
        result: Box::new(result_type),
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
            "{label} requires ^attrs.comptime_phase comptime fn {comptime_collect_name}(args: S) where S"
        ));
    }

    Ok(Protocol {
        interface: id,
        tmp: tmp.declaration,
        err,
        init: init_name,
        collect: collect_name,
        comptime_collect: comptime_collect_name,
    })
}
