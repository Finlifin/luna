use comptime_interpretor::{Budget, ErrorKind, Event, Interpreter, Value};
use nir::program::{Expr, ExprKind as K, Function, FunctionPhase, Literal, Package, SourceRange};
use nir::{DeclarationId, Primitive as P, Sort, Term};

fn id(local: u32) -> DeclarationId {
    DeclarationId { package: 1, local }
}
fn expression(ty: Term, kind: K) -> Expr {
    Expr {
        ty,
        kind,
        source: SourceRange { start: 1, end: 2 },
    }
}

fn package(body: Expr) -> Package {
    Package {
        functions: vec![Function {
            declaration: id(1),
            name: "inspect".into(),
            phase: FunctionPhase::Comptime,
            inline: false,
            pure: false,
            is_unsafe: false,
            preconditions: vec![],
            postconditions: vec![],
            result_binding: None,
            entry_snapshots: vec![],
            capabilities: vec![],
            requirements: vec![],
            type_parameters: vec![id(2)],
            signature: Term::Function {
                parameters: vec![],
                result: Box::new(Term::Primitive(P::Bool)),
            },
            parameters: vec![],
            defaults: vec![],
            body,
        }],
        ..Default::default()
    }
}

#[test]
fn expanded_instance_copies_fail_before_cache_publication() {
    let body = expression(
        Term::Primitive(P::Bool),
        K::Block {
            statements: (0..40)
                .map(|_| {
                    expression(
                        Term::Primitive(P::Type),
                        K::TypeValue(Term::Rigid {
                            declaration: id(2),
                            sort: Sort::Type,
                        }),
                    )
                })
                .collect(),
            result: Some(Box::new(expression(
                Term::Primitive(P::Bool),
                K::Literal(Literal::Bool(true)),
            ))),
        },
    );
    let package = package(body);
    let input = Term::Tuple(vec![Term::Primitive(P::Bool); 128]);
    let before = package.to_string();
    let mut bounded = Interpreter::for_comptime(
        &package,
        Budget {
            steps: 3000,
            depth: 128,
        },
    )
    .unwrap();
    assert_eq!(
        bounded
            .invoke_instantiated(id(1), vec![], vec![])
            .unwrap_err()
            .kind,
        ErrorKind::Type
    );
    assert_eq!(
        bounded
            .invoke_instantiated(id(1), vec![input.clone()], vec![])
            .unwrap_err()
            .kind,
        ErrorKind::StepsExhausted
    );
    assert!(
        !bounded
            .events()
            .iter()
            .any(|event| matches!(event, Event::InstantiateFunction { .. }))
    );
    let mut complete = Interpreter::for_comptime(
        &package,
        Budget {
            steps: 100_000,
            depth: 128,
        },
    )
    .unwrap();
    for _ in 0..2 {
        assert_eq!(
            complete
                .invoke_instantiated(id(1), vec![input.clone()], vec![])
                .unwrap(),
            Value::Bool(true)
        );
    }
    assert_eq!(
        complete
            .events()
            .iter()
            .filter(|event| matches!(event, Event::InstantiateFunction { .. }))
            .count(),
        1
    );
    assert_eq!(package.to_string(), before);
}

#[test]
fn cached_instances_preflight_program_contract_copies_on_each_call() {
    for postcondition in [false, true] {
        let mut package = package(expression(
            Term::Primitive(P::Bool),
            K::Literal(Literal::Bool(true)),
        ));
        let input = Term::Tuple(vec![Term::Primitive(P::Bool); 128]);
        let value_binding = nir::program::LocalId { owner: 1, index: 0 };
        let domain = Term::Rigid {
            declaration: id(2),
            sort: Sort::Type,
        };
        package.functions[0].parameters = vec![value_binding];
        package.functions[0].signature = Term::Function {
            parameters: vec![nir::Parameter {
                sort: Sort::Runtime,
                ty: domain.clone(),
            }],
            result: Box::new(Term::Primitive(P::Bool)),
        };
        // Ordinary value reads in a program predicate, not Type propositions.
        let mut condition = expression(
            Term::Primitive(P::Bool),
            K::Block {
                statements: (0..20)
                    .map(|_| expression(domain.clone(), K::Local(value_binding)))
                    .collect(),
                result: Some(Box::new(expression(
                    Term::Primitive(P::Bool),
                    K::Literal(Literal::Bool(true)),
                ))),
            },
        );
        condition.source = SourceRange { start: 10, end: 11 };
        if postcondition {
            package.functions[0].postconditions.push(condition);
        } else {
            package.functions[0].preconditions.push(condition);
        }
        nir::phases::validate(&package).unwrap();
        let mut vm = Interpreter::for_comptime(
            &package,
            Budget {
                steps: 100_000,
                depth: 128,
            },
        )
        .unwrap();
        let value = Value::Tuple(vec![Value::Bool(true); 128]);
        assert_eq!(
            vm.invoke_instantiated(id(1), vec![input.clone()], vec![value.clone()])
                .unwrap(),
            Value::Bool(true)
        );
        vm.charge_preparation((vm.remaining_steps() - 2000) as usize)
            .unwrap();
        let error = vm
            .invoke_instantiated(id(1), vec![input], vec![value])
            .unwrap_err();
        assert_eq!(error.kind, ErrorKind::StepsExhausted);
        assert_eq!(error.source, Some(SourceRange { start: 10, end: 11 }));
        assert_eq!(vm.events().iter().filter(|event| matches!(event,Event::InstantiateFunction {declaration,..} if *declaration==id(1))).count(),1);
    }
}
