//! Callable evidence is indexed per immutable declaration world, not memoized invocation.
use nir::program::{Expr, ExprKind, Function, FunctionPhase, LocalId, Package, SourceRange};
use nir::value_admission::Error as AdmissionError;
use nir::{DeclarationId, Primitive, Sort, Term};
use type_resolution::dispatch::DeclarationWorld;
use type_resolution::ptc::Budget;

fn id(local: u32) -> DeclarationId {
    DeclarationId { package: 11, local }
}
fn boolean() -> Term {
    Term::Primitive(Primitive::Bool)
}
fn callable(domain: Term) -> Term {
    Term::Function {
        parameters: vec![nir::Parameter {
            sort: Sort::Runtime,
            ty: domain.clone(),
        }],
        result: Box::new(domain),
    }
}
fn function(local: u32) -> Function {
    let binding = LocalId {
        owner: local,
        index: 0,
    };
    Function {
        declaration: id(local),
        name: format!("f{local}"),
        phase: FunctionPhase::Comptime,
        pure: false,
        inline: false,
        is_unsafe: false,
        type_parameters: vec![],
        parameters: vec![binding],
        signature: callable(boolean()),
        requirements: vec![],
        capabilities: vec![],
        defaults: vec![],
        preconditions: vec![],
        postconditions: vec![],
        result_binding: None,
        entry_snapshots: vec![],
        body: Expr {
            source: SourceRange { start: 0, end: 1 },
            ty: boolean(),
            kind: ExprKind::Local(binding),
        },
    }
}
fn package(noise: u32) -> Package {
    let mut generic = function(1);
    generic.type_parameters = vec![id(10)];
    let t = Term::Rigid {
        declaration: id(10),
        sort: Sort::Type,
    };
    generic.signature = callable(t.clone());
    generic.body.ty = t;
    let mut functions = vec![generic];
    functions.extend((1000..1000 + noise).map(function));
    Package {
        functions,
        runtime_function_identities: vec![nir::program::RuntimeFunctionIdentity {
            declaration: id(2),
            type_parameters: vec![],
            signature: callable(boolean()),
        }],
        normal_forms: vec![nir::normal_forms::Head {
            declaration: id(3),
            collector: None,
            role: nir::normal_forms::Role::Opaque,
            parameters: vec![nir::normal_forms::Parameter {
                declaration: id(30),
                binding: LocalId { owner: 3, index: 0 },
                kind: nir::normal_forms::Kind::Type,
                ty: Term::Primitive(Primitive::Type),
            }],
            requirements: vec![],
        }],
        ..Default::default()
    }
}

#[test]
fn evidence_lookup_work_does_not_scale_with_unrelated_declarations() {
    let mut baseline = None;
    for noise in [0, 100, 2000] {
        let package = package(noise);
        let (world, remaining) = DeclarationWorld::new(
            &package,
            Budget {
                steps: 1_000_000,
                ..Budget::default()
            },
            None,
        )
        .unwrap();
        assert!(remaining < 1_000_000);
        let mut observed = vec![];
        for (declaration, arguments) in [(id(1), vec![boolean()]), (id(2), vec![]), (id(3), vec![])]
        {
            let mut fuel = 100;
            let signature = world
                .callable_signature(declaration, &arguments, &[], &mut fuel)
                .unwrap()
                .unwrap();
            signature.check_scope(&[]).unwrap();
            observed.push((signature, 100 - fuel));
        }
        assert_eq!(observed[0].0, callable(boolean()).with_comptime_phase(true));
        assert_eq!(observed[1].0, callable(boolean()));
        assert!(observed[2].0.is_normal_form_callable());
        if let Some(baseline) = &baseline {
            assert_eq!(&observed, baseline);
        } else {
            baseline = Some(observed);
        }
        assert!(
            world
                .callable_signature(id(9999), &[], &[], &mut 10)
                .unwrap()
                .is_none()
        );
        assert!(matches!(
            world.callable_signature(id(1), &[], &[], &mut 100),
            Err(AdmissionError::Invalid(_))
        ));
        assert_eq!(
            world.callable_signature(id(1), &[boolean()], &[], &mut 1),
            Err(AdmissionError::Budget)
        );
        assert!(
            world
                .callable_signature(id(1), &[boolean()], &[], &mut 100)
                .unwrap()
                .is_some()
        );
    }
}

#[test]
fn worlds_keep_signature_and_input_evidence_isolated() {
    let first = package(0);
    let mut second = package(0);
    second.functions[0].is_unsafe = true;
    let (a, _) = DeclarationWorld::new(&first, Budget::default(), None).unwrap();
    let (b, _) = DeclarationWorld::new(&second, Budget::default(), None).unwrap();
    let left = a
        .callable_signature(id(1), &[boolean()], &[], &mut 100)
        .unwrap()
        .unwrap();
    let right = b
        .callable_signature(id(1), &[boolean()], &[], &mut 100)
        .unwrap()
        .unwrap();
    assert!(!left.is_unsafe_callable());
    assert!(right.is_unsafe_callable());
    assert!(matches!(
        a.callable_signature(id(1), &[Term::Bool(true)], &[], &mut 100),
        Err(AdmissionError::Invalid(_))
    ));
    assert_eq!(
        a.callable_signature(id(1), &[boolean()], &[], &mut 100),
        Ok(Some(left))
    );
}

#[test]
fn ambiguous_runtime_metadata_does_not_publish_an_index() {
    let mut package = package(0);
    package
        .runtime_function_identities
        .push(package.runtime_function_identities[0].clone());
    assert!(DeclarationWorld::new(&package, Budget::default(), None).is_err());
    package.runtime_function_identities.pop();
    assert!(DeclarationWorld::new(&package, Budget::default(), None).is_ok());
    assert!(
        DeclarationWorld::new(
            &package,
            Budget {
                steps: 0,
                ..Budget::default()
            },
            None
        )
        .is_err()
    );
}

#[test]
fn callable_query_exhaustion_is_not_missing_evidence_with_remaining_fuel() {
    let mut package = package(0);
    package.functions[0].type_parameters = vec![id(10), id(11), id(12)];
    let (world, _) = DeclarationWorld::new(&package, Budget::default(), None).unwrap();
    let arguments = [boolean(), boolean(), boolean()];
    let mut fuel = 2;
    let result = world.with_admission_queries(|queries| {
        (queries.callable)(
            nir::CallableValueRef::Function {
                declaration: id(1),
                arguments: &arguments,
            },
            &[],
            &mut fuel,
        )
        .map(|signature| signature.map(|ty| ty.into_owned()))
    });
    assert_eq!(result, Err(nir::value_admission::Error::Budget));
    assert_eq!(
        fuel, 1,
        "the failed reservation does not consume unreserved work"
    );
    let mut completed = false;
    for allowance in 0..1000 {
        let mut fuel = allowance;
        match world.callable_signature(id(1), &arguments, &[], &mut fuel) {
            Err(AdmissionError::Budget) => {}
            Ok(Some(signature)) => {
                assert_eq!(signature, callable(boolean()).with_comptime_phase(true));
                completed = true;
                break;
            }
            other => panic!("valid callable query lost its outcome: {other:?}"),
        }
    }
    assert!(completed);
    assert!(matches!(
        world.callable_signature(id(1), &vec![Term::Bool(true); 3], &[], &mut 1000),
        Err(AdmissionError::Invalid(_))
    ));
    assert_eq!(
        world.callable_signature(id(9999), &[], &[], &mut 1000),
        Ok(None)
    );
    assert!(
        world
            .callable_signature(id(1), &arguments, &[], &mut 1000)
            .unwrap()
            .is_some()
    );
}

#[test]
fn method_signature_queries_preserve_missing_invalid_and_budget_outcomes() {
    use nir::program::{InterfaceInstance, MethodContract, Trait, TraitPhase};
    let mut package = package(0);
    package.traits.push(Trait {
        comptime_only: false,
        parameter_types: vec![],
        associated: vec![],
        requirements: vec![],
        capabilities: vec![],
        type_parameters: vec![],
        declaration: id(70),
        parents: vec![],
        self_parameter: id(71),
        name: "Inspect".into(),
        methods: vec![MethodContract {
            phase: TraitPhase::Runtime,
            is_quote: false,
            pure: false,
            is_unsafe: false,
            type_arguments: vec![],
            type_parameters: vec![id(73)],
            capabilities: vec![],
            program: Default::default(),
            parameter_bindings: vec![],
            by_value_receiver: false,
            by_reference_receiver: false,
            declaration: id(72),
            name: "inspect".into(),
            signature: callable(Term::Rigid {
                declaration: id(73),
                sort: Sort::Type,
            })
            .into(),
            requirements: vec![],
            default: None,
        }],
    });
    let method = nir::TraitMethodIdentity {
        interface: InterfaceInstance {
            declaration: id(70),
            arguments: vec![],
        },
        subject: boolean(),
        member: id(72),
        method_arguments: vec![boolean()],
    };
    let (world, _) = DeclarationWorld::new(&package, Budget::default(), None).unwrap();
    let query = |method: &nir::TraitMethodIdentity, fuel: &mut usize| {
        world.callable_value_signature(nir::CallableValueRef::Method(method), &[], fuel)
    };
    let mut fuel = 1;
    assert_eq!(query(&method, &mut fuel), Err(AdmissionError::Budget));
    assert_eq!(fuel, 1);
    let mut completed = false;
    for allowance in 0..1000 {
        let mut fuel = allowance;
        match query(&method, &mut fuel) {
            Err(AdmissionError::Budget) => {}
            Ok(Some(signature)) => {
                assert_eq!(signature, callable(boolean()));
                completed = true;
                break;
            }
            other => panic!("method query lost its outcome: {other:?}"),
        }
    }
    assert!(completed);
    let mut invalid = method.clone();
    invalid.method_arguments.clear();
    assert!(matches!(
        query(&invalid, &mut 1000),
        Err(AdmissionError::Invalid(_))
    ));
    invalid = method.clone();
    invalid.member = id(9999);
    assert_eq!(query(&invalid, &mut 1000), Ok(None));
    invalid = method.clone();
    invalid.interface.declaration = id(9999);
    assert_eq!(query(&invalid, &mut 1000), Ok(None));
    assert_eq!(query(&method, &mut 1000), Ok(Some(callable(boolean()))));
}

#[test]
fn construction_shares_work_across_metadata_and_recovers_after_exhaustion() {
    let package = package(30);
    let mut baseline = Budget::default();
    let world = DeclarationWorld::new_metered(&package, &mut baseline, None).unwrap();
    let work = Budget::default().steps - baseline.steps;
    assert!(work > 30);
    assert!(std::ptr::eq(
        world.function_declaration(id(1)).unwrap(),
        &package.functions[0]
    ));
    for steps in [0, 1, work / 2, work - 1] {
        let mut limited = Budget {
            steps,
            ..Budget::default()
        };
        assert!(matches!(
            DeclarationWorld::new_metered(&package, &mut limited, None),
            Err(AdmissionError::Budget)
        ));
        assert!(limited.steps <= steps);
    }
    let mut exact = Budget {
        steps: work,
        ..Budget::default()
    };
    DeclarationWorld::new_metered(&package, &mut exact, None).unwrap();
    assert_eq!(exact.steps, 0);
    let mut retry = Budget::default();
    DeclarationWorld::new_metered(&package, &mut retry, None).unwrap();
    assert_eq!(retry.steps, baseline.steps);
}

#[test]
fn metadata_payloads_are_charged_but_borrowed_function_bodies_are_not_copied() {
    let mut package = package(0);
    let mut f = function(200);
    let symbol = Term::Primitive(Primitive::Symbol);
    let Term::Function { result, .. } = &mut f.signature else {
        unreachable!()
    };
    *result = Box::new(symbol.clone());
    f.body.ty = symbol;
    f.body.kind = ExprKind::Literal(nir::program::Literal::Symbol("x".into()));
    package.functions.push(f);
    let mut short = Budget::default();
    DeclarationWorld::new_metered(&package, &mut short, None).unwrap();
    package.functions.last_mut().unwrap().body.kind =
        ExprKind::Literal(nir::program::Literal::Symbol("x".repeat(200_000)));
    let mut long = Budget::default();
    let world = DeclarationWorld::new_metered(&package, &mut long, None).unwrap();
    assert_eq!(short.steps, long.steps);
    assert!(std::ptr::eq(
        world.function_declaration(id(200)).unwrap(),
        package.functions.last().unwrap()
    ));
    package.structs.push(nir::program::Struct {
        declaration: id(300),
        name: "Payload".into(),
        type_parameters: vec![],
        parameter_types: vec![],
        capabilities: vec![],
        requirements: vec![],
        fields: vec![("x".repeat(200_000), boolean())],
    });
    let mut limited = Budget::default();
    assert!(matches!(
        DeclarationWorld::new_metered(&package, &mut limited, None),
        Err(AdmissionError::Budget)
    ));
    assert!(limited.steps > 0 && limited.steps < Budget::default().steps);
    package.structs[0].fields[0].0 = "small".into();
    DeclarationWorld::new_metered(&package, &mut limited, None).unwrap();
}

#[test]
fn supplied_assumption_payloads_share_the_world_construction_allowance() {
    let package = Package::default();
    let context = type_resolution::ptc::Context::default();
    let assumptions = [nir::program::TraitRequirement {
        interface: id(10),
        interface_arguments: vec![],
        subject: Term::Record(vec![("x".repeat(2000), boolean())]),
    }];
    let mut limited = Budget {
        steps: 100,
        ..Budget::default()
    };
    assert!(matches!(
        DeclarationWorld::new_metered(&package, &mut limited, Some((&context, &assumptions))),
        Err(AdmissionError::Budget)
    ));
    assert!(limited.steps > 0 && limited.steps < 100);
    let mut enough = Budget::default();
    let world =
        DeclarationWorld::new_metered(&package, &mut enough, Some((&context, &assumptions)))
            .unwrap();
    assert_eq!(world.assumptions(), &assumptions);
    assert!(enough.steps < Budget::default().steps - 2000);
}
