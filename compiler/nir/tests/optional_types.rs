use nir::{DeclarationId, Parameter, Primitive, RuntimeId, Sort, Term, TypeInterner, TypeRef};
use std::sync::Arc;

fn optional(inner: Term) -> Term {
    Term::Optional(Box::new(inner))
}

#[test]
fn optional_layers_are_distinct_and_substitution_preserves_opaque_heads() {
    let boolean = Term::Primitive(Primitive::Bool);
    let binder = Term::Bound {
        depth: 0,
        index: 0,
        sort: Sort::Type,
    };
    let nested = optional(optional(binder));
    assert!(nested.check_scope(&[vec![Sort::Type]]).is_ok());
    assert!(nested.check_scope(&[]).is_err());
    assert_eq!(
        nested.substitute(&[boolean.clone()]).unwrap(),
        optional(optional(boolean.clone()))
    );
    assert_ne!(
        optional(boolean.clone()),
        optional(optional(boolean.clone()))
    );
    assert_ne!(
        optional(boolean.clone()),
        Term::Pointer(Box::new(boolean.clone()))
    );
    assert_eq!(
        optional(optional(boolean.clone())).to_string(),
        "(optional (optional (primitive bool)))"
    );
    let opaque = Term::NormalForm {
        head: DeclarationId {
            package: 17,
            local: 9,
        },
        sort: Sort::Type,
        arguments: vec![boolean],
    };
    assert_eq!(
        nested.substitute(&[opaque.clone()]).unwrap(),
        optional(optional(opaque))
    );
    assert!(optional(Term::Bool(true)).check_scope(&[]).is_err());
}

#[test]
fn optional_core_sharing_preserves_runtime_identity_and_payload_validation() {
    let mut interner = TypeInterner::default();
    let parameters = vec![Parameter {
        sort: Sort::Runtime,
        ty: Term::Primitive(Primitive::Bool),
    }];
    let body = optional(Term::NormalForm {
        head: DeclarationId {
            package: 17,
            local: 4,
        },
        sort: Sort::Type,
        arguments: vec![Term::Hole {
            index: 0,
            sort: Sort::Runtime,
        }],
    });
    let first = interner.intern(parameters.clone(), body.clone()).unwrap();
    let second = interner.intern(parameters, body).unwrap();
    assert!(Arc::ptr_eq(&first, &second));
    let a = TypeRef::bind(
        first,
        vec![Term::Runtime(RuntimeId {
            instance: 1,
            local: 0,
        })],
    )
    .unwrap();
    let b = TypeRef::bind(
        second,
        vec![Term::Runtime(RuntimeId {
            instance: 2,
            local: 0,
        })],
    )
    .unwrap();
    assert_ne!(a, b);
    assert_ne!(a.materialize().unwrap(), b.materialize().unwrap());
    assert!(
        interner
            .intern(
                vec![],
                optional(Term::Hole {
                    index: 0,
                    sort: Sort::Type
                })
            )
            .is_err()
    );
}

#[test]
fn optional_constructor_keeps_operand_visible_to_binding_analysis_and_budgeting() {
    use nir::program::{Expr, ExprKind as K, LocalId, SourceRange};
    let source = SourceRange { start: 0, end: 1 };
    let old = LocalId { owner: 1, index: 0 };
    let new = LocalId { owner: 2, index: 0 };
    let ty = Term::Primitive(Primitive::Type);
    let operand = Expr {
        source,
        ty: ty.clone(),
        kind: K::Local(old),
    };
    let mut construction = Expr {
        source,
        ty,
        kind: K::OptionalType(Box::new(operand.clone())),
    };
    assert_eq!(
        nir::free_bindings::collect(&construction, &mut 100).unwrap(),
        std::collections::BTreeSet::from([old])
    );
    let mut operand_fuel = 100;
    nir::copy_budget::charge_expression(&operand, &mut operand_fuel).unwrap();
    let mut constructor_fuel = 100;
    nir::copy_budget::charge_expression(&construction, &mut constructor_fuel).unwrap();
    assert!(constructor_fuel < operand_fuel);
    assert!(nir::copy_budget::charge_expression(&construction, &mut 1).is_err());
    assert!(nir::completed_values::validate(&construction, &mut 100).is_err());
    nir::contracts::remap_predicate(
        &mut construction,
        &std::collections::BTreeMap::from([(old, new)]),
    )
    .unwrap();
    assert_eq!(
        nir::free_bindings::collect(&construction, &mut 100).unwrap(),
        std::collections::BTreeSet::from([new])
    );
}

#[test]
fn completed_optional_values_validate_each_layer_and_rewrite_payload_types() {
    use nir::program::{Expr, ExprKind as K, Literal, SourceRange};
    let source = SourceRange { start: 1, end: 2 };
    let boolean = Term::Primitive(Primitive::Bool);
    let absent = Expr {
        source,
        ty: optional(boolean.clone()),
        kind: K::OptionalAbsent,
    };
    let nested = Expr {
        source,
        ty: optional(absent.ty.clone()),
        kind: K::OptionalPresent(Box::new(absent.clone())),
    };
    nir::completed_values::validate(&nested, &mut 100).unwrap();
    assert!(nested.to_string().contains("(optional-present"));
    let wrong_layer = Expr {
        ty: optional(boolean.clone()),
        ..nested.clone()
    };
    assert!(nir::completed_values::validate(&wrong_layer, &mut 100).is_err());
    let missing_optional = Expr {
        ty: boolean.clone(),
        ..absent
    };
    assert!(nir::completed_values::validate(&missing_optional, &mut 100).is_err());
    let wrong_payload = Expr {
        source,
        ty: optional(boolean.clone()),
        kind: K::OptionalPresent(Box::new(Expr {
            source,
            ty: boolean,
            kind: K::Literal(Literal::Char('x')),
        })),
    };
    assert!(nir::completed_values::validate(&wrong_payload, &mut 100).is_err());
    let parameter = DeclarationId {
        package: 1,
        local: 99,
    };
    let inner = Term::Rigid {
        declaration: parameter,
        sort: Sort::Type,
    };
    let template = Expr {
        source,
        ty: optional(optional(inner.clone())),
        kind: K::OptionalPresent(Box::new(Expr {
            source,
            ty: optional(inner),
            kind: K::OptionalAbsent,
        })),
    };
    let specialized = template
        .instantiate_types(&[parameter], &[Term::Primitive(Primitive::Char)])
        .unwrap();
    nir::completed_values::validate(&specialized, &mut 100).unwrap();
    assert!(template.to_string().contains("(rigid"));
    assert!(!specialized.to_string().contains("(rigid"));
}

#[test]
fn frozen_optional_indices_check_payload_domains_scope_and_keep_layer_identity() {
    let boolean = Term::Primitive(Primitive::Bool);
    let ty = optional(boolean.clone());
    let id = DeclarationId {
        package: 1,
        local: 1,
    };
    let absent = Term::OptionalValue {
        ty: Box::new(ty.clone()),
        value: None,
    };
    let present = Term::OptionalValue {
        ty: Box::new(ty.clone()),
        value: Some(Box::new(Term::Bool(true))),
    };
    for value in [&absent, &present] {
        value.check_scope(&[]).unwrap();
        nir::validate_declaration_arguments(&[id], &[ty.clone()], &[value.clone()], &mut 1000)
            .unwrap();
    }
    assert_eq!(
        absent.to_string(),
        "(optional-value (type (optional (primitive bool))) (absent))"
    );
    assert_ne!(absent, present);
    for wrong in [
        Term::OptionalValue {
            ty: Box::new(ty.clone()),
            value: Some(Box::new(Term::Char('x'))),
        },
        Term::OptionalValue {
            ty: Box::new(optional(ty.clone())),
            value: None,
        },
    ] {
        assert!(
            nir::validate_declaration_arguments(&[id], &[ty.clone()], &[wrong], &mut 1000).is_err()
        );
    }
    let abstract_ty = Term::Rigid {
        declaration: id,
        sort: Sort::Type,
    };
    let template = Term::OptionalValue {
        ty: Box::new(optional(Term::Primitive(Primitive::Type))),
        value: Some(Box::new(abstract_ty)),
    };
    assert_eq!(
        template
            .instantiate_types(&[id], &[boolean.clone()])
            .unwrap(),
        Term::OptionalValue {
            ty: Box::new(optional(Term::Primitive(Primitive::Type))),
            value: Some(Box::new(boolean))
        }
    );
    let runtime = Term::OptionalValue {
        ty: Box::new(ty),
        value: Some(Box::new(Term::Runtime(RuntimeId {
            instance: 1,
            local: 0,
        }))),
    };
    assert!(runtime.check_scope(&[]).is_err());
    assert!(nir::type_computations::charge_term(&present, &mut 1).is_err());
}

#[test]
fn optional_pattern_observations_preserve_binding_scope_and_type_rewriting() {
    use nir::program::{Expr, ExprKind as K, LocalId, Pattern, SourceRange};
    let source = SourceRange { start: 1, end: 2 };
    let bound = LocalId { owner: 1, index: 0 };
    let free = LocalId { owner: 1, index: 1 };
    let parameter = DeclarationId {
        package: 1,
        local: 99,
    };
    let unknown = Term::Rigid {
        declaration: parameter,
        sort: Sort::Type,
    };
    let mut pattern = Pattern::OptionalPresent(Box::new(Pattern::Guard {
        pattern: Box::new(Pattern::Binding {
            binding: bound,
            inner: Box::new(Pattern::Wild),
        }),
        condition: Box::new(Expr {
            source,
            ty: Term::Primitive(Primitive::Bool),
            kind: K::TypeEqual(
                Box::new(Expr {
                    source,
                    ty: unknown.clone(),
                    kind: K::Local(bound),
                }),
                Box::new(Expr {
                    source,
                    ty: unknown,
                    kind: K::Local(free),
                }),
            ),
        }),
    }));
    assert_eq!(
        nir::free_bindings::collect_pattern(&pattern, &mut 1000).unwrap(),
        std::collections::BTreeSet::from([free])
    );
    pattern
        .rewrite_types(&mut |ty| {
            ty.instantiate_types(&[parameter], &[Term::Primitive(Primitive::Type)])
        })
        .unwrap();
    assert!(!pattern.to_string().contains("(rigid"));
    let observations = nir::visit::pattern_observations(&pattern, &mut 100).unwrap();
    assert!(!observations.is_empty());
    assert!(nir::copy_budget::charge_pattern_rewrite(&pattern, &mut 1).is_err());
    nir::copy_budget::charge_pattern_rewrite(&pattern, &mut 100).unwrap();
    assert!(pattern.to_string().contains("(optional-present-pattern"));
}
