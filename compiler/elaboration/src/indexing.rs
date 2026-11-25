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
    let self_ty = Term::Rigid {
        declaration: interface.self_parameter,
        sort: Sort::Type,
    };
    let expected = [
        Parameter {
            sort: Sort::Runtime,
            ty: Term::Pointer(Box::new(self_ty)),
        },
        Parameter {
            sort: Sort::Runtime,
            ty: Term::Rigid {
                declaration: *argument,
                sort: Sort::Type,
            },
        },
    ];
    let Ok(Term::Function { parameters, .. }) = method.signature.callable_signature() else {
        return Err("Index method must be callable".into());
    };
    if parameters != &expected
        || method.by_value_receiver
        || !method.by_reference_receiver
        || method.parameter_bindings.len() != 2
        || !method.type_parameters.is_empty()
        || !method.type_arguments.is_empty()
    {
        return Err("Index protocol requires fn index(*self, idx: Idx)".into());
    }
    Ok(method)
}

pub fn index_mut_protocol(
    traits: &[Trait],
    id: DeclarationId,
    index_id: Option<DeclarationId>,
) -> Result<&MethodContract, String> {
    let Some(index_id) = index_id else {
        return Err("IndexMut protocol requires an Index protocol binding".into());
    };
    let index_trait = traits
        .iter()
        .find(|item| item.declaration == index_id)
        .ok_or_else(|| "Index binding is not a trait".to_string())?;
    let output_member = index_trait
        .associated
        .iter()
        .find(|item| item.name == "Output")
        .ok_or_else(|| "Index protocol requires an associated Output".to_string())?;
    let mut candidates = traits.iter().filter(|item| item.declaration == id);
    let interface = candidates
        .next()
        .ok_or_else(|| "IndexMut binding is not a trait".to_string())?;
    if candidates.next().is_some() {
        return Err("IndexMut binding matches multiple trait declarations".into());
    }
    let [argument] = interface.type_parameters.as_slice() else {
        return Err("IndexMut protocol requires one trait parameter".into());
    };
    let idx_ty = Term::Rigid {
        declaration: *argument,
        sort: Sort::Type,
    };
    let expected_parent = nir::program::InterfaceInstance {
        declaration: index_id,
        arguments: vec![idx_ty.clone()],
    };
    if !interface.parents.contains(&expected_parent) {
        return Err("IndexMut protocol requires Index<Idx> supertrait".into());
    }
    let mut methods = interface.methods.iter().filter(|m| m.name == "index_mut");
    let method = methods
        .next()
        .ok_or_else(|| "IndexMut protocol requires an index_mut method".to_string())?;
    if methods.next().is_some() {
        return Err("IndexMut protocol allows only one index_mut method".into());
    }
    if method.signature.is_comptime_callable() || method.is_quote {
        return Err("IndexMut index_mut must be a runtime function".into());
    }
    let self_ty = Term::Rigid {
        declaration: interface.self_parameter,
        sort: Sort::Type,
    };
    let expected = [
        Parameter {
            sort: Sort::Runtime,
            ty: Term::Pointer(Box::new(self_ty.clone())),
        },
        Parameter {
            sort: Sort::Runtime,
            ty: idx_ty.clone(),
        },
    ];
    let expected_result = Term::Pointer(Box::new(Term::AssociatedType {
        origin: Box::new(nir::AssociatedTypeOrigin {
            interface: index_id,
            member: output_member.declaration,
        }),
        arguments: vec![idx_ty],
        subject: Box::new(self_ty),
    }));
    let Ok(Term::Function { parameters, result }) = method.signature.callable_signature() else {
        return Err("IndexMut method must be callable".into());
    };
    if parameters != &expected
        || result.as_ref() != &expected_result
        || method.by_value_receiver
        || !method.by_reference_receiver
        || method.parameter_bindings.len() != 2
        || !method.type_parameters.is_empty()
        || !method.type_arguments.is_empty()
    {
        return Err(
            "IndexMut protocol requires fn index_mut(*self, idx: Idx) -> *Self.as(Index<Idx>).Output"
                .into(),
        );
    }
    Ok(method)
}

pub fn space_apply_protocol(traits: &[Trait], id: DeclarationId) -> Result<(), String> {
    let mut candidates = traits.iter().filter(|item| item.declaration == id);
    let interface = candidates
        .next()
        .ok_or_else(|| "SpaceApply binding is not a trait".to_string())?;
    if candidates.next().is_some() || !interface.type_parameters.is_empty() {
        return Err("SpaceApply protocol requires one trait without explicit parameters".into());
    }
    let mut collect_tos = interface
        .associated
        .iter()
        .filter(|a| a.name == "CollectTo");
    let collect_to = collect_tos
        .next()
        .ok_or_else(|| "SpaceApply requires an associated CollectTo".to_string())?;
    if collect_tos.next().is_some() || collect_to.ty != Term::Primitive(nir::Primitive::Type) {
        return Err("SpaceApply requires one Type-valued CollectTo".into());
    }
    let mut targets = interface.associated.iter().filter(|a| a.name == "Target");
    let target = targets
        .next()
        .ok_or_else(|| "SpaceApply requires an associated Target".to_string())?;
    if targets.next().is_some() || target.ty != Term::Primitive(nir::Primitive::Type) {
        return Err("SpaceApply requires one Type-valued Target".into());
    }
    let mut methods = interface.methods.iter().filter(|m| m.name == "space_apply");
    let method = methods
        .next()
        .ok_or_else(|| "SpaceApply protocol requires a space_apply method".to_string())?;
    if methods.next().is_some() {
        return Err("SpaceApply protocol allows only one space_apply method".into());
    }
    if method.signature.is_comptime_callable() || method.is_quote {
        return Err("SpaceApply space_apply must be a runtime function".into());
    }
    let self_ty = Term::Rigid {
        declaration: interface.self_parameter,
        sort: Sort::Type,
    };
    let collect_to_ty = Term::AssociatedType {
        origin: Box::new(nir::AssociatedTypeOrigin {
            interface: id,
            member: collect_to.declaration,
        }),
        arguments: vec![],
        subject: Box::new(self_ty.clone()),
    };
    let target_ty = Term::AssociatedType {
        origin: Box::new(nir::AssociatedTypeOrigin {
            interface: id,
            member: target.declaration,
        }),
        arguments: vec![],
        subject: Box::new(self_ty.clone()),
    };
    let expected = [
        Parameter {
            sort: Sort::Runtime,
            ty: Term::Pointer(Box::new(self_ty)),
        },
        Parameter {
            sort: Sort::Runtime,
            ty: collect_to_ty,
        },
    ];
    let Ok(Term::Function { parameters, result }) = method.signature.callable_signature() else {
        return Err("SpaceApply method must be callable".into());
    };
    if parameters != &expected
        || result.as_ref() != &target_ty
        || method.by_value_receiver
        || !method.by_reference_receiver
        || method.parameter_bindings.len() != 2
        || !method.type_parameters.is_empty()
        || !method.type_arguments.is_empty()
    {
        return Err(
            "SpaceApply protocol requires fn space_apply(*self, index: CollectTo) -> Target".into(),
        );
    }
    Ok(())
}
