use super::*;
use nir::{Sort, program::Struct};

fn id(local: u32) -> DeclarationId {
    DeclarationId { package: 71, local }
}
fn p(primitive: Primitive) -> Term {
    Term::Primitive(primitive)
}
fn nominal(arguments: Vec<Term>) -> Term {
    Term::Nominal {
        declaration: id(1),
        arguments,
    }
}
fn package() -> Package {
    let mut package = Package::default();
    package.structs.push(Struct {
        declaration: id(1),
        name: "Pair".into(),
        type_parameters: vec![id(2)],
        parameter_types: vec![],
        capabilities: vec![],
        requirements: vec![],
        fields: vec![
            (
                "first\n\"".into(),
                Term::Rigid {
                    declaration: id(2),
                    sort: Sort::Type,
                },
            ),
            (
                "next".into(),
                Term::Pointer(Box::new(nominal(vec![Term::Rigid {
                    declaration: id(2),
                    sort: Sort::Type,
                }]))),
            ),
        ],
    });
    package
}

#[test]
fn applied_fields_keep_order_identity_and_recursive_pointer_without_layout() {
    let package = package();
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    let ty = nominal(vec![p(Primitive::Bool)]);
    let view = vm.reflect_struct(&ty).unwrap();
    assert_eq!(view.subject, ty);
    assert_eq!(
        view.fields[0],
        ReflectedField {
            name: "first\n\"".into(),
            ty: p(Primitive::Bool)
        }
    );
    assert_eq!(
        view.fields[1],
        ReflectedField {
            name: "next".into(),
            ty: Term::Pointer(Box::new(ty.clone()))
        }
    );
    let display = view.to_string();
    assert!(display.starts_with("(struct-reflection (subject "));
    assert!(display.contains("first\\n\\\""), "{display}");
    assert_eq!(vm.reflect_struct(&ty).unwrap(), view);
    assert!(vm.events().is_empty());
    assert!(package.target.is_none());
}

#[test]
fn wrong_arity_values_unknown_types_and_runtime_phase_are_rejected() {
    let package = package();
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    for ty in [
        nominal(vec![]),
        nominal(vec![Term::Bool(true)]),
        Term::Nominal {
            declaration: id(99),
            arguments: vec![],
        },
        p(Primitive::Bool),
    ] {
        assert!(vm.reflect_struct(&ty).is_err(), "{ty}");
    }
    assert!(
        vm.reflect_struct(&nominal(vec![p(Primitive::Char)]))
            .is_ok()
    );
    let mut runtime = Interpreter::new(&package, Budget::default()).unwrap();
    assert_eq!(
        runtime
            .reflect_struct(&nominal(vec![p(Primitive::Bool)]))
            .unwrap_err()
            .kind,
        ErrorKind::PhaseMismatch
    );
}

#[test]
fn opaque_heads_are_preserved_inside_fields_and_not_unfolded_as_structs() {
    let mut package = package();
    package.normal_forms.push(nir::normal_forms::Head {
        declaration: id(3),
        role: nir::normal_forms::Role::Opaque,
        parameters: vec![],
        requirements: vec![],
    });
    let opaque = Term::NormalForm {
        head: id(3),
        sort: Sort::Type,
        arguments: vec![],
    };
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    let view = vm.reflect_struct(&nominal(vec![opaque.clone()])).unwrap();
    assert_eq!(view.fields[0].ty, opaque);
    assert_eq!(
        vm.reflect_struct(&opaque).unwrap_err().kind,
        ErrorKind::Type
    );
}

#[test]
fn exhaustion_does_not_modify_definition_and_fresh_budget_can_retry() {
    let package = package();
    let original = package.clone();
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    vm.budget.steps = 1;
    assert_eq!(
        vm.reflect_struct(&nominal(vec![p(Primitive::Bool)]))
            .unwrap_err()
            .kind,
        ErrorKind::StepsExhausted
    );
    assert_eq!(package, original);
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    assert_eq!(
        vm.reflect_struct(&nominal(vec![p(Primitive::Bool)]))
            .unwrap()
            .fields
            .len(),
        2
    );
}

#[test]
fn empty_struct_still_checks_phantom_inputs_and_formation_predicates() {
    let mut package = package();
    package.structs[0].fields.clear();
    package.structs[0].requirements = vec![nir::program::TypePredicate::Bool(false)];
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    assert_eq!(
        vm.reflect_struct(&nominal(vec![p(Primitive::Bool)]))
            .unwrap_err()
            .kind,
        ErrorKind::TypeContractRejected
    );
    package.structs[0].requirements.clear();
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    assert!(vm.reflect_struct(&nominal(vec![])).is_err());
    assert!(
        vm.reflect_struct(&nominal(vec![p(Primitive::Bool)]))
            .unwrap()
            .fields
            .is_empty()
    );
}

#[test]
fn large_field_names_consume_invocation_budget_before_copy() {
    let mut package = package();
    package.structs[0].fields[0].0 = "x".repeat(20_000);
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    vm.budget.steps = 10_000;
    assert_eq!(
        vm.reflect_struct(&nominal(vec![p(Primitive::Bool)]))
            .unwrap_err()
            .kind,
        ErrorKind::StepsExhausted
    );
}

#[test]
fn concrete_iir_reflection_agrees_with_declaration_world() {
    let package = package();
    let arguments = vec![p(Primitive::Bool)];
    let ty = nominal(arguments.clone());
    let mut definition = package.structs[0].clone();
    definition.fields = definition.instantiate_fields(&arguments).unwrap();
    definition.type_parameters.clear();
    definition.parameter_types.clear();
    let graph = iir::Package {
        nominals: vec![iir::NominalInstance {
            ty: ty.clone(),
            definition: iir::NominalDefinition::Struct(definition),
        }],
        ..iir::Package::default()
    };
    let mut normal = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    let mut concrete = Interpreter::from_iir_for_comptime(&graph, Budget::default()).unwrap();
    assert_eq!(
        normal.reflect_struct(&ty).unwrap(),
        concrete.reflect_struct(&ty).unwrap()
    );
}

#[test]
fn reflection_rejects_non_type_and_unbound_field_descriptions() {
    for field in [
        Term::Bool(true),
        Term::Rigid {
            declaration: id(99),
            sort: Sort::Type,
        },
    ] {
        let mut package = package();
        package.structs[0].fields = vec![("bad".into(), field)];
        // A malformed package may be rejected at construction; if admitted,
        // reflection itself must not publish this field as a completed Type.
        if let Ok(mut vm) = Interpreter::for_comptime(&package, Budget::default()) {
            assert!(
                vm.reflect_struct(&nominal(vec![p(Primitive::Bool)]))
                    .is_err()
            );
        }
    }
}

fn associated(member: u32) -> Term {
    Term::AssociatedType {
        origin: Box::new(nir::AssociatedTypeOrigin {
            interface: id(80),
            member: id(member),
        }),
        arguments: vec![],
        subject: Box::new(p(Primitive::Bool)),
    }
}
fn associated_key(member: u32) -> (InterfaceInstance, Term, DeclarationId) {
    (
        InterfaceInstance {
            declaration: id(80),
            arguments: vec![],
        },
        p(Primitive::Bool),
        id(member),
    )
}
fn selected_type(ty: Term) -> Expr {
    Expr {
        source: SourceRange { start: 0, end: 0 },
        ty: p(Primitive::Type),
        kind: K::TypeValue(ty),
    }
}

#[test]
fn selected_associated_subject_and_arguments_keep_original_query_identity() {
    let package = package();
    let applied = nominal(vec![p(Primitive::Char)]);
    let selected = selected_type(applied.clone());
    let element = selected_type(p(Primitive::Char));
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    vm.associated_values.insert(associated_key(81), &selected);
    vm.associated_values.insert(associated_key(82), &element);
    let root = vm.reflect_struct(&associated(81)).unwrap();
    assert_eq!(root.subject, associated(81));
    assert_eq!(root.fields[0].ty, p(Primitive::Char));
    let argument = nominal(vec![associated(82)]);
    let nested = vm.reflect_struct(&argument).unwrap();
    assert_eq!(nested.subject, argument);
    assert_eq!(nested.fields, root.fields);
}

#[test]
fn missing_invalid_and_cyclic_selection_fail_without_poisoning_later_queries() {
    let package = package();
    let cyclic = selected_type(associated(81));
    let invalid = Expr {
        source: SourceRange { start: 0, end: 0 },
        ty: p(Primitive::Bool),
        kind: K::Literal(Literal::Bool(true)),
    };
    let valid = selected_type(nominal(vec![p(Primitive::Bool)]));
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    assert_eq!(
        vm.reflect_struct(&associated(81)).unwrap_err().kind,
        ErrorKind::UnmaterializedDispatch
    );
    vm.associated_values.insert(associated_key(81), &invalid);
    assert_eq!(
        vm.reflect_struct(&associated(81)).unwrap_err().kind,
        ErrorKind::Type
    );
    vm.associated_values.insert(associated_key(81), &cyclic);
    assert_eq!(
        vm.reflect_struct(&associated(81)).unwrap_err().kind,
        ErrorKind::DepthExhausted
    );
    vm.associated_values.insert(associated_key(81), &valid);
    assert_eq!(
        vm.reflect_struct(&associated(81)).unwrap().fields[0].ty,
        p(Primitive::Bool)
    );
    vm.budget.steps = 1;
    assert_eq!(
        vm.reflect_struct(&associated(81)).unwrap_err().kind,
        ErrorKind::StepsExhausted
    );
}

#[test]
fn selected_field_queries_are_admitted_after_substitution() {
    let mut package = package();
    package.structs[0].fields = vec![("selected".into(), associated(81))];
    let selected = selected_type(p(Primitive::Bool));
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    assert_eq!(
        vm.reflect_struct(&nominal(vec![p(Primitive::Bool)]))
            .unwrap_err()
            .kind,
        ErrorKind::UnmaterializedDispatch
    );
    vm.associated_values.insert(associated_key(81), &selected);
    assert_eq!(
        vm.reflect_struct(&nominal(vec![p(Primitive::Bool)]))
            .unwrap()
            .fields[0]
            .ty,
        p(Primitive::Bool)
    );
}
