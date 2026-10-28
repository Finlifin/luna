//! Source-independent behavior checks retain P3/P4 and diagnostic boundaries.
use nir::program::{Expr, ExprKind, Function, FunctionPhase, LocalId, Package, SourceRange};
use nir::{DeclarationId, Primitive, Term};
use program_verification::purity::{Protocols, validate_defaults, validate_runtime_promises};
use std::collections::BTreeSet;

#[test]
fn nonterminating_private_loop_is_p4_but_not_a_stable_default() {
    let site = SourceRange { start: 10, end: 20 };
    let loop_id = LocalId { owner: 0, index: 0 };
    let body = Expr {
        source: site,
        ty: Term::Primitive(Primitive::Never),
        kind: ExprKind::Loop {
            identity: loop_id,
            body: Box::new(Expr {
                source: site,
                ty: Term::Primitive(Primitive::Void),
                kind: ExprKind::Unit,
            }),
        },
    };
    let package = Package {
        functions: vec![Function {
            phase: FunctionPhase::Runtime,
            inline: false,
            pure: true,
            is_unsafe: false,
            preconditions: vec![],
            postconditions: vec![],
            result_binding: None,
            entry_snapshots: vec![],
            capabilities: vec![],
            requirements: vec![],
            type_parameters: vec![],
            declaration: DeclarationId {
                package: 0,
                local: 0,
            },
            name: "spin".into(),
            signature: Term::Function {
                parameters: vec![],
                result: Box::new(body.ty.clone()),
            },
            parameters: vec![],
            body: body.clone(),
            defaults: vec![],
        }],
        ..Default::default()
    };
    let protocols = Protocols::default();
    validate_runtime_promises(&package, &protocols).unwrap();
    let staged = BTreeSet::new();
    let default_site = SourceRange {
        start: 100,
        end: 120,
    };
    let failure =
        validate_defaults(&package, [(&body, default_site, &staged)], &protocols).unwrap_err();
    assert_eq!(failure.source, default_site);
    assert!(failure.message.contains("P3"));
    let value = Expr {
        source: site,
        ty: Term::Primitive(Primitive::Bool),
        kind: ExprKind::Literal(nir::program::Literal::Bool(true)),
    };
    validate_defaults(&package, [(&value, default_site, &staged)], &protocols).unwrap();
    validate_runtime_promises(&package, &protocols).unwrap();
}
