use super::*;
use nir::newtypes::Newtype;
fn id(local: u32) -> DeclarationId {
    DeclarationId { package: 92, local }
}
fn boolean() -> Term {
    Term::Primitive(Primitive::Bool)
}
fn ty(local: u32) -> Term {
    Term::Nominal {
        declaration: id(local),
        arguments: vec![],
    }
}
fn definition(local: u32, representation: Term) -> Newtype {
    Newtype {
        declaration: id(local),
        name: format!("Generated{local}"),
        type_parameters: vec![],
        parameter_types: vec![],
        capture_count: 0,
        requirements: vec![],
        capabilities: vec![],
        representation,
    }
}
#[test]
fn owned_definitions_execute_conversions_in_both_carriers_and_release_with_vm() {
    let package = Package::default();
    for compiled in [false, true] {
        let definitions: Arc<[Newtype]> = vec![definition(1, boolean())].into();
        let weak = Arc::downgrade(&definitions);
        let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
        vm.install_owned_generated_newtypes_with(definitions, || Ok::<_, ()>(()))
            .unwrap();
        let source = SourceRange { start: 1, end: 2 };
        let wrap = Expr {
            source,
            ty: ty(1),
            kind: K::NewtypeConvert(Box::new(Expr {
                source,
                ty: boolean(),
                kind: K::Literal(Literal::Bool(true)),
            })),
        };
        let unwrap = Expr {
            source,
            ty: boolean(),
            kind: K::NewtypeConvert(Box::new(wrap.clone())),
        };
        for (body, expected) in [
            (
                wrap,
                Value::Newtype {
                    ty: ty(1),
                    value: Box::new(Value::Bool(true)),
                },
            ),
            (unwrap, Value::Bool(true)),
        ] {
            let flow = if compiled {
                let code = Arc::new(comptime_compiler::compile(&body, &mut 1000).unwrap());
                vm.execute_code(&code, &mut Environment::new())
            } else {
                vm.eval(&body, &mut Environment::new())
            }
            .unwrap();
            let Flow::Value(value) = flow else {
                panic!("value")
            };
            assert_eq!(value, expected);
        }
        assert!(weak.upgrade().is_some());
        drop(vm);
        assert!(weak.upgrade().is_none());
    }
}
#[test]
fn unknown_representations_and_failed_commits_leave_no_partial_batch() {
    let package = Package {
        newtypes: vec![definition(1, boolean())],
        ..Default::default()
    };
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    let invalid = [definition(2, ty(3)), definition(3, ty(99))];
    assert!(vm.install_generated_newtypes(&invalid).is_err());
    assert!(!vm.newtypes.contains_key(&id(2)) && !vm.newtypes.contains_key(&id(3)));
    for panic in [false, true] {
        let definitions: Arc<[Newtype]> = vec![definition(2, ty(1))].into();
        let weak = Arc::downgrade(&definitions);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            vm.install_owned_generated_newtypes_with(definitions, || -> Result<(), &str> {
                if panic {
                    panic!("publisher abort")
                }
                Err("publisher rejected")
            })
        }));
        if panic {
            assert!(result.is_err());
        } else {
            assert!(matches!(
                result.unwrap(),
                Err(NewtypePublicationError::Commit("publisher rejected"))
            ));
        }
        assert!(weak.upgrade().is_none());
        assert!(!vm.newtypes.contains_key(&id(2)));
        assert!(vm.newtypes.contains_key(&id(1)));
    }
    let valid = [definition(2, ty(3)), definition(3, boolean())];
    vm.install_generated_newtypes(&valid).unwrap();
    assert!(vm.newtypes.contains_key(&id(2)) && vm.newtypes.contains_key(&id(3)));
}
#[test]
fn phase_formation_scope_and_identity_checks_precede_publication() {
    let package = Package {
        newtypes: vec![definition(1, boolean())],
        ..Default::default()
    };
    let mut impossible = definition(2, boolean());
    impossible
        .requirements
        .push(nir::program::TypePredicate::Bool(false));
    let mut generic = definition(2, boolean());
    generic.type_parameters.push(id(50));
    let mut captured = definition(2, boolean());
    captured.capture_count = 1;
    for batch in [
        vec![definition(1, boolean())],
        vec![definition(2, boolean()), definition(2, boolean())],
        vec![definition(
            2,
            Term::Bound {
                depth: 0,
                index: 0,
                sort: nir::Sort::Type,
            },
        )],
        vec![definition(2, Term::Bool(true))],
        vec![impossible],
        vec![generic],
        vec![captured],
    ] {
        let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
        let called = std::cell::Cell::new(false);
        assert!(
            vm.install_generated_newtypes_with(&batch, || {
                called.set(true);
                Ok::<_, ()>(())
            })
            .is_err()
        );
        assert!(!called.get());
        assert!(!vm.newtypes.contains_key(&id(2)));
        assert_eq!(vm.newtypes.get(&id(1)).unwrap().representation, boolean());
    }
    let good = [definition(2, boolean())];
    let mut runtime = Interpreter::new(&package, Budget::default()).unwrap();
    assert_eq!(
        runtime.install_generated_newtypes(&good).unwrap_err().kind,
        ErrorKind::PhaseMismatch
    );
}
#[test]
fn exhausted_admission_rolls_back_and_can_retry_with_new_fuel() {
    let package = Package::default();
    let definitions = [definition(1, boolean()), definition(2, ty(1))];
    let mut probe = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    let before = probe.remaining_steps();
    probe.install_generated_newtypes(&definitions).unwrap();
    let cost = before - probe.remaining_steps();
    for steps in 0..cost {
        let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
        vm.budget.steps = steps;
        assert_eq!(
            vm.install_generated_newtypes(&definitions)
                .unwrap_err()
                .kind,
            ErrorKind::StepsExhausted
        );
        assert!(!vm.newtypes.contains_key(&id(1)) && !vm.newtypes.contains_key(&id(2)));
        vm.budget.steps = 100_000;
        vm.install_generated_newtypes(&definitions).unwrap();
    }
}
