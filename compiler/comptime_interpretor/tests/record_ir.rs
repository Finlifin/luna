use comptime_interpretor::{Budget, ErrorKind, Event, Interpreter, Value};
use nir::program::{Expr, ExprKind as K, Function, FunctionPhase, Literal, Package, SourceRange};
use nir::{DeclarationId, Primitive as P, Term};
fn id(local: u32) -> DeclarationId {
    DeclarationId { package: 1, local }
}
fn p(primitive: P) -> Term {
    Term::Primitive(primitive)
}
fn expr(ty: Term, kind: K) -> Expr {
    Expr {
        ty,
        kind,
        source: SourceRange { start: 1, end: 2 },
    }
}
fn function(local: u32, body: Expr, result: Term) -> Function {
    Function {
        declaration: id(local),
        name: format!("run{local}"),
        phase: FunctionPhase::Runtime,
        inline: false,
        pure: false,
        is_unsafe: false,
        preconditions: vec![],
        postconditions: vec![],
        result_binding: None,
        entry_snapshots: vec![],
        type_parameters: vec![],
        requirements: vec![],
        capabilities: vec![],
        signature: Term::Function {
            parameters: vec![],
            result: Box::new(result),
        },
        parameters: vec![],
        defaults: vec![],
        body,
    }
}
fn call(local: u32) -> Expr {
    expr(
        p(P::Bool),
        K::Call {
            parameter_bindings: vec![],
            callee: Box::new(expr(
                Term::Function {
                    parameters: vec![],
                    result: Box::new(p(P::Bool)),
                },
                K::Function(id(local)),
            )),
            arguments: vec![],
        },
    )
}
fn record() -> Term {
    Term::Record(vec![
        ("first".into(), p(P::Bool)),
        ("second".into(), p(P::Bool)),
    ])
}
#[test]
fn record_ir_constructs_named_fields_once_in_order() {
    let body = expr(
        record(),
        K::AnonymousRecord {
            fields: vec![call(1), call(2)],
        },
    );
    let package = Package {
        functions: vec![
            function(0, body, record()),
            function(
                1,
                expr(p(P::Bool), K::Literal(Literal::Bool(true))),
                p(P::Bool),
            ),
            function(
                2,
                expr(p(P::Bool), K::Literal(Literal::Bool(false))),
                p(P::Bool),
            ),
        ],
        ..Default::default()
    };
    nir::phases::validate(&package).unwrap();
    let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
    assert_eq!(
        vm.invoke(id(0), vec![]).unwrap(),
        Value::AnonymousRecord(vec![
            ("first".into(), Value::Bool(true)),
            ("second".into(), Value::Bool(false))
        ])
    );
    assert_eq!(
        vm.events()
            .iter()
            .filter_map(|event| if let Event::EnterFunction(id) = event {
                Some(*id)
            } else {
                None
            })
            .collect::<Vec<_>>(),
        vec![id(0), id(1), id(2)]
    );
    println!("RECORD_IR_DUMP={package}\nRECORD_IR_END");
}
#[test]
fn record_ir_field_control_transfer_skips_later_fields_and_construction() {
    let body = expr(
        p(P::Never),
        K::AnonymousRecord {
            fields: vec![call(1), expr(p(P::Never), K::Return(None)), call(2)],
        },
    );
    let package = Package {
        functions: vec![
            function(0, body, p(P::Void)),
            function(
                1,
                expr(p(P::Bool), K::Literal(Literal::Bool(true))),
                p(P::Bool),
            ),
            function(
                2,
                expr(p(P::Bool), K::Literal(Literal::Bool(false))),
                p(P::Bool),
            ),
        ],
        ..Default::default()
    };
    nir::phases::validate(&package).unwrap();
    let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
    assert_eq!(vm.invoke(id(0), vec![]).unwrap(), Value::Unit);
    assert_eq!(
        vm.events()
            .iter()
            .filter_map(|event| if let Event::EnterFunction(id) = event {
                Some(*id)
            } else {
                None
            })
            .collect::<Vec<_>>(),
        vec![id(0), id(1)]
    );
}
#[test]
fn malformed_record_ir_rejects_shape_and_runtime_field_types() {
    for ty in [p(P::Bool), record()] {
        let package = Package {
            functions: vec![function(
                0,
                expr(ty.clone(), K::AnonymousRecord { fields: vec![] }),
                ty,
            )],
            ..Default::default()
        };
        assert!(
            nir::phases::validate(&package)
                .unwrap_err()
                .message
                .contains("matching named field types")
        );
    }
    let package = Package {
        functions: vec![function(
            0,
            expr(
                record(),
                K::AnonymousRecord {
                    fields: vec![
                        expr(p(P::Bool), K::Literal(Literal::Bool(true))),
                        expr(p(P::Char), K::Literal(Literal::Char('x'))),
                    ],
                },
            ),
            record(),
        )],
        ..Default::default()
    };
    let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
    assert_eq!(vm.invoke(id(0), vec![]).unwrap_err().kind, ErrorKind::Type);
}
#[test]
fn record_ir_specialization_visits_field_values_and_their_types() {
    let parameter = id(10);
    let rigid = Term::Rigid {
        declaration: parameter,
        sort: nir::Sort::Type,
    };
    let original = expr(
        Term::Record(vec![("value".into(), rigid.clone())]),
        K::AnonymousRecord {
            fields: vec![expr(
                rigid,
                K::Local(nir::program::LocalId { owner: 1, index: 0 }),
            )],
        },
    );
    let concrete = original
        .instantiate_types(&[parameter], &[p(P::Bool)])
        .unwrap();
    assert_eq!(
        concrete.ty,
        Term::Record(vec![("value".into(), p(P::Bool))])
    );
    let K::AnonymousRecord { fields } = concrete.kind else {
        panic!("record")
    };
    assert_eq!(fields[0].ty, p(P::Bool));
    assert!(
        original.ty.mentions_rigid(parameter),
        "specialization must not mutate the shared source"
    );
}

#[test]
fn record_field_ir_validates_labels_and_visits_the_record_producer() {
    for name in ["first", "missing"] {
        let record = expr(
            record(),
            K::AnonymousRecord {
                fields: vec![
                    expr(p(P::Bool), K::Literal(Literal::Bool(true))),
                    expr(p(P::Bool), K::Literal(Literal::Bool(false))),
                ],
            },
        );
        let body = expr(
            p(P::Bool),
            K::AnonymousField {
                value: Box::new(record),
                name: name.into(),
                index: 0,
            },
        );
        let package = Package {
            functions: vec![function(0, body, p(P::Bool))],
            ..Default::default()
        };
        let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
        if name == "first" {
            nir::phases::validate(&package).unwrap();
            assert_eq!(vm.invoke(id(0), vec![]).unwrap(), Value::Bool(true));
            println!("RECORD_FIELD_DUMP={package}\nRECORD_FIELD_END");
        } else {
            assert!(
                nir::phases::validate(&package)
                    .unwrap_err()
                    .message
                    .contains("matching record label")
            );
            assert_eq!(vm.invoke(id(0), vec![]).unwrap_err().kind, ErrorKind::Type);
        }
    }
}

#[test]
fn record_pointer_ir_checks_layout_and_preserves_escaped_dump() {
    use nir::program::LocalId;
    let name = "flag\"\\\n";
    let ty = Term::Record(vec![(name.into(), p(P::Bool))]);
    let local = LocalId { owner: 0, index: 0 };
    for (label, index, field_type, valid) in [
        (name, 0, p(P::Bool), true),
        ("other", 0, p(P::Bool), false),
        (name, 1, p(P::Bool), false),
        (name, 0, p(P::Char), false),
    ] {
        let root = expr(ty.clone(), K::Local(local));
        let pointer = expr(
            Term::Pointer(Box::new(ty.clone())),
            K::AddressOf(Box::new(root)),
        );
        let field = expr(
            Term::Pointer(Box::new(field_type.clone())),
            K::PointerAnonymousField {
                pointer: Box::new(pointer),
                name: label.into(),
                index,
            },
        );
        let body = expr(
            field_type.clone(),
            K::Block {
                statements: vec![expr(
                    p(P::Void),
                    K::Let {
                        binding: local,
                        value: Box::new(expr(
                            ty.clone(),
                            K::AnonymousRecord {
                                fields: vec![expr(p(P::Bool), K::Literal(Literal::Bool(true)))],
                            },
                        )),
                    },
                )],
                result: Some(Box::new(expr(
                    field_type.clone(),
                    K::ReadPointer(Box::new(field)),
                ))),
            },
        );
        let package = Package {
            functions: vec![function(0, body, field_type)],
            ..Default::default()
        };
        let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
        if valid {
            nir::phases::validate(&package).unwrap();
            assert_eq!(vm.invoke(id(0), vec![]).unwrap(), Value::Bool(true));
            println!("RECORD_POINTER_DUMP={package}\nRECORD_POINTER_END");
        } else {
            assert!(
                nir::phases::validate(&package)
                    .unwrap_err()
                    .message
                    .contains("record field pointer")
            );
            assert_eq!(vm.invoke(id(0), vec![]).unwrap_err().kind, ErrorKind::Type);
        }
    }
}
