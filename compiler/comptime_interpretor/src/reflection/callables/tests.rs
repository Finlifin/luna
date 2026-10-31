use super::*;
use nir::{Parameter, Sort};

fn p(value: Primitive) -> Term {
    Term::Primitive(value)
}
fn id(local: u32) -> DeclarationId {
    DeclarationId { package: 91, local }
}
fn signature() -> Term {
    Term::Function {
        parameters: vec![
            Parameter {
                sort: Sort::Type,
                ty: p(Primitive::Type),
            },
            Parameter {
                sort: Sort::Runtime,
                ty: Term::Bound {
                    depth: 0,
                    index: 0,
                    sort: Sort::Type,
                },
            },
        ],
        result: Box::new(Term::Bound {
            depth: 0,
            index: 0,
            sort: Sort::Type,
        }),
    }
}

#[test]
fn dependent_signature_retains_binders_without_implying_comptime_phase() {
    let package = Package::default();
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    for comptime in [false, true] {
        for unsafe_call in [false, true] {
            let ty = signature()
                .with_comptime_phase(comptime)
                .with_unsafe_call_requirement(unsafe_call);
            let view = vm.reflect_callable(&ty).unwrap();
            assert_eq!(view.subject, ty);
            assert_eq!(view.callable, ty);
            assert_eq!(view.signature(), &signature());
            assert_eq!(view.is_comptime(), comptime);
            assert_eq!(view.is_unsafe(), unsafe_call);
            assert!(view.signature().check_scope(&[]).is_ok());
            let Term::Function { result, .. } = view.signature() else {
                panic!()
            };
            assert!(
                result.check_scope(&[]).is_err(),
                "result still needs its signature's binder"
            );
        }
    }
    assert!(vm.events().is_empty());
}

#[test]
fn closure_environment_descriptions_survive_without_observing_capture_values() {
    let package = Package::default();
    let fields = vec![
        Term::Record(vec![("capture\n\"".into(), p(Primitive::Bool))]),
        Term::Pointer(Box::new(p(Primitive::Char))),
    ];
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    for ty in [
        Term::Closure {
            environment: fields.clone(),
            signature: Box::new(signature()),
        },
        Term::ContextualClosure {
            context: Box::new(Term::ClosureContextValue(fields.clone())),
            signature: Box::new(
                signature()
                    .with_comptime_phase(true)
                    .with_unsafe_call_requirement(true),
            ),
        },
    ] {
        let view = vm.reflect_callable(&ty).unwrap();
        assert_eq!(view.subject, ty);
        let Term::Closure { environment, .. } = &view.callable else {
            panic!()
        };
        assert_eq!(environment, &fields);
        assert_eq!(view.is_comptime(), ty.is_comptime_callable());
        assert_eq!(view.is_unsafe(), ty.is_unsafe_callable());
        assert_eq!(view.signature(), &signature());
        let dump = view.to_string();
        assert!(dump.starts_with("(callable-reflection (subject "));
        assert!(dump.contains("capture\\n\\\""), "{dump}");
        assert_eq!(dump, vm.reflect_callable(&ty).unwrap().to_string());
    }
    assert!(vm.events().is_empty());
}

#[test]
fn contextual_closure_checks_environment_and_signature_formation() {
    let mut package = Package::default();
    package.structs.push(nir::program::Struct {
        declaration: id(1),
        name: "Rejected".into(),
        type_parameters: vec![],
        parameter_types: vec![],
        requirements: vec![nir::program::TypePredicate::Bool(false)],
        capabilities: vec![],
        fields: vec![],
    });
    let nominal = Term::Nominal {
        declaration: id(1),
        arguments: vec![],
    };
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    for in_environment in [true, false] {
        let ty = Term::ContextualClosure {
            context: Box::new(Term::ClosureContextValue(if in_environment {
                vec![nominal.clone()]
            } else {
                vec![]
            })),
            signature: Box::new(if in_environment {
                signature()
            } else {
                Term::Function {
                    parameters: vec![],
                    result: Box::new(nominal.clone()),
                }
            }),
        };
        assert_eq!(
            vm.admit_type(&ty, 0).unwrap_err().kind,
            ErrorKind::TypeContractRejected
        );
        assert_eq!(
            vm.reflect_callable(&ty).unwrap_err().kind,
            ErrorKind::TypeContractRejected
        );
        assert_eq!(
            vm.validate_immutable_input(&Value::Type(ty), &p(Primitive::Type))
                .unwrap_err()
                .kind,
            ErrorKind::TypeContractRejected
        );
    }
    assert!(vm.reflect_callable(&signature()).is_ok());
}

#[test]
fn wrong_phase_malformed_telescope_and_budget_failure_do_not_grant_a_view() {
    let package = Package::default();
    let mut runtime = Interpreter::new(&package, Budget::default()).unwrap();
    assert_eq!(
        runtime.reflect_callable(&signature()).unwrap_err().kind,
        ErrorKind::PhaseMismatch
    );
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    for ty in [
        p(Primitive::Bool),
        Term::Bool(true),
        Term::ComptimeFunction(Box::new(p(Primitive::Bool))),
        Term::Function {
            parameters: vec![],
            result: Box::new(Term::Bound {
                depth: 0,
                index: 0,
                sort: Sort::Type,
            }),
        },
    ] {
        assert!(vm.reflect_callable(&ty).is_err(), "{ty}");
    }
    let big = Term::Function {
        parameters: vec![
            Parameter {
                sort: Sort::Runtime,
                ty: p(Primitive::Bool)
            };
            256
        ],
        result: Box::new(p(Primitive::Bool)),
    };
    vm.budget.steps = 1;
    assert_eq!(
        vm.reflect_callable(&big).unwrap_err().kind,
        ErrorKind::StepsExhausted
    );
    vm.budget = Budget::default();
    assert_eq!(vm.reflect_callable(&big).unwrap().callable, big);
}

#[test]
fn associated_callable_view_preserves_the_original_query() {
    let package = Package::default();
    let query = Term::AssociatedType {
        origin: Box::new(nir::AssociatedTypeOrigin {
            interface: id(2),
            member: id(3),
        }),
        arguments: vec![],
        subject: Box::new(p(Primitive::Bool)),
    };
    let selected = Expr {
        source: SourceRange { start: 0, end: 0 },
        ty: p(Primitive::Type),
        kind: K::TypeValue(signature()),
    };
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    assert!(vm.reflect_callable(&query).is_err());
    vm.associated_values
        .insert((id(2).into(), p(Primitive::Bool), id(3)), &selected);
    let view = vm.reflect_callable(&query).unwrap();
    assert_eq!(view.subject, query);
    assert_eq!(view.signature(), &signature());
    assert!(vm.events().is_empty());
}

#[test]
fn normal_form_heads_are_preserved_in_signatures_and_never_unfolded() {
    let mut package = Package::default();
    package.normal_forms.push(nir::normal_forms::Head {
        collector: None,
        declaration: id(8),
        role: nir::normal_forms::Role::Opaque,
        parameters: vec![],
        requirements: vec![],
    });
    let opaque = Term::NormalForm {
        head: id(8),
        sort: Sort::Type,
        arguments: vec![],
    };
    let ty = Term::Function {
        parameters: vec![Parameter {
            sort: Sort::Runtime,
            ty: opaque.clone(),
        }],
        result: Box::new(opaque.clone()),
    };
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    assert_eq!(vm.reflect_callable(&ty).unwrap().signature(), &ty);
    assert_eq!(
        vm.reflect_callable(&opaque).unwrap_err().kind,
        ErrorKind::Type
    );
}
