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

fn evaluate(spelling: &str, domain: P) -> Result<Value, comptime_interpretor::Error> {
    let package = package(expr(domain, K::Literal(Literal::Float(spelling.into()))));
    let mut interpreter = Interpreter::for_comptime(&package, Budget::default())?;
    interpreter.invoke(package.functions[0].declaration, vec![])
}

#[test]
fn exact_float_evaluation_and_freezing_preserve_all_decimal_digits() {
    let a = evaluate("1.0000000000000000000000000000001", P::Float).unwrap();
    let equivalent = evaluate("10.000000000000000000000000000001e-1", P::Float).unwrap();
    let b = evaluate("1.0000000000000000000000000000002", P::Float).unwrap();
    assert_eq!(a, equivalent);
    assert_ne!(a, b);
    let mut budget = comptime_runtime::Budget {
        steps: 10000,
        max_limbs: 4096,
    };
    let term = a.clone().into_normal_term(&mut budget).unwrap();
    let Term::Float(value) = &term else {
        panic!("expected exact Float")
    };
    assert_eq!(value.digits(), "10000000000000000000000000000001");
    assert_eq!(a.to_string(), term.to_string());
    assert_eq!(term, equivalent.into_normal_term(&mut budget).unwrap());
    assert_ne!(term, b.into_normal_term(&mut budget).unwrap());
}

#[test]
fn exact_float_rejects_wrong_domains_invalid_literals_and_exhausted_export() {
    for domain in [P::F32, P::F64, P::Integer] {
        assert!(evaluate("1.25", domain).is_err());
    }
    for spelling in ["NaN", "inf", "1e", ""] {
        assert!(evaluate(spelling, P::Float).is_err());
    }
    let value = evaluate("123456789012345678901234567890.5", P::Float).unwrap();
    let mut empty = comptime_runtime::Budget {
        steps: 0,
        max_limbs: 4096,
    };
    assert_eq!(
        value.clone().into_normal_term(&mut empty),
        Err(ErrorKind::StepsExhausted)
    );
    let mut small = comptime_runtime::Budget {
        steps: 10000,
        max_limbs: 1,
    };
    assert_eq!(
        value.clone().into_normal_term(&mut small),
        Err(ErrorKind::RepresentationSizeLimit)
    );
    let mut sufficient = comptime_runtime::Budget {
        steps: 10000,
        max_limbs: 4096,
    };
    assert!(value.into_normal_term(&mut sufficient).is_ok());
}

fn compare_engines(body: Expr, expected: Result<Value, ErrorKind>) {
    let package = package(body);
    let id = package.functions[0].declaration;
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    let mut tree = Interpreter::new(&package, Budget::default()).unwrap();
    for _ in 0..2 {
        assert_eq!(vm.invoke(id, vec![]).map_err(|error| error.kind), expected);
        assert_eq!(
            tree.invoke(id, vec![]).map_err(|error| error.kind),
            expected
        );
    }
    assert_eq!(vm.bytecode_invocations(), 2);
    assert_eq!(vm.bytecode_compilations(), 1);
}

#[test]
fn float_bytecode_negation_preserves_zero_and_nan_payload_bits() {
    for (domain, input, expected) in [
        (P::F16, Literal::Float16(0), Value::Float16(0x8000)),
        (P::Bf16, Literal::BFloat16(0), Value::BFloat16(0x8000)),
        (P::F32, Literal::Float32(0), Value::Float32(0x80000000)),
        (
            P::F64,
            Literal::Float64(0),
            Value::Float64(0x8000000000000000),
        ),
        (P::F16, Literal::Float16(0x7e13), Value::Float16(0xfe13)),
        (P::Bf16, Literal::BFloat16(0x7fc3), Value::BFloat16(0xffc3)),
        (
            P::F32,
            Literal::Float32(0x7fc00013),
            Value::Float32(0xffc00013),
        ),
        (
            P::F64,
            Literal::Float64(0x7ff8000000000013),
            Value::Float64(0xfff8000000000013),
        ),
    ] {
        compare_engines(
            expr(
                domain,
                K::FloatNegate(Box::new(expr(domain, K::Literal(input)))),
            ),
            Ok(expected),
        );
    }
}

#[test]
fn float_bytecode_preserves_rounding_and_exact_decimal_arithmetic() {
    use nir::program::FloatOp;
    for (domain, a, b, expected) in [
        // One plus half an ULP rounds to even in each precision.
        (
            P::F16,
            Literal::Float16(0x3c00),
            Literal::Float16(0x1000),
            Value::Float16(0x3c00),
        ),
        (
            P::Bf16,
            Literal::BFloat16(0x3f80),
            Literal::BFloat16(0x3b80),
            Value::BFloat16(0x3f80),
        ),
        (
            P::F32,
            Literal::Float32(0x3f800000),
            Literal::Float32(0x33800000),
            Value::Float32(0x3f800000),
        ),
        (
            P::F64,
            Literal::Float64(0x3ff0000000000000),
            Literal::Float64(0x3ca0000000000000),
            Value::Float64(0x3ff0000000000000),
        ),
    ] {
        compare_engines(
            expr(
                domain,
                K::FloatBinary {
                    op: FloatOp::Add,
                    left: Box::new(expr(domain, K::Literal(a))),
                    right: Box::new(expr(domain, K::Literal(b))),
                },
            ),
            Ok(expected),
        );
    }
    for (op, expected) in [
        (FloatOp::Add, "0.3"),
        (FloatOp::Subtract, "-0.1"),
        (FloatOp::Multiply, "0.02"),
    ] {
        compare_engines(
            expr(
                P::Float,
                K::FloatBinary {
                    op,
                    left: Box::new(expr(P::Float, K::Literal(Literal::Float("0.1".into())))),
                    right: Box::new(expr(P::Float, K::Literal(Literal::Float("0.2".into())))),
                },
            ),
            Ok(evaluate(expected, P::Float).unwrap()),
        );
    }
    compare_engines(
        expr(
            P::Float,
            K::FloatBinary {
                op: FloatOp::Divide,
                left: Box::new(expr(P::Float, K::Literal(Literal::Float("1.0".into())))),
                right: Box::new(expr(P::Float, K::Literal(Literal::Float("3.0".into())))),
            },
        ),
        Err(ErrorKind::Type),
    );
}
