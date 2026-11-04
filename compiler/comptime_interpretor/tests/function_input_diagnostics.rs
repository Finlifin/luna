use comptime_interpretor::{
    Budget, ErrorKind, Event, FunctionInputRejection, InputAdmissionStage, Interpreter, Value,
};
use nir::program::{Expr, ExprKind, Function, FunctionPhase, Literal, Package, SourceRange};
use nir::{DeclarationId, Primitive, Sort, Term};

fn id(local: u32) -> DeclarationId {
    DeclarationId { package: 7, local }
}
fn boolean() -> Term {
    Term::Primitive(Primitive::Bool)
}
fn world() -> Package {
    Package {
        function_parameters: vec![nir::function_parameters::Declaration {
            declaration: id(1),
            parameters: vec![id(2), id(3)],
            domains: vec![
                Term::Primitive(Primitive::Type),
                Term::Rigid {
                    declaration: id(2),
                    sort: Sort::Type,
                },
            ],
        }],
        functions: vec![Function {
            declaration: id(1),
            name: "accept".into(),
            phase: FunctionPhase::Comptime,
            inline: false,
            pure: true,
            is_unsafe: false,
            preconditions: vec![],
            postconditions: vec![],
            result_binding: None,
            entry_snapshots: vec![],
            capabilities: vec![],
            requirements: vec![],
            type_parameters: vec![id(2), id(3)],
            signature: Term::Function {
                parameters: vec![],
                result: Box::new(boolean()),
            },
            parameters: vec![],
            defaults: vec![],
            body: Expr {
                source: SourceRange { start: 10, end: 20 },
                ty: boolean(),
                kind: ExprKind::Literal(Literal::Bool(true)),
            },
        }],
        ..Default::default()
    }
}
fn rejected(kind: ErrorKind) -> FunctionInputRejection {
    let ErrorKind::FunctionInputRejected(rejection) = kind else {
        panic!("missing input rejection: {kind:?}");
    };
    assert_eq!(rejection.index, 1);
    assert_eq!(rejection.parameter, id(3));
    assert_eq!(rejection.stage, InputAdmissionStage::Value);
    *rejection
}

#[test]
fn dependent_domain_failure_keeps_the_slot_and_does_not_publish_an_instance() {
    let world = world();
    let mut vm = Interpreter::for_comptime(&world, Budget::default()).unwrap();
    let failure = vm
        .invoke_instantiated(id(1), vec![boolean(), Term::Char('x')], vec![])
        .unwrap_err();
    assert_eq!(failure.source, Some(SourceRange { start: 10, end: 20 }));
    assert!(!rejected(failure.kind).reason.is_empty());
    assert!(!vm.events().iter().any(|event| matches!(
        event,
        Event::InstantiateFunction { .. } | Event::EnterFunction(_)
    )));
    assert_eq!(vm.bytecode_compilations(), 0);
    assert_eq!(
        vm.invoke_instantiated(id(1), vec![boolean(), Term::Bool(true)], vec![])
            .unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn missing_symbolic_evidence_is_not_created_by_rejection() {
    let mut world = world();
    let callable = Term::ComptimeFunction(Box::new(world.functions[0].signature.clone()));
    let mut obtain = world.functions[0].clone();
    obtain.declaration = id(10);
    obtain.name = "obtain".into();
    obtain.type_parameters.clear();
    obtain.signature = Term::Function {
        parameters: vec![],
        result: Box::new(callable.clone()),
    };
    obtain.body = Expr {
        source: SourceRange { start: 30, end: 40 },
        ty: callable,
        kind: ExprKind::FunctionInstance {
            declaration: id(1),
            arguments: vec![
                boolean(),
                Term::Rigid {
                    declaration: id(99),
                    sort: Sort::ComptimeValue,
                },
            ],
        },
    };
    world.functions.push(obtain);
    let mut vm = Interpreter::for_comptime(&world, Budget::default()).unwrap();
    for _ in 0..2 {
        let failure = vm.invoke(id(10), vec![]).unwrap_err();
        assert_eq!(failure.source, Some(SourceRange { start: 30, end: 40 }));
        assert!(rejected(failure.kind).reason.contains("domain evidence"));
    }
    assert!(!vm.events().iter().any(|event| matches!(event,
        Event::InstantiateFunction { declaration, .. } | Event::EnterFunction(declaration)
            if *declaration == id(1))));
    vm.authorize_abstract_value_parameters(&[(id(99), boolean())])
        .unwrap();
    let function = vm.invoke(id(10), vec![]).unwrap();
    assert!(matches!(function, Value::Closure(_)));
    assert!(vm.events().iter().any(|event| matches!(event,
        Event::InstantiateFunction { declaration, .. } if *declaration == id(1))));
}

#[test]
fn input_rejection_dump_escapes_the_reason() {
    let failure = FunctionInputRejection {
        index: 1,
        parameter: id(3),
        stage: InputAdmissionStage::Domain,
        reason: "unavailable \"input\"\nnext".into(),
    };
    assert_eq!(
        failure.to_string(),
        "(function-input-rejected (index 1) (parameter (declaration (package 7) (local 3))) (stage domain) (reason \"unavailable \\\"input\\\"\\nnext\"))"
    );
}

#[test]
fn nested_callable_inputs_share_admission_and_keep_rejection_context() {
    let mut world = world();
    let mut callback = world.functions[0].clone();
    callback.declaration = id(20);
    callback.type_parameters.clear();
    let callable = Term::ComptimeFunction(Box::new(callback.signature.clone()));
    world.functions.push(callback);
    let domain = Term::Optional(Box::new(callable));
    let make = |declaration| Term::OptionalValue {
        ty: Box::new(domain.clone()),
        value: Some(Box::new(Term::FunctionValue {
            declaration,
            arguments: vec![],
        })),
    };
    let mut vm = Interpreter::for_comptime(&world, Budget::default()).unwrap();
    let error = vm
        .invoke_instantiated(id(1), vec![domain.clone(), make(id(99))], vec![])
        .unwrap_err();
    assert!(rejected(error.kind).reason.contains("signature evidence"));
    assert!(!vm.events().iter().any(|event| matches!(
        event,
        Event::InstantiateFunction { .. } | Event::EnterFunction(_)
    )));
    assert_eq!(
        vm.invoke_instantiated(id(1), vec![domain.clone(), make(id(20))], vec![])
            .unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn record_input_recovery_uses_field_domains_in_declaration_order() {
    let world = world();
    let mut vm = Interpreter::for_comptime(&world, Budget::default()).unwrap();
    let domain = Term::Record(vec![
        ("flag".into(), boolean()),
        ("letter".into(), Term::Primitive(Primitive::Char)),
    ]);
    let value = Term::RecordValue(vec![
        ("letter".into(), Term::Char('x')),
        ("flag".into(), Term::Bool(true)),
    ]);
    assert_eq!(
        vm.decode_immutable_input(&value, &domain).unwrap(),
        Value::AnonymousRecord(vec![
            ("flag".into(), Value::Bool(true)),
            ("letter".into(), Value::Char('x'))
        ])
    );
    let duplicated = Term::RecordValue(vec![
        ("flag".into(), Term::Bool(true)),
        ("flag".into(), Term::Bool(false)),
    ]);
    assert!(vm.decode_immutable_input(&duplicated, &domain).is_err());
    assert!(vm.decode_immutable_input(&value, &domain).is_ok());
}
