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
