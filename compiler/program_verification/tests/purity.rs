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

#[test]
fn stable_defaults_discard_tuple_values_using_registered_collector_layout() {
    use nir::Sort;
    use nir::normal_forms::{Head, Kind, Parameter, Role};
    let id = |local| DeclarationId { package: 42, local };
    let source = SourceRange { start: 20, end: 30 };
    let domain = Term::NormalForm {
        head: id(0),
        sort: Sort::Type,
        arguments: vec![Term::Primitive(Primitive::Type)],
    };
    let head = Head {
        declaration: id(1),
        role: Role::Tuple,
        collector: Some(0),
        parameters: vec![Parameter {
            declaration: id(2),
            binding: LocalId { owner: 1, index: 0 },
            kind: Kind::Value,
            ty: domain.clone(),
        }],
        requirements: vec![],
    };
    let mut package = Package {
        normal_forms: vec![head],
        ..Default::default()
    };
    let tuple = |fields: Vec<Expr>| Expr {
        source,
        ty: Term::NormalForm {
            head: id(1),
            sort: Sort::Type,
            arguments: vec![Term::NominalValue {
                ty: Box::new(domain.clone()),
                variant: None,
                fields: fields.iter().map(|field| field.ty.clone()).collect(),
            }],
        },
        kind: ExprKind::Tuple(fields),
    };
    let boolean = Expr {
        source,
        ty: Term::Primitive(Primitive::Bool),
        kind: ExprKind::Literal(nir::program::Literal::Bool(true)),
    };
    let block = |value| Expr {
        source,
        ty: boolean.ty.clone(),
        kind: ExprKind::Block {
            statements: vec![value],
            result: Some(Box::new(boolean.clone())),
        },
    };
    let staged = BTreeSet::new();
    let protocols = Protocols::default();
    for fields in [
        vec![],
        vec![boolean.clone()],
        vec![tuple(vec![boolean.clone()])],
    ] {
        let body = block(tuple(fields));
        validate_defaults(&package, [(&body, source, &staged)], &protocols).unwrap();
    }
    let good = tuple(vec![boolean.clone()]);
    let mut foreign_domain = good.clone();
    if let Term::NormalForm { arguments, .. } = &mut foreign_domain.ty {
        let Term::NominalValue { ty, .. } = &mut arguments[0] else {
            panic!()
        };
        let Term::NormalForm { head, .. } = ty.as_mut() else {
            panic!()
        };
        *head = id(99);
    }
    package.structs.push(nir::program::Struct {
        declaration: id(9),
        name: "Owner".into(),
        type_parameters: vec![],
        parameter_types: vec![],
        requirements: vec![],
        capabilities: vec![],
        fields: vec![],
    });
    let affine = tuple(vec![Expr {
        source,
        ty: Term::Nominal {
            declaration: id(9),
            arguments: vec![],
        },
        kind: ExprKind::Record {
            declaration: id(9),
            fields: vec![],
        },
    }]);
    for rejected in [foreign_domain, affine] {
        let body = block(rejected);
        assert!(validate_defaults(&package, [(&body, source, &staged)], &protocols).is_err());
    }
    package.normal_forms[0].role = Role::Opaque;
    let body = block(good);
    assert!(validate_defaults(&package, [(&body, source, &staged)], &protocols).is_err());
    package.normal_forms[0].role = Role::Tuple;
    validate_defaults(&package, [(&body, source, &staged)], &protocols).unwrap();
}
