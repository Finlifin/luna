use comptime_interpretor::{Budget, ErrorKind, Interpreter, Value};
use nir::program::{Expr, ExprKind as K, Function, FunctionPhase, Literal, Package, SourceRange};
use nir::{DeclarationId, Primitive as P, Term};

fn expr(ty: P, kind: K) -> Expr {
    Expr {
        ty: Term::Primitive(ty),
        kind,
        source: SourceRange { start: 1, end: 2 },
    }
}
fn package(body: Expr) -> Package {
    Package {
        functions: vec![Function {
            declaration: DeclarationId {
                package: 1,
                local: 0,
            },
            name: "compare".into(),
            phase: FunctionPhase::Comptime,
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
                result: Box::new(body.ty.clone()),
            },
            parameters: vec![],
            defaults: vec![],
            body,
        }],
        ..Default::default()
    }
}

#[test]
fn platform_literal_invocation_uses_package_target() {
    for (width, fits) in [(32, false), (64, true)] {
        let mut package = package(expr(
            P::Usize,
            K::Literal(Literal::UnsignedInteger(1u128 << 32)),
        ));
        package.target = Some(nir::target::TargetData::new(width).unwrap());
        let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
        assert_eq!(vm.target(), package.target);
        let result = vm.invoke(package.functions[0].declaration, vec![]);
        if fits {
            assert_eq!(result.unwrap(), Value::UnsignedInteger(1u128 << 32));
        } else {
            assert_eq!(result.unwrap_err().kind, ErrorKind::Type);
        }
    }
    let package = package(expr(P::Usize, K::Literal(Literal::UnsignedInteger(0))));
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    assert_eq!(vm.target(), None);
    assert!(vm.invoke(package.functions[0].declaration, vec![]).is_err());
}

#[test]
fn iir_interpreter_preserves_explicit_target() {
    for width in [32, 64] {
        let graph = iir::Package {
            target: Some(nir::target::TargetData::new(width).unwrap()),
            ..Default::default()
        };
        let vm = Interpreter::from_iir(&graph, Budget::default()).unwrap();
        assert_eq!(vm.target(), graph.target);
        assert!(
            graph
                .to_string()
                .contains(&format!("(target-data (pointer-bits {width}))"))
        );
    }
}

fn evaluate_target(body: Expr, width: u32) -> Result<Value, comptime_interpretor::Error> {
    let mut package = package(body);
    package.target = Some(nir::target::TargetData::new(width).unwrap());
    let mut vm = Interpreter::for_comptime(&package, Budget::default())?;
    let result = vm.invoke(package.functions[0].declaration, vec![]);
    assert_eq!(vm.bytecode_invocations(), 1);
    let tree = Interpreter::new(&package, Budget::default())?
        .invoke(package.functions[0].declaration, vec![]);
    assert_eq!(
        result.as_ref().map_err(|error| &error.kind),
        tree.as_ref().map_err(|error| &error.kind)
    );
    result
}

#[test]
fn platform_arithmetic_uses_target_overflow_and_signed_division() {
    use nir::program::IntegerOp as Op;
    let unsigned = |n| expr(P::Usize, K::Literal(Literal::UnsignedInteger(n)));
    let signed = |n| expr(P::Isize, K::Literal(Literal::SignedInteger(n)));
    let binary = |ty, op, a, b| {
        expr(
            ty,
            K::IntegerBinary {
                op,
                left: Box::new(a),
                right: Box::new(b),
            },
        )
    };
    let sum = binary(P::Usize, Op::Add, unsigned(u32::MAX as u128), unsigned(1));
    assert_eq!(
        evaluate_target(sum.clone(), 32).unwrap_err().kind,
        ErrorKind::MachineOverflow
    );
    assert_eq!(
        evaluate_target(sum, 64).unwrap(),
        Value::UnsignedInteger(1u128 << 32)
    );
    for width in [32, 64] {
        for (op, result) in [
            (Op::DivideTrunc, -4),
            (Op::RemainderTrunc, -1),
            (Op::Multiply, -68),
            (Op::Subtract, -21),
        ] {
            assert_eq!(
                evaluate_target(binary(P::Isize, op, signed(-17), signed(4)), width).unwrap(),
                Value::SignedInteger(result)
            );
        }
        assert_eq!(
            evaluate_target(
                binary(P::Usize, Op::Subtract, unsigned(0), unsigned(1)),
                width
            )
            .unwrap_err()
            .kind,
            ErrorKind::MachineOverflow
        );
        assert_eq!(
            evaluate_target(
                binary(P::Isize, Op::DivideTrunc, signed(1), signed(0)),
                width
            )
            .unwrap_err()
            .kind,
            ErrorKind::DivisionByZero
        );
        let min = -(1i128 << (width - 1));
        assert_eq!(
            evaluate_target(
                expr(P::Isize, K::IntegerNegate(Box::new(signed(min)))),
                width
            )
            .unwrap_err()
            .kind,
            ErrorKind::MachineOverflow
        );
        assert_eq!(
            evaluate_target(
                binary(P::Isize, Op::DivideTrunc, signed(min), signed(-1)),
                width
            )
            .unwrap_err()
            .kind,
            ErrorKind::MachineOverflow
        );
    }
}

#[test]
fn platform_captured_integer_converts_with_target_domain() {
    for (primitive, n, expected) in [
        (P::Usize, 1i128 << 32, Value::UnsignedInteger(1u128 << 32)),
        (
            P::Isize,
            i32::MIN as i128 - 1,
            Value::SignedInteger(i32::MIN as i128 - 1),
        ),
    ] {
        let body = expr(
            primitive,
            K::ComptimeValue(Term::Integer(comptime_runtime::Integer::from_i128(n))),
        );
        assert!(evaluate_target(body.clone(), 32).is_err());
        assert_eq!(evaluate_target(body, 64).unwrap(), expected);
    }
}

#[test]
fn platform_comparisons_and_staging_agree_on_target_domains() {
    for width in [32, 64] {
        let package = Package {
            target: Some(nir::target::TargetData::new(width).unwrap()),
            ..Default::default()
        };
        for (primitive, a, b) in [(P::Isize, -7i128, 3i128), (P::Usize, 3, 7)] {
            let operand = |n| {
                expr(
                    primitive,
                    K::Literal(if primitive == P::Isize {
                        Literal::SignedInteger(n)
                    } else {
                        Literal::UnsignedInteger(n as u128)
                    }),
                )
            };
            for (op, expected) in [(0, false), (1, true), (2, true), (3, false), (4, false)] {
                let a = Box::new(operand(a));
                let b = Box::new(operand(b));
                let body = expr(
                    P::Bool,
                    match op {
                        0 => K::ScalarEqual(a, b),
                        1 => K::ScalarLess(a, b),
                        2 => K::ScalarLessEqual(a, b),
                        3 => K::ScalarGreater(a, b),
                        _ => K::ScalarGreaterEqual(a, b),
                    },
                );
                assert_eq!(
                    evaluate_target(body.clone(), width).unwrap(),
                    Value::Bool(expected)
                );
                let folded =
                    iir::staging::fold_value_with_layouts(&package, &body, &mut 10000).unwrap();
                assert_eq!(folded.kind, K::Literal(Literal::Bool(expected)));
                assert!(iir::staging::fold_value(&body, &mut 10000).is_err());
            }
        }
    }
    let large = expr(P::Usize, K::Literal(Literal::UnsignedInteger(1u128 << 32)));
    let comparison = expr(
        P::Bool,
        K::ScalarEqual(Box::new(large.clone()), Box::new(large)),
    );
    assert!(evaluate_target(comparison.clone(), 32).is_err());
    let narrow = Package {
        target: Some(nir::target::TargetData::new(32).unwrap()),
        ..Default::default()
    };
    assert!(iir::staging::fold_value_with_layouts(&narrow, &comparison, &mut 10000).is_err());
    assert_eq!(evaluate_target(comparison, 64).unwrap(), Value::Bool(true));
}

#[test]
fn staged_platform_arithmetic_rejects_narrow_overflow() {
    let body = expr(
        P::Usize,
        K::IntegerBinary {
            op: nir::program::IntegerOp::Add,
            left: Box::new(expr(
                P::Usize,
                K::Literal(Literal::UnsignedInteger(u32::MAX as u128)),
            )),
            right: Box::new(expr(P::Usize, K::Literal(Literal::UnsignedInteger(1)))),
        },
    );
    for width in [32, 64] {
        let package = Package {
            target: Some(nir::target::TargetData::new(width).unwrap()),
            ..Default::default()
        };
        let result = iir::staging::fold_value_with_layouts(&package, &body, &mut 10000);
        if width == 32 {
            assert!(result.is_err());
        } else {
            let value = result.unwrap();
            assert_eq!(value.ty, Term::Primitive(P::Usize));
            assert_eq!(
                value.kind,
                K::Literal(Literal::UnsignedInteger(1u128 << 32))
            );
        }
    }
}
