use super::*;
use nir::program::{Enum, Variant, VariantShape};

fn id(local: u32) -> DeclarationId {
    DeclarationId { package: 89, local }
}
fn ty(local: u32) -> Term {
    Term::Nominal {
        declaration: id(local),
        arguments: vec![],
    }
}
fn boolean() -> Term {
    Term::Primitive(Primitive::Bool)
}
fn enumeration(local: u32) -> Enum {
    Enum {
        declaration: id(local),
        name: format!("Generated{local}"),
        is_intrinsic: false,
        type_parameters: vec![],
        parameter_types: vec![],
        capabilities: vec![],
        requirements: vec![],
        groups: vec!["Group".into()],
        encoding: None,
        symbol_encoding: None,
        variants: vec![
            Variant {
                name: "Ready".into(),
                shape: VariantShape::Unit,
                fields: vec![],
            },
            Variant {
                name: "Group.Data".into(),
                shape: VariantShape::Named(vec!["flag".into()]),
                fields: vec![boolean()],
            },
        ],
    }
}
fn package() -> Package {
    Package {
        enums: vec![enumeration(10)],
        ..Default::default()
    }
}

#[test]
fn owned_enum_publication_retains_structure_and_executes_new_variants() {
    let package = package();
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    let definitions: Arc<[Enum]> = vec![enumeration(20)].into();
    let weak = Arc::downgrade(&definitions);
    let before = vm.remaining_steps();
    vm.install_owned_generated_enums_with(definitions, || Ok::<_, ()>(()))
        .unwrap();
    assert!(vm.remaining_steps() < before);
    assert!(weak.upgrade().is_some());
    let reflected = vm.reflect_enum(&ty(20)).unwrap();
    assert_eq!(reflected.groups, ["Group"]);
    assert_eq!(reflected.variants[1].fields, [boolean()]);
    assert!(reflected.to_string().contains("(named \"flag\")"));
    let source = SourceRange { start: 1, end: 2 };
    let value = Expr {
        source,
        ty: ty(20),
        kind: K::Variant {
            declaration: id(20),
            index: 1,
            fields: vec![Expr {
                source,
                ty: boolean(),
                kind: K::Literal(Literal::Bool(true)),
            }],
        },
    };
    let Flow::Value(constructed) = vm.eval(&value, &mut Environment::new()).unwrap() else {
        panic!("expected variant")
    };
    assert_eq!(
        constructed,
        Value::Variant {
            declaration: id(20),
            index: 1,
            fields: vec![Value::Bool(true)],
            arguments: vec![]
        }
    );
    let name = Expr {
        source,
        ty: Term::Primitive(Primitive::Str),
        kind: K::EnumName(Box::new(value)),
    };
    let Flow::Value(name) = vm.eval(&name, &mut Environment::new()).unwrap() else {
        panic!("expected name")
    };
    assert_eq!(name, Value::String("Group.Data".into()));
    assert!(vm.reflect_enum(&ty(10)).is_ok());
    drop(vm);
    assert!(weak.upgrade().is_none());
}

#[test]
fn enum_batches_admit_forward_pointer_references_and_reject_unknown_payloads() {
    let package = package();
    let mut first = enumeration(20);
    first.variants[1].fields = vec![Term::Pointer(Box::new(ty(21)))];
    let mut second = enumeration(21);
    second.variants[1].fields = vec![Term::Pointer(Box::new(ty(20)))];
    let mut invalid = second.clone();
    invalid.variants[1].fields = vec![ty(99)];
    let bad = [first.clone(), invalid];
    let good = [first, second];
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    let committed = std::cell::Cell::new(false);
    assert!(
        vm.install_generated_enums_with(&bad, || {
            committed.set(true);
            Ok::<_, ()>(())
        })
        .is_err()
    );
    assert!(!committed.get());
    assert!(!vm.enums.contains_key(&id(20)) && !vm.enums.contains_key(&id(21)));
    vm.install_generated_enums(&good).unwrap();
    assert_eq!(
        vm.reflect_enum(&ty(20)).unwrap().variants[1].fields,
        [Term::Pointer(Box::new(ty(21)))]
    );
}

#[test]
fn enum_commit_error_and_unwind_remove_only_the_candidate_batch() {
    let package = package();
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    for panic in [false, true] {
        let definitions: Arc<[Enum]> = vec![enumeration(20)].into();
        let weak = Arc::downgrade(&definitions);
        let before = vm.remaining_steps();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            vm.install_owned_generated_enums_with(definitions, || -> Result<(), &str> {
                if panic {
                    panic!("publisher unwind");
                }
                Err("publisher rejected")
            })
        }));
        if panic {
            assert!(result.is_err());
        } else {
            assert!(matches!(
                result.unwrap(),
                Err(EnumPublicationError::Commit("publisher rejected"))
            ));
        }
        assert!(vm.remaining_steps() < before);
        assert!(weak.upgrade().is_none());
        assert!(!vm.enums.contains_key(&id(20)));
        assert!(vm.reflect_enum(&ty(10)).is_ok());
    }
    vm.install_owned_generated_enums_with(vec![enumeration(20)].into(), || Ok::<_, ()>(()))
        .unwrap();
    assert!(vm.reflect_enum(&ty(20)).is_ok());
}

#[test]
fn enum_shape_identity_and_scope_failures_do_not_replace_existing_definitions() {
    let package = package();
    let mut duplicate_name = enumeration(20);
    duplicate_name.variants[1].name = "Ready".into();
    let mut wrong_arity = enumeration(20);
    wrong_arity.variants[1].fields.clear();
    let mut unit_payload = enumeration(20);
    unit_payload.variants[0].fields.push(boolean());
    let mut missing_group = enumeration(20);
    missing_group.groups.clear();
    let mut generic = enumeration(20);
    generic.type_parameters.push(id(90));
    let mut intrinsic = enumeration(20);
    intrinsic.is_intrinsic = true;
    let mut open = enumeration(20);
    open.variants[1].fields = vec![Term::Bound {
        depth: 0,
        index: 0,
        sort: nir::Sort::Type,
    }];
    for definitions in [
        vec![enumeration(10)],
        vec![enumeration(20), enumeration(20)],
        vec![duplicate_name],
        vec![wrong_arity],
        vec![unit_payload],
        vec![missing_group],
        vec![generic],
        vec![intrinsic],
        vec![open],
    ] {
        let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
        assert!(vm.install_generated_enums(&definitions).is_err());
        assert!(!vm.enums.contains_key(&id(20)));
        assert_eq!(vm.reflect_enum(&ty(10)).unwrap().variants.len(), 2);
    }
}

#[test]
fn enum_formation_phase_and_cross_kind_collisions_are_checked() {
    let mut package = package();
    let structure = nir::program::Struct {
        declaration: id(30),
        name: "ExistingStruct".into(),
        type_parameters: vec![],
        parameter_types: vec![],
        requirements: vec![],
        capabilities: vec![],
        fields: vec![],
    };
    package.structs.push(structure.clone());
    let collision = [enumeration(30)];
    let definitions = [enumeration(20)];
    let mut impossible = enumeration(20);
    impossible.variants.clear();
    impossible.groups.clear();
    impossible
        .requirements
        .push(nir::program::TypePredicate::Bool(false));
    let impossible = [impossible];
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    assert_eq!(
        vm.install_generated_enums(&collision).unwrap_err().kind,
        ErrorKind::InvalidCallPlan
    );
    assert!(vm.install_generated_enums(&impossible).is_err());
    assert!(!vm.enums.contains_key(&id(20)));
    vm.install_generated_enums(&definitions).unwrap();
    let mut structure = structure;
    structure.declaration = id(20);
    let structures = [structure];
    assert_eq!(
        vm.install_generated_structs(&structures).unwrap_err().kind,
        ErrorKind::InvalidCallPlan
    );
    assert!(vm.reflect_enum(&ty(20)).is_ok());

    let mut runtime = Interpreter::new(&package, Budget::default()).unwrap();
    assert_eq!(
        runtime
            .install_generated_enums(&definitions)
            .unwrap_err()
            .kind,
        ErrorKind::PhaseMismatch
    );
    let mut abstract_vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    abstract_vm
        .authorize_abstract_type_parameters(&[id(90)])
        .unwrap();
    assert_eq!(
        abstract_vm
            .install_generated_enums(&definitions)
            .unwrap_err()
            .kind,
        ErrorKind::Type
    );
}

#[test]
fn enum_solver_unwind_rolls_back_forward_visible_candidates() {
    let package = package();
    let mut conditional = enumeration(21);
    conditional
        .requirements
        .push(nir::program::TypePredicate::Equal(
            boolean(),
            Term::Primitive(Primitive::Char),
        ));
    let definitions = [enumeration(20), conditional];
    let solver = |_: &nir::program::TypePredicate| -> ContractOutcome { panic!("solver unwind") };
    let mut vm = Interpreter::for_comptime(&package, Budget::default())
        .unwrap()
        .with_type_solver(&solver);
    assert!(
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(
            || vm.install_generated_enums(&definitions)
        ))
        .is_err()
    );
    assert!(!vm.enums.contains_key(&id(20)) && !vm.enums.contains_key(&id(21)));
    assert!(vm.reflect_enum(&ty(10)).is_ok());
}

#[test]
fn every_insufficient_enum_admission_budget_rolls_back_and_can_retry() {
    let package = package();
    let definitions = [enumeration(20), enumeration(21)];
    let mut measured = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    let before = measured.remaining_steps();
    measured.install_generated_enums(&definitions).unwrap();
    let cost = before - measured.remaining_steps();
    for steps in 0..cost {
        let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
        vm.budget.steps = steps;
        let failure = vm.install_generated_enums(&definitions).unwrap_err();
        assert_eq!(failure.kind, ErrorKind::StepsExhausted);
        assert!(!vm.enums.contains_key(&id(20)) && !vm.enums.contains_key(&id(21)));
        vm.budget.steps = 100_000;
        vm.install_generated_enums(&definitions).unwrap();
        assert!(vm.reflect_enum(&ty(21)).is_ok());
    }
}

#[test]
fn source_session_shares_layouts_but_isolates_generated_enum_publication() {
    let package = Arc::new(package());
    let mut session = CodeSession::new(package.clone());
    let mut first =
        Interpreter::for_comptime_in_session(&package, &mut session, &mut Budget::default())
            .unwrap();
    let base = first.enums.base.clone();
    assert!(std::ptr::eq(
        &**first.enums.get(&id(10)).unwrap(),
        &package.enums[0]
    ));
    first
        .install_owned_generated_enums_with(vec![enumeration(20)].into(), || Ok::<_, ()>(()))
        .unwrap();
    assert!(first.reflect_enum(&ty(20)).is_ok());
    let mut next =
        Interpreter::for_comptime_in_session(&package, &mut session, &mut Budget::default())
            .unwrap();
    assert!(Arc::ptr_eq(&base, &next.enums.base));
    assert!(next.reflect_enum(&ty(20)).is_err());
    assert!(
        next.install_owned_generated_enums_with(vec![enumeration(21)].into(), || Err::<(), _>(
            "publisher rejected"
        ),)
            .is_err()
    );
    assert!(!next.enums.contains_key(&id(21)));
    assert_eq!(next.reflect_enum(&ty(10)).unwrap().variants.len(), 2);
    assert!(
        next.install_owned_generated_enums_with(vec![enumeration(10)].into(), || Ok::<_, ()>(()))
            .is_err()
    );
    assert_eq!(next.reflect_enum(&ty(10)).unwrap().variants.len(), 2);
}
