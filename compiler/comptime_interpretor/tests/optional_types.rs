use comptime_interpretor::{Budget, ErrorKind, Interpreter, Value};
use nir::program::{Expr, ExprKind as K, Function, FunctionPhase, Package, SourceRange};
use nir::{DeclarationId, Primitive as P, Term};

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
            result: Box::new(p(P::Type)),
        },
        parameters: vec![],
        defaults: vec![],
        body,
    }
}
#[test]
fn optional_types_are_comptime_values_without_an_enum_world() {
    let nested = Term::Optional(Box::new(Term::Optional(Box::new(p(P::Bool)))));
    let malformed = Term::Optional(Box::new(Term::Bool(true)));
    let package = Package {
        functions: vec![
            function(0, value(nested.clone())),
            function(1, value(malformed)),
        ],
        ..Default::default()
    };
    assert!(package.enums.is_empty());
    let mut interpreter = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    assert_eq!(
        interpreter.invoke(id(0), vec![]).unwrap(),
        Value::Type(nested.clone())
    );
    assert_eq!(
        interpreter.invoke(id(1), vec![]).unwrap_err().kind,
        ErrorKind::Type
    );
    assert_eq!(
        interpreter.invoke(id(0), vec![]).unwrap(),
        Value::Type(nested)
    );
}

#[test]
fn optional_type_admission_meters_payloads_and_recovers_with_sufficient_budget() {
    let nested = (0..20).fold(p(P::Bool), |inner, _| Term::Optional(Box::new(inner)));
    let package = Package {
        functions: vec![function(0, value(nested.clone()))],
        ..Default::default()
    };
    let mut low = Interpreter::for_comptime(
        &package,
        Budget {
            steps: 8,
            depth: 128,
        },
    )
    .unwrap();
    assert_eq!(
        low.invoke(id(0), vec![]).unwrap_err().kind,
        ErrorKind::StepsExhausted
    );
    let mut enough = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    assert_eq!(enough.invoke(id(0), vec![]).unwrap(), Value::Type(nested));
}

fn optional_type(inner: Expr) -> Expr {
    expr(p(P::Type), K::OptionalType(Box::new(inner)))
}

#[test]
fn optional_construction_evaluates_operand_once_and_preserves_nested_opaque_identity() {
    use comptime_interpretor::Event;
    let opaque = Term::NormalForm {
        head: id(90),
        sort: nir::Sort::Type,
        arguments: vec![p(P::Bool)],
    };
    let signature = Term::Function {
        parameters: vec![],
        result: Box::new(p(P::Type)),
    }
    .with_comptime_phase(true);
    let call = expr(
        p(P::Type),
        K::Call {
            parameter_bindings: vec![],
            callee: Box::new(expr(signature, K::Function(id(1)))),
            arguments: vec![],
        },
    );
    let package = Package {
        functions: vec![
            function(0, optional_type(optional_type(call))),
            function(1, value(opaque.clone())),
        ],
        ..Default::default()
    };
    let mut interpreter = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    assert_eq!(
        interpreter.invoke(id(0), vec![]).unwrap(),
        Value::Type(Term::Optional(Box::new(Term::Optional(Box::new(opaque)))))
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
}

#[test]
fn optional_construction_rejects_invalid_operands_and_recovers() {
    for invalid in [
        optional_type(value(Term::Bool(true))),
        optional_type(expr(
            p(P::Bool),
            K::Literal(nir::program::Literal::Bool(true)),
        )),
        expr(p(P::Bool), K::OptionalType(Box::new(value(p(P::Char))))),
    ] {
        let package = Package {
            functions: vec![
                function(0, invalid),
                function(1, optional_type(value(p(P::Char)))),
            ],
            ..Default::default()
        };
        let mut interpreter = Interpreter::for_comptime(&package, Budget::default()).unwrap();
        assert_eq!(
            interpreter.invoke(id(0), vec![]).unwrap_err().kind,
            ErrorKind::Type
        );
        assert_eq!(
            interpreter.invoke(id(1), vec![]).unwrap(),
            Value::Type(Term::Optional(Box::new(p(P::Char))))
        );
    }
}

#[test]
fn optional_constructor_specializes_without_mutating_template_and_is_metered() {
    let parameter = id(99);
    let template = optional_type(optional_type(value(Term::Rigid {
        declaration: parameter,
        sort: nir::Sort::Type,
    })));
    let original = template.clone();
    let body = template
        .instantiate_types(&[parameter], &[p(P::Bool)])
        .unwrap();
    assert_eq!(template, original);
    assert_ne!(template, body);
    assert!(body.to_string().contains("(optional-type-value"));
    let package = Package {
        functions: vec![function(0, body)],
        ..Default::default()
    };
    let mut low = Interpreter::for_comptime(
        &package,
        Budget {
            steps: 2,
            depth: 128,
        },
    )
    .unwrap();
    assert_eq!(
        low.invoke(id(0), vec![]).unwrap_err().kind,
        ErrorKind::StepsExhausted
    );
    let mut enough = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    assert_eq!(
        enough.invoke(id(0), vec![]).unwrap(),
        Value::Type(Term::Optional(Box::new(Term::Optional(Box::new(p(
            P::Bool
        ))))))
    );
}
