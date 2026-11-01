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
fn construction(fields: Vec<Expr>) -> Expr {
    expr(
        p(P::Type),
        K::NormalFormType {
            head: id(100),
            arguments: fields
                .into_iter()
                .map(nir::program::NormalFormArgument::Value)
                .collect(),
            plan: None,
        },
    )
}
fn result(fields: Vec<Term>) -> Term {
    Term::NormalForm {
        head: id(100),
        sort: Sort::Type,
        arguments: fields,
    }
}
fn world(functions: Vec<Function>, arity: usize) -> Package {
    use nir::normal_forms::{Head, Kind, Parameter, Role};
    Package {
        functions,
        normal_forms: vec![Head {
            declaration: id(100),
            role: Role::Opaque,
            collector: None,
            requirements: vec![],
            parameters: (0..arity)
                .map(|index| Parameter {
                    declaration: id(200 + index as u32),
                    binding: nir::program::LocalId {
                        owner: 100,
                        index: index as u32,
                    },
                    kind: Kind::Type,
                    ty: p(P::Type),
                })
                .collect(),
        }],
        ..Default::default()
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

#[test]
fn inputs_execute_once_in_order_and_preserve_the_opaque_type_head() {
    let package = world(
        vec![
            function(0, construction(vec![call(1), call(2)])),
            function(1, value(p(P::Bool))),
            function(2, value(p(P::Char))),
        ],
        2,
    );
    let mut interpreter = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    assert_eq!(
        interpreter.invoke(id(0), vec![]).unwrap(),
        Value::Type(result(vec![p(P::Bool), p(P::Char)]))
    );
    let entered: Vec<_> = interpreter
        .events()
        .iter()
        .filter_map(|event| match event {
            Event::EnterFunction(id) => Some(*id),
            _ => None,
        })
        .collect();
    assert_eq!(entered, vec![id(0), id(1), id(2)]);
    assert_eq!(interpreter.bytecode_invocations(), 3);
}

#[test]
fn malformed_fields_fail_and_a_later_valid_invocation_recovers() {
    let mut wrong_output = construction(vec![value(p(P::Bool))]);
    wrong_output.ty = p(P::Bool);
    for (malformed, expected) in [
        (wrong_output, ErrorKind::Type),
        // A value-shaped operand lacks the Type evidence required by the
        // ordinary normal-form input compiler. It must not fall back to a tree.
        (
            construction(vec![expr(
                p(P::Bool),
                K::Literal(nir::program::Literal::Bool(true)),
            )]),
            ErrorKind::TypeContractUnresolved,
        ),
        (construction(vec![value(Term::Bool(true))]), ErrorKind::Type),
    ] {
        let package = world(
            vec![function(0, malformed), function(1, value(p(P::Char)))],
            1,
        );
        let mut interpreter = Interpreter::for_comptime(&package, Budget::default()).unwrap();
        assert_eq!(
            interpreter.invoke(id(0), vec![]).unwrap_err().kind,
            expected
        );
        assert_eq!(
            interpreter.invoke(id(1), vec![]).unwrap(),
            Value::Type(p(P::Char))
        );
    }
}

#[test]
fn normal_form_construction_is_budgeted_and_specialization_visits_inputs() {
    let parameter = id(9);
    let body = construction(vec![
        value(Term::Rigid {
            declaration: parameter,
            sort: Sort::Type
        });
        16
    ]);
    let specialized = body.instantiate_types(&[parameter], &[p(P::Bool)]).unwrap();
    assert_ne!(body, specialized);
    assert!(body.to_string().contains("normal-form-type"));
    let package = world(vec![function(0, specialized)], 16);
    let mut interpreter = Interpreter::for_comptime(
        &package,
        Budget {
            steps: 8,
            depth: 128,
        },
    )
    .unwrap();
    assert_eq!(
        interpreter.invoke(id(0), vec![]).unwrap_err().kind,
        ErrorKind::StepsExhausted
    );
    let mut interpreter = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    assert_eq!(
        interpreter.invoke(id(0), vec![]).unwrap(),
        Value::Type(result(vec![p(P::Bool); 16]))
    );
}

#[test]
fn a_failed_field_does_not_execute_later_fields() {
    let bad_width = expr(
        p(P::Type),
        K::BitVecType(Box::new(expr(
            p(P::Integer),
            K::Literal(nir::program::Literal::Integer("-1".into())),
        ))),
    );
    let package = world(
        vec![
            function(0, construction(vec![bad_width, call(1)])),
            function(1, value(p(P::Bool))),
        ],
        2,
    );
    let mut interpreter = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    assert_eq!(
        interpreter.invoke(id(0), vec![]).unwrap_err().kind,
        ErrorKind::InvalidBitWidth
    );
    assert!(!interpreter.events().contains(&Event::EnterFunction(id(1))));
    assert_eq!(
        interpreter.invoke(id(1), vec![]).unwrap(),
        Value::Type(p(P::Bool))
    );
}
