use comptime_interpretor::{Budget, ErrorKind, Interpreter, Value};
use nir::program::{Expr, ExprKind as K, Function, FunctionPhase, LocalId, Package, SourceRange};
use nir::{DeclarationId, Parameter, Primitive as P, Sort, Term};

fn ty() -> Term {
    Term::Record(vec![
        ("enabled".into(), Term::Primitive(P::Bool)),
        (
            "nested".into(),
            Term::Record(vec![("count".into(), Term::Primitive(P::U8))]),
        ),
    ])
}
fn valid() -> Value {
    Value::AnonymousRecord(vec![
        ("enabled".into(), Value::Bool(true)),
        (
            "nested".into(),
            Value::AnonymousRecord(vec![("count".into(), Value::UnsignedInteger(7))]),
        ),
    ])
}
fn identity(ty: Term) -> Package {
    let binding = LocalId { owner: 1, index: 0 };
    Package {
        functions: vec![Function {
            declaration: DeclarationId {
                package: 1,
                local: 0,
            },
            name: "identity".into(),
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
                parameters: vec![Parameter {
                    sort: Sort::Runtime,
                    ty: ty.clone(),
                }],
                result: Box::new(ty.clone()),
            },
            parameters: vec![binding],
            defaults: vec![],
            body: Expr {
                ty,
                kind: K::Local(binding),
                source: SourceRange { start: 1, end: 2 },
            },
        }],
        ..Default::default()
    }
}

#[test]
fn named_record_values_cross_typed_calls_without_losing_labels() {
    let package = identity(ty());
    let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
    let value = valid();
    assert_eq!(
        vm.invoke(package.functions[0].declaration, vec![value.clone()])
            .unwrap(),
        value
    );
}

#[test]
fn malformed_record_inputs_reject_before_delivery_and_later_calls_recover() {
    let package = identity(ty());
    let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
    let id = package.functions[0].declaration;
    let nested = || Value::AnonymousRecord(vec![("count".into(), Value::UnsignedInteger(7))]);
    for bad in [
        Value::Tuple(vec![Value::Bool(true), nested()]),
        Value::AnonymousRecord(vec![("enabled".into(), Value::Bool(true))]),
        Value::AnonymousRecord(vec![
            ("wrong".into(), Value::Bool(true)),
            ("nested".into(), nested()),
        ]),
        Value::AnonymousRecord(vec![
            ("enabled".into(), Value::Bool(true)),
            ("enabled".into(), nested()),
        ]),
        Value::AnonymousRecord(vec![
            ("enabled".into(), Value::Bool(true)),
            (
                "nested".into(),
                Value::AnonymousRecord(vec![("count".into(), Value::UnsignedInteger(256))]),
            ),
        ]),
    ] {
        assert_eq!(vm.invoke(id, vec![bad]).unwrap_err().kind, ErrorKind::Type);
        assert_eq!(vm.invoke(id, vec![valid()]).unwrap(), valid());
    }
}

#[test]
fn empty_named_records_are_not_void_or_positional_tuples() {
    let package = identity(Term::Record(vec![]));
    let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
    let id = package.functions[0].declaration;
    assert_eq!(
        vm.invoke(id, vec![Value::AnonymousRecord(vec![])]).unwrap(),
        Value::AnonymousRecord(vec![])
    );
    for bad in [Value::Unit, Value::Tuple(vec![])] {
        assert_eq!(vm.invoke(id, vec![bad]).unwrap_err().kind, ErrorKind::Type);
    }
}

#[test]
fn record_values_have_escaped_dumps_and_do_not_freeze_as_type_terms() {
    let value = Value::AnonymousRecord(vec![("quote\"\n".into(), Value::Bool(true))]);
    assert_eq!(
        value.to_string(),
        r#"(anonymous-record (fields (field (string "quote\"\n") (bool true))))"#
    );
    let frozen = value
        .clone()
        .into_normal_term(&mut comptime_runtime::Budget::default())
        .unwrap();
    assert_eq!(
        frozen,
        Term::RecordValue(vec![("quote\"\n".into(), Term::Bool(true))])
    );
    assert_eq!(frozen.sort(), nir::Sort::ComptimeValue);
    println!("RECORD_VALUE_DUMP={value}\nRECORD_VALUE_END");
}

#[test]
fn deeply_nested_record_inputs_are_bounded() {
    let mut ty = Term::Primitive(P::Bool);
    let mut value = Value::Bool(true);
    for _ in 0..140 {
        ty = Term::Record(vec![("next".into(), ty)]);
        value = Value::AnonymousRecord(vec![("next".into(), value)]);
    }
    let package = identity(ty);
    let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
    assert_eq!(
        vm.invoke(package.functions[0].declaration, vec![value])
            .unwrap_err()
            .kind,
        ErrorKind::DepthExhausted
    );
}

#[test]
fn frozen_records_reconstruct_with_exact_labels_and_field_types() {
    let package = identity(ty());
    let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
    let id = package.functions[0].declaration;
    let frozen = valid()
        .into_normal_term(&mut comptime_runtime::Budget::default())
        .unwrap();
    assert_eq!(
        vm.invoke_with_normal_arguments(id, vec![(ty(), frozen.clone())])
            .unwrap(),
        valid()
    );
    let Term::RecordValue(mut fields) = frozen else {
        panic!()
    };
    fields[0].0 = "wrong".into();
    assert_eq!(
        vm.invoke_with_normal_arguments(id, vec![(ty(), Term::RecordValue(fields))])
            .unwrap_err()
            .kind,
        ErrorKind::Type
    );
    assert_eq!(
        vm.invoke_with_normal_arguments(id, vec![(ty(), Term::RecordValue(vec![]))])
            .unwrap_err()
            .kind,
        ErrorKind::Type
    );
}

#[test]
fn anonymous_record_pattern_checks_labels_and_has_stable_dump() {
    use nir::program::{MatchArm, Pattern};
    let mut package = identity(ty());
    let input = package.functions[0].body.clone();
    let pattern = Pattern::AnonymousRecord {
        names: vec!["enabled".into(), "nested".into()],
        fields: vec![Pattern::Wild, Pattern::Wild],
    };
    assert_eq!(
        pattern.to_string(),
        "(anonymous-record-pattern (names \"enabled\" \"nested\") (fields (wild) (wild)))"
    );
    package.functions[0].body = Expr {
        ty: Term::Primitive(P::Bool),
        source: input.source,
        kind: K::Match {
            value: Box::new(input),
            arms: vec![MatchArm {
                pattern,
                guard: None,
                body: Expr {
                    ty: Term::Primitive(P::Bool),
                    source: SourceRange { start: 0, end: 1 },
                    kind: K::Literal(nir::program::Literal::Bool(true)),
                },
            }],
        },
    };
    let Term::Function { result, .. } = &mut package.functions[0].signature else {
        panic!()
    };
    **result = Term::Primitive(P::Bool);
    let id = package.functions[0].declaration;
    let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
    assert_eq!(vm.invoke(id, vec![valid()]).unwrap(), Value::Bool(true));
    let K::Match { arms, .. } = &mut package.functions[0].body.kind else {
        panic!()
    };
    let Pattern::AnonymousRecord { names, .. } = &mut arms[0].pattern else {
        panic!()
    };
    names[0] = "wrong".into();
    let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
    assert_eq!(
        vm.invoke(id, vec![valid()]).unwrap_err().kind,
        ErrorKind::Type
    );
}

#[test]
fn unused_wide_arguments_still_pay_for_invocation_preparation() {
    let fields = (0..64)
        .map(|i| (format!("field_{i}"), Term::Primitive(P::Bool)))
        .collect();
    let mut package = identity(Term::Record(fields));
    let Term::Function { result, .. } = &mut package.functions[0].signature else {
        panic!()
    };
    *result = Box::new(Term::Primitive(P::Bool));
    package.functions[0].body = Expr {
        ty: Term::Primitive(P::Bool),
        source: SourceRange { start: 3, end: 4 },
        kind: K::Literal(nir::program::Literal::Bool(true)),
    };
    let id = package.functions[0].declaration;
    for owned in [false, true] {
        let value = Value::AnonymousRecord(
            (0..64)
                .map(|i| (format!("field_{i}"), Value::Bool(false)))
                .collect(),
        );
        let value = if owned {
            Value::Storage(comptime_interpretor::Storage::new(value))
        } else {
            value
        };
        let mut limited = Interpreter::new(
            &package,
            Budget {
                steps: 8,
                depth: 128,
            },
        )
        .unwrap();
        assert_eq!(
            limited.invoke(id, vec![value.clone()]).unwrap_err().kind,
            ErrorKind::StepsExhausted
        );
        let mut complete = Interpreter::new(&package, Budget::default()).unwrap();
        assert_eq!(complete.invoke(id, vec![value]).unwrap(), Value::Bool(true));
    }
}
