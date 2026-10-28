use comptime_runtime::Integer;
use elaboration::verification::Limits;

#[test]
fn captured_integer_facts_check_domains_budgets_and_symbolic_inputs() {
    use elaboration::verification::flow;
    use nir::program::{
        Expr, ExprKind as K, Function, FunctionPhase, IntegerOp, Literal, SourceRange,
    };
    use nir::{DeclarationId, Primitive, Sort, Term};
    let function = |primitive, value| {
        let ty = Term::Primitive(primitive);
        let expression = |kind| Expr {
            ty: ty.clone(),
            source: SourceRange { start: 0, end: 1 },
            kind,
        };
        Function {
            declaration: DeclarationId {
                package: 1,
                local: 1,
            },
            name: "increment".into(),
            phase: FunctionPhase::Runtime,
            inline: false,
            pure: false,
            is_unsafe: false,
            signature: Term::Function {
                parameters: vec![],
                result: Box::new(ty.clone()),
            },
            parameters: vec![],
            type_parameters: vec![],
            defaults: vec![],
            preconditions: vec![],
            postconditions: vec![],
            result_binding: None,
            entry_snapshots: vec![],
            capabilities: vec![],
            requirements: vec![],
            body: expression(K::IntegerBinary {
                op: IntegerOp::Add,
                left: Box::new(expression(K::ComptimeValue(value))),
                right: Box::new(expression(K::Literal(match primitive {
                    Primitive::I8 => Literal::SignedInteger(1),
                    Primitive::Integer => Literal::Integer("1".into()),
                    _ => Literal::UnsignedInteger(1),
                }))),
            }),
        }
    };
    for (primitive, value, accepted) in [
        (Primitive::U8, 254, true),
        (Primitive::U8, 255, false),
        (Primitive::U8, 256, false),
        (Primitive::U8, -1, false),
        (Primitive::I8, -128, true),
        (Primitive::I8, -129, false),
        (Primitive::I8, 127, false),
    ] {
        let input = function(primitive, Term::Integer(Integer::from_i128(value)));
        assert_eq!(
            flow::analyze(&input, &mut Limits::default()).is_ok(),
            accepted,
            "{primitive:?}/{value}"
        );
    }
    let symbolic = function(
        Primitive::U8,
        Term::Rigid {
            declaration: DeclarationId {
                package: 1,
                local: 2,
            },
            sort: Sort::ComptimeValue,
        },
    );
    assert!(flow::analyze(&symbolic, &mut Limits::default()).is_err());
    let huge = Integer::parse_literal(&"9".repeat(200), &mut Default::default()).unwrap();
    let input = function(Primitive::Integer, Term::Integer(huge));
    let mut limited = Limits::default();
    limited.arithmetic.max_limbs = 1;
    assert!(flow::analyze(&input, &mut limited).is_err());
    assert!(flow::analyze(&input, &mut Limits::default()).is_ok());
    let mut exhausted = Limits::default();
    exhausted.arithmetic.steps = 2;
    assert!(flow::analyze(&input, &mut exhausted).is_err());
    assert!(flow::analyze(&input, &mut Limits::default()).is_ok());

    // A checked associated value contributes its domain, never its initializer
    // or a fabricated constant. Reading u8 is valid; incrementing an unknown
    // u8 still needs an overflow proof.
    let mut associated = function(Primitive::U8, Term::Integer(Integer::from_u128(0)));
    let K::IntegerBinary { left, .. } = &mut associated.body.kind else {
        unreachable!()
    };
    left.kind = K::AssociatedValue {
        interface: nir::program::InterfaceInstance {
            declaration: DeclarationId {
                package: 1,
                local: 10,
            },
            arguments: vec![],
        },
        member: DeclarationId {
            package: 1,
            local: 11,
        },
        subject: Term::Primitive(Primitive::Bool),
    };
    let read = Function {
        body: *left.clone(),
        ..associated.clone()
    };
    assert!(flow::analyze(&read, &mut Limits::default()).is_ok());
    assert_eq!(
        flow::analyze(&associated, &mut Limits::default())
            .err()
            .unwrap()
            .reason,
        flow::Reason::MissingMachineProof,
    );
}

#[test]
fn checked_nonzero_division_preserves_both_signs_and_machine_obligations() {
    use elaboration::verification::flow;
    use nir::program::{
        Expr, ExprKind as K, Function, FunctionPhase, IntegerOp, Literal, LocalId, Place,
        SourceRange,
    };
    use nir::{DeclarationId, Parameter, Primitive, Sort, Term};
    let ty = Term::Primitive(Primitive::I8);
    let divisor = LocalId { owner: 1, index: 0 };
    let expression = |kind, ty| Expr {
        kind,
        ty,
        source: SourceRange { start: 0, end: 1 },
    };
    let literal = |value| expression(K::Literal(Literal::SignedInteger(value)), ty.clone());
    let function = |op, numerator, checked, overwrite| {
        let mut statements = vec![];
        if checked {
            let condition = expression(
                K::IntegerBinary {
                    op: IntegerOp::NotEqual,
                    left: Box::new(expression(K::Local(divisor), ty.clone())),
                    right: Box::new(literal(0)),
                },
                Term::Primitive(Primitive::Bool),
            );
            statements.push(expression(
                K::CheckPredicate(Box::new(condition)),
                Term::Primitive(Primitive::Void),
            ));
        }
        if overwrite {
            statements.push(expression(
                K::Assign {
                    place: Place {
                        root: divisor,
                        fields: vec![],
                    },
                    value: Box::new(literal(0)),
                },
                Term::Primitive(Primitive::Void),
            ));
        }
        Function {
            declaration: DeclarationId {
                package: 1,
                local: 1,
            },
            name: "checked_division".into(),
            phase: FunctionPhase::Runtime,
            inline: false,
            pure: false,
            is_unsafe: false,
            signature: Term::Function {
                parameters: vec![Parameter {
                    sort: Sort::Runtime,
                    ty: ty.clone(),
                }],
                result: Box::new(ty.clone()),
            },
            parameters: vec![divisor],
            type_parameters: vec![],
            defaults: vec![],
            preconditions: vec![],
            postconditions: vec![],
            result_binding: None,
            entry_snapshots: vec![],
            capabilities: vec![],
            requirements: vec![],
            body: expression(
                K::Block {
                    statements,
                    result: Some(Box::new(expression(
                        K::IntegerBinary {
                            op,
                            left: Box::new(literal(numerator)),
                            right: Box::new(expression(K::Local(divisor), ty.clone())),
                        },
                        ty.clone(),
                    ))),
                },
                ty.clone(),
            ),
        }
    };
    for op in [IntegerOp::DivideTrunc, IntegerOp::RemainderTrunc] {
        for numerator in [-127, -7, 0, 7, 127] {
            let input = function(op, numerator, true, false);
            let report = flow::analyze(&input, &mut Limits::default())
                .unwrap_or_else(|error| panic!("{op:?}/{numerator}: {error}"));
            let range = report
                .points
                .iter()
                .rev()
                .find(|point| point.kind == flow::PointKind::Arithmetic(op))
                .and_then(|point| point.range.as_ref())
                .unwrap();
            // Exhaust all machine divisors, independently of the abstract transfer.
            for denominator in -128i128..=127 {
                if denominator == 0 {
                    continue;
                }
                let value = Integer::from_i128(if op == IntegerOp::DivideTrunc {
                    numerator / denominator
                } else {
                    numerator % denominator
                });
                assert!(range.lower() <= &value && &value <= range.upper());
            }
            let mut exhausted = Limits::default();
            exhausted.arithmetic.steps = 0;
            assert!(flow::analyze(&input, &mut exhausted).is_err());
            assert!(flow::analyze(&input, &mut Limits::default()).is_ok());
        }
        for (checked, overwrite) in [(false, false), (true, true)] {
            assert_eq!(
                flow::analyze(&function(op, 7, checked, overwrite), &mut Limits::default())
                    .err()
                    .unwrap()
                    .reason,
                flow::Reason::MissingDivisionProof
            );
        }
        // MIN % -1 also needs the quotient to fit the machine kernel.
        assert_eq!(
            flow::analyze(&function(op, -128, true, false), &mut Limits::default())
                .err()
                .unwrap()
                .reason,
            flow::Reason::MissingMachineProof
        );
    }
}
