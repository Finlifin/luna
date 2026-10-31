use nir::code_values::{ExpressionCode, PatternCode};
use nir::program::{Expr, ExprKind, LocalId, Package, Pattern, SourceRange};
use nir::{Primitive, Term};
use std::{collections::BTreeMap, sync::Arc};

fn binding(index: u32) -> LocalId {
    LocalId { owner: 7, index }
}

fn expression() -> ExpressionCode {
    let ty = Term::Primitive(Primitive::Bool);
    ExpressionCode {
        body: Some(Arc::new(Expr {
            ty: ty.clone(),
            source: SourceRange { start: 0, end: 1 },
            kind: ExprKind::Local(binding(1)),
        })),
        result: ty.clone(),
        context: BTreeMap::from([(binding(1), ty.clone()), (binding(2), ty)]),
        ports: BTreeMap::from([("input\"\n".into(), binding(1))]),
        defaults: BTreeMap::from([(binding(1), binding(9)), (binding(2), binding(2))]),
    }
}

#[test]
fn contextual_transport_retains_fixed_dependencies_and_external_defaults() {
    let code = Arc::new(expression());
    let received = Arc::clone(&code);
    received
        .validate_runtime_interface(&Package::default(), &[])
        .unwrap();
    assert!(Arc::ptr_eq(&code, &received));
    assert!(received.context.contains_key(&binding(2)));
    assert!(!received.ports.values().any(|id| *id == binding(2)));
    assert_eq!(received.defaults[&binding(1)], binding(9));
    assert_eq!(
        received.body.as_ref().unwrap().kind,
        ExprKind::Local(binding(1))
    );
    let dump = received.to_string();
    assert!(dump.contains("\"input\\\"\\n\""), "{dump}");
    assert!(dump.starts_with("(expression-code (context"));
    assert_eq!(dump, code.to_string());
}

#[test]
fn malformed_port_and_default_sources_are_not_transportable_interfaces() {
    let package = Package::default();
    let mut code = expression();
    code.ports.insert("missing".into(), binding(8));
    assert_eq!(
        code.validate_runtime_interface(&package, &[]),
        Err("code port is absent from its runtime context")
    );
    code.ports.remove("missing");
    code.defaults.insert(binding(8), binding(9));
    assert_eq!(
        code.validate_runtime_interface(&package, &[]),
        Err("code default is absent from its runtime context")
    );
    code.defaults.remove(&binding(8));
    code.validate_runtime_interface(&package, &[]).unwrap();
}

#[test]
fn quoted_interfaces_reject_comptime_storage_on_each_side() {
    let package = Package::default();
    for invalid in [
        Term::Primitive(Primitive::Type),
        Term::Optional(Box::new(Term::Primitive(Primitive::Type))),
    ] {
        let mut code = expression();
        code.result = invalid.clone();
        assert!(code.validate_runtime_interface(&package, &[]).is_err());
        code.result = Term::Primitive(Primitive::Bool);
        code.context.insert(binding(2), invalid.clone());
        assert!(code.validate_runtime_interface(&package, &[]).is_err());
        let mut pattern = PatternCode {
            subject: invalid.clone(),
            outputs: BTreeMap::new(),
            body: Some(Pattern::Wild),
        };
        assert!(pattern.validate_runtime_interface(&package, &[]).is_err());
        pattern.subject = Term::Primitive(Primitive::Bool);
        pattern
            .outputs
            .insert("value".into(), (binding(1), invalid));
        assert!(pattern.validate_runtime_interface(&package, &[]).is_err());
    }
}

#[test]
fn definition_evidence_does_not_acquire_a_body_during_interface_validation() {
    let mut code = expression();
    code.body = None;
    code.validate_runtime_interface(&Package::default(), &[])
        .unwrap();
    assert!(code.body.is_none());
    assert!(code.to_string().contains("(body pending-code-parameter)"));
    let pattern = PatternCode {
        subject: Term::Primitive(Primitive::Bool),
        outputs: BTreeMap::from([(
            "value\"\n".into(),
            (binding(3), Term::Primitive(Primitive::Bool)),
        )]),
        body: None,
    };
    pattern
        .validate_runtime_interface(&Package::default(), &[])
        .unwrap();
    assert!(pattern.body.is_none());
    let dump = pattern.to_string();
    assert!(dump.contains("(body pending-pattern-code)"), "{dump}");
    assert!(dump.contains("\"value\\\"\\n\""), "{dump}");
}
