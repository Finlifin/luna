//! Source-bound indexing: selection is ordinary method selection. The caller
//! retains the declared result, including any pointer, phase and effect row.
use nir::program::{MethodContract, Trait};
use nir::{DeclarationId, Parameter, Sort, Term};

pub fn protocol(traits: &[Trait], id: DeclarationId) -> Result<&MethodContract, String> {
    let mut declarations = traits.iter().filter(|item| item.declaration == id);
    let interface = declarations
        .next()
        .ok_or("Index protocol is not a declared trait")?;
    if declarations.next().is_some() {
        return Err("duplicate Index protocol identity".into());
    }
    let [argument] = interface.type_parameters.as_slice() else {
        return Err("Index protocol requires one index type parameter".into());
    };
    let methods = interface
        .methods
        .iter()
        .filter(|method| method.name == "index")
        .collect::<Vec<_>>();
    let [method] = methods.as_slice() else {
        return Err("Index protocol requires one index method".into());
    };
    let expected = [
        Parameter {
            sort: Sort::Runtime,
            ty: Term::Rigid {
                declaration: interface.self_parameter,
                sort: Sort::Type,
            },
        },
        Parameter {
            sort: Sort::Runtime,
            ty: Term::Rigid {
                declaration: *argument,
                sort: Sort::Type,
            },
        },
    ];
    let Term::Function { parameters, .. } = method.signature.callable_signature() else {
        return Err("Index method must be callable".into());
    };
    if parameters != &expected
        || !method.by_value_receiver
        || method.by_reference_receiver
        || method.parameter_bindings.len() != 2
        || !method.type_parameters.is_empty()
        || !method.type_arguments.is_empty()
    {
        return Err("Index protocol requires fn index(self, idx: Idx)".into());
    }
    Ok(method)
}
