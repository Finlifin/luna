use comptime_interpretor::{Budget, ErrorKind, Event, Interpreter, Value};
use nir::program::{Expr, ExprKind as K, Function, FunctionPhase, Package, SourceRange};
use nir::{DeclarationId, Primitive as P, Sort, Term};

fn id(local: u32) -> DeclarationId {
    DeclarationId { package: 1, local }
}
fn p(value: P) -> Term {
    Term::Primitive(value)
}
fn expr(ty: Term, kind: K) -> Expr {
    Expr {
        ty,
        kind,
        source: SourceRange { start: 1, end: 2 },
    }
}
fn value(ty: Term) -> Expr {
    expr(p(P::Type), K::TypeValue(ty))
}
fn function(local: u32, body: Expr) -> Function {
    Function {
        declaration: id(local),
        name: format!("Type{local}"),
        phase: FunctionPhase::Comptime,
        inline: false,
        pure: false,
        preconditions: vec![],
        postconditions: vec![],
        result_binding: None,
        entry_snapshots: vec![],
        type_parameters: vec![],
        requirements: vec![],
        capabilities: vec![],
        signature: Term::Function {
            parameters: vec![],
            result: Box::new(p(P::Type)),
        },
        parameters: vec![],
        defaults: vec![],
        body,
    }
}
fn call(local: u32) -> Expr {
    let signature = Term::Function {
        parameters: vec![],
        result: Box::new(p(P::Type)),
    }
    .with_comptime_phase(true);
    expr(
        p(P::Type),
        K::Call {
            parameter_bindings: vec![],
            callee: Box::new(expr(signature, K::Function(id(local)))),
            arguments: vec![],
        },
    )
}

fn record(requirements: Vec<nir::program::TypePredicate>) -> nir::program::Struct {
    nir::program::Struct {
        parameter_types: vec![],
        declaration: id(20),
        name: "Box".into(),
        type_parameters: vec![id(9)],
        requirements,
        capabilities: vec![],
        fields: vec![(
            "value".into(),
            Term::Rigid {
                declaration: id(9),
                sort: Sort::Type,
            },
        )],
    }
}
fn nominal(argument: Expr) -> Expr {
    expr(
        p(P::Type),
        K::NominalType {
            declaration: id(20),
            arguments: vec![argument],
        },
    )
}

#[test]
fn evaluated_arguments_are_checked_against_the_nominal_contract_before_escape() {
    let package = Package {
        structs: vec![record(vec![nir::program::TypePredicate::Equal(
            Term::Rigid {
                declaration: id(9),
                sort: Sort::Type,
            },
            p(P::Bool),
        )])],
        functions: vec![
            function(0, nominal(call(1))),
            function(1, value(p(P::Char))),
            function(2, nominal(value(p(P::Bool)))),
        ],
        ..Default::default()
    };
    let mut interpreter = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    assert_eq!(
        interpreter.invoke(id(0), vec![]).unwrap_err().kind,
        ErrorKind::TypeContractUnresolved
    );
    let entered: Vec<_> = interpreter
        .events()
        .iter()
        .filter_map(|event| match event {
            Event::EnterFunction(id) => Some(*id),
            _ => None,
        })
        .collect();
    assert_eq!(entered, vec![id(0), id(1)]);
    assert_eq!(
        interpreter.invoke(id(2), vec![]).unwrap(),
        Value::Type(Term::Nominal {
            declaration: id(20),
            arguments: vec![p(P::Bool)]
        })
    );
}

#[test]
fn missing_descriptors_arity_and_non_type_inputs_do_not_construct_nominals() {
    for body in [
        expr(
            p(P::Type),
            K::NominalType {
                declaration: id(99),
                arguments: vec![value(p(P::Bool))],
            },
        ),
        expr(
            p(P::Type),
            K::NominalType {
                declaration: id(20),
                arguments: vec![],
            },
        ),
        nominal(expr(
            p(P::Bool),
            K::Literal(nir::program::Literal::Bool(true)),
        )),
        nominal(value(Term::Bool(true))),
    ] {
        let package = Package {
            structs: vec![record(vec![])],
            functions: vec![function(0, body), function(1, nominal(value(p(P::Bool))))],
            ..Default::default()
        };
        let mut interpreter = Interpreter::new(&package, Budget::default()).unwrap();
        assert_eq!(
            interpreter.invoke(id(0), vec![]).unwrap_err().kind,
            ErrorKind::Type
        );
        assert!(interpreter.invoke(id(1), vec![]).is_ok());
    }
}

#[test]
fn specialization_preserves_the_template_and_bounded_construction_recovers() {
    let body = nominal(value(Term::Rigid {
        declaration: id(9),
        sort: Sort::Type,
    }));
    let specialized = body.instantiate_types(&[id(9)], &[p(P::Char)]).unwrap();
    assert_ne!(body, specialized);
    let package = Package {
        structs: vec![record(vec![])],
        functions: vec![function(0, specialized)],
        ..Default::default()
    };
    assert_eq!(
        Interpreter::new(
            &package,
            Budget {
                steps: 1,
                depth: 128
            }
        )
        .unwrap()
        .invoke(id(0), vec![])
        .unwrap_err()
        .kind,
        ErrorKind::StepsExhausted
    );
    assert_eq!(
        Interpreter::new(&package, Budget::default())
            .unwrap()
            .invoke(id(0), vec![])
            .unwrap(),
        Value::Type(Term::Nominal {
            declaration: id(20),
            arguments: vec![p(P::Char)]
        })
    );
}

#[test]
fn transparent_alias_guards_check_before_body_and_do_not_produce_nominal_values() {
    for allowed in [true, false] {
        let guard = expr(
            p(P::Void),
            K::CheckTypeAlias {
                declaration: id(20),
                arguments: vec![call(2)],
            },
        );
        let package = Package {
            alias_contracts: vec![nir::type_aliases::Contract {
                declaration: id(20),
                parameters: vec![id(9)],
                requirements: vec![nir::program::TypePredicate::Bool(allowed)],
                capabilities: vec![],
            }],
            functions: vec![
                function(
                    1,
                    expr(
                        p(P::Type),
                        K::Block {
                            statements: vec![guard],
                            result: Some(Box::new(call(3))),
                        },
                    ),
                ),
                function(2, value(p(P::Bool))),
                function(3, value(p(P::Char))),
            ],
            ..Default::default()
        };
        let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
        let result = vm.invoke(id(1), vec![]);
        if allowed {
            assert_eq!(result.unwrap(), Value::Type(p(P::Char)));
        } else {
            assert_eq!(result.unwrap_err().kind, ErrorKind::TypeContractRejected);
        }
        let calls: Vec<_> = vm
            .events()
            .iter()
            .filter_map(|event| match event {
                Event::EnterFunction(id) => Some(*id),
                _ => None,
            })
            .collect();
        assert_eq!(
            calls,
            if allowed {
                vec![id(1), id(2), id(3)]
            } else {
                vec![id(1), id(2)]
            }
        );
    }
}

#[test]
fn alias_guards_reject_runtime_placement_and_malformed_inputs() {
    let contract = nir::type_aliases::Contract {
        declaration: id(20),
        parameters: vec![id(9)],
        requirements: vec![],
        capabilities: vec![],
    };
    let guard = expr(
        p(P::Void),
        K::CheckTypeAlias {
            declaration: id(20),
            arguments: vec![value(p(P::Bool))],
        },
    );
    let mut package = Package {
        alias_contracts: vec![contract],
        functions: vec![function(
            1,
            expr(
                p(P::Type),
                K::Block {
                    statements: vec![guard],
                    result: Some(Box::new(value(p(P::Bool)))),
                },
            ),
        )],
        ..Default::default()
    };
    assert!(nir::phases::validate(&package).is_ok());
    package.functions[0].phase = FunctionPhase::Runtime;
    assert!(nir::phases::validate(&package).is_err());
    package.functions[0].phase = FunctionPhase::Comptime;
    let K::Block { statements, .. } = &mut package.functions[0].body.kind else {
        unreachable!()
    };
    let K::CheckTypeAlias { arguments, .. } = &mut statements[0].kind else {
        unreachable!()
    };
    arguments.clear();
    assert!(nir::phases::validate(&package).is_err());
    package.alias_contracts.clear();
    assert!(nir::phases::validate(&package).is_err());
}

#[test]
fn nominal_constructor_checks_value_domains_before_export_and_recovers() {
    let mut descriptor = record(vec![]);
    descriptor.parameter_types = vec![p(P::U8)];
    descriptor.fields.clear();
    let integer = |n: u128| {
        expr(
            p(P::Integer),
            K::Literal(nir::program::Literal::Integer(n.to_string())),
        )
    };
    let package = Package {
        structs: vec![descriptor],
        functions: vec![
            function(0, nominal(integer(256))),
            function(1, nominal(integer(255))),
        ],
        ..Default::default()
    };
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    assert_eq!(vm.invoke(id(0), vec![]).unwrap_err().kind, ErrorKind::Type);
    assert_eq!(
        vm.invoke(id(1), vec![]).unwrap(),
        Value::Type(Term::Nominal {
            declaration: id(20),
            arguments: vec![Term::Integer(comptime_runtime::Integer::from_u128(255))]
        })
    );
}

#[test]
fn nominal_constructor_value_indices_keep_contract_substitution_sorts() {
    let mut descriptor = record(vec![nir::program::TypePredicate::Equal(
        Term::Nominal {
            declaration: id(20),
            arguments: vec![Term::Rigid {
                declaration: id(9),
                sort: Sort::ComptimeValue,
            }],
        },
        Term::Nominal {
            declaration: id(20),
            arguments: vec![Term::Bool(true)],
        },
    )]);
    descriptor.parameter_types = vec![p(P::Bool)];
    descriptor.fields.clear();
    let argument = |flag| expr(p(P::Bool), K::Literal(nir::program::Literal::Bool(flag)));
    let package = Package {
        structs: vec![descriptor],
        functions: vec![
            function(0, nominal(argument(false))),
            function(1, nominal(argument(true))),
        ],
        ..Default::default()
    };
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    assert_eq!(
        vm.invoke(id(0), vec![]).unwrap_err().kind,
        ErrorKind::TypeContractUnresolved
    );
    assert_eq!(
        vm.invoke(id(1), vec![]).unwrap(),
        Value::Type(Term::Nominal {
            declaration: id(20),
            arguments: vec![Term::Bool(true)]
        })
    );
}
