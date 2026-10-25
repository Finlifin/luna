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
fn equal(left: Expr, right: Expr) -> Expr {
    expr(P::Bool, K::ScalarEqual(Box::new(left), Box::new(right)))
}
#[test]
fn scalar_equality_respects_categories_and_integer_widths() {
    let cases = [
        (P::Bool, Literal::Bool(true), Literal::Bool(false), false),
        (P::Char, Literal::Char('雪'), Literal::Char('雪'), true),
        (
            P::Str,
            Literal::String("雪".into()),
            Literal::String("雨".into()),
            false,
        ),
        (
            P::I8,
            Literal::SignedInteger(-128),
            Literal::SignedInteger(127),
            false,
        ),
        (
            P::U128,
            Literal::UnsignedInteger(u128::MAX),
            Literal::UnsignedInteger(u128::MAX),
            true,
        ),
    ];
    for (ty, left, right, expected) in cases {
        let package = package(equal(
            expr(ty, K::Literal(left)),
            expr(ty, K::Literal(right)),
        ));
        let mut interpreter = Interpreter::for_comptime(&package, Budget::default()).unwrap();
        assert_eq!(
            interpreter
                .invoke(package.functions[0].declaration, vec![])
                .unwrap(),
            Value::Bool(expected)
        );
    }
}
#[test]
fn scalar_equality_rejects_malformed_nir_and_does_not_compare_types_by_storage() {
    for (left, right) in [
        (
            expr(P::F32, K::Literal(Literal::Float32(0))),
            expr(P::F64, K::Literal(Literal::Float64(0))),
        ),
        (
            expr(P::F32, K::Literal(Literal::Float64(0))),
            expr(P::F32, K::Literal(Literal::Float32(0))),
        ),
        (
            expr(P::U8, K::Literal(Literal::UnsignedInteger(1))),
            expr(P::U16, K::Literal(Literal::UnsignedInteger(1))),
        ),
        (
            expr(P::Type, K::TypeValue(Term::Primitive(P::Bool))),
            expr(P::Type, K::TypeValue(Term::Primitive(P::Bool))),
        ),
        (
            expr(P::U8, K::Literal(Literal::UnsignedInteger(256))),
            expr(P::U8, K::Literal(Literal::UnsignedInteger(0))),
        ),
        (
            expr(P::Bool, K::Literal(Literal::Char('a'))),
            expr(P::Bool, K::Literal(Literal::Bool(true))),
        ),
    ] {
        let package = package(equal(left, right));
        let mut interpreter = Interpreter::for_comptime(&package, Budget::default()).unwrap();
        assert_eq!(
            interpreter
                .invoke(package.functions[0].declaration, vec![])
                .unwrap_err()
                .kind,
            ErrorKind::Type
        );
    }
}

#[test]
fn scalar_order_uses_values_and_rejects_unimplemented_categories() {
    for (ty, left, right, expected) in [
        (
            P::I8,
            Literal::SignedInteger(-128),
            Literal::SignedInteger(127),
            true,
        ),
        (
            P::U128,
            Literal::UnsignedInteger(u128::MAX),
            Literal::UnsignedInteger(0),
            false,
        ),
        (P::Char, Literal::Char('雨'), Literal::Char('雨'), true),
    ] {
        let body = expr(
            P::Bool,
            K::ScalarLessEqual(
                Box::new(expr(ty, K::Literal(left))),
                Box::new(expr(ty, K::Literal(right))),
            ),
        );
        let package = package(body);
        let mut interpreter = Interpreter::for_comptime(&package, Budget::default()).unwrap();
        assert_eq!(
            interpreter
                .invoke(package.functions[0].declaration, vec![])
                .unwrap(),
            Value::Bool(expected)
        );
    }
    for (ty, literal) in [
        (P::Bool, Literal::Bool(false)),
        (P::Str, Literal::String("a".into())),
    ] {
        let operand = expr(ty, K::Literal(literal));
        let package = package(expr(
            P::Bool,
            K::ScalarLessEqual(Box::new(operand.clone()), Box::new(operand)),
        ));
        let mut interpreter = Interpreter::for_comptime(&package, Budget::default()).unwrap();
        assert_eq!(
            interpreter
                .invoke(package.functions[0].declaration, vec![])
                .unwrap_err()
                .kind,
            ErrorKind::Type
        );
    }
}

#[test]
fn scalar_comparisons_reject_wrong_result_types() {
    for op in [
        K::ScalarEqual,
        K::ScalarLess,
        K::ScalarLessEqual,
        K::ScalarGreater,
        K::ScalarGreaterEqual,
    ] {
        let operand = expr(P::F32, K::Literal(Literal::Float32(0)));
        let package = package(expr(
            P::F32,
            op(Box::new(operand.clone()), Box::new(operand)),
        ));
        let mut interpreter = Interpreter::for_comptime(&package, Budget::default()).unwrap();
        assert_eq!(
            interpreter
                .invoke(package.functions[0].declaration, vec![])
                .unwrap_err()
                .kind,
            ErrorKind::Type
        );
    }
}

#[test]
fn float_arithmetic_checks_nir_precision_and_result_types() {
    use nir::program::FloatOp;
    for (left, right, result) in [
        (P::F32, P::F64, P::F32),
        (P::F32, P::F32, P::Bool),
        (P::Bool, P::F32, P::F32),
    ] {
        let body = expr(
            result,
            K::FloatBinary {
                op: FloatOp::Add,
                left: Box::new(expr(left, K::Literal(Literal::Float32(0)))),
                right: Box::new(expr(right, K::Literal(Literal::Float32(0)))),
            },
        );
        let package = package(body);
        let mut interpreter = Interpreter::for_comptime(&package, Budget::default()).unwrap();
        assert_eq!(
            interpreter
                .invoke(package.functions[0].declaration, vec![])
                .unwrap_err()
                .kind,
            ErrorKind::Type
        );
    }
    for (input, result) in [(P::F32, P::F64), (P::Bool, P::Bool)] {
        let package = package(expr(
            result,
            K::FloatNegate(Box::new(expr(input, K::Literal(Literal::Float32(0))))),
        ));
        let mut interpreter = Interpreter::for_comptime(&package, Budget::default()).unwrap();
        assert_eq!(
            interpreter
                .invoke(package.functions[0].declaration, vec![])
                .unwrap_err()
                .kind,
            ErrorKind::Type
        );
    }
}
