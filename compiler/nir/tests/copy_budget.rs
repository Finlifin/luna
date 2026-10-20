use nir::copy_budget::{CopyBudgetExhausted, charge_expression};
use nir::program::{Expr, ExprKind as K, Literal, MatchArm, Pattern, SourceRange};
use nir::{DeclarationId, Primitive as P, Term};

fn expr(kind: K) -> Expr {
    Expr {
        source: SourceRange { start: 0, end: 1 },
        ty: Term::Primitive(P::Str),
        kind,
    }
}
fn large() -> Expr {
    expr(K::Literal(Literal::String("x".repeat(2048))))
}
fn small() -> Expr {
    expr(K::Literal(Literal::String("ok".into())))
}
fn check(value: &Expr) {
    assert_eq!(
        charge_expression(value, &mut 128),
        Err(CopyBudgetExhausted::Steps)
    );
    charge_expression(value, &mut 20_000).unwrap();
}

#[test]
fn copy_charges_unselected_branches_and_pattern_guards() {
    check(&expr(K::If {
        condition: Box::new(expr(K::Literal(Literal::Bool(true)))),
        then_branch: Box::new(small()),
        else_branch: Some(Box::new(large())),
    }));
    check(&expr(K::Match {
        value: Box::new(small()),
        arms: vec![MatchArm {
            pattern: Pattern::Guard {
                pattern: Box::new(Pattern::Wild),
                condition: Box::new(large()),
            },
            guard: None,
            body: small(),
        }],
    }));
    check(&expr(K::LetPattern {
        pattern: Pattern::Wild,
        value: Box::new(small()),
        guard: None,
        otherwise: Some(Box::new(expr(K::Return(Some(Box::new(large())))))),
    }));
}

#[test]
fn copy_charges_dynamic_bit_selections_and_effect_payloads() {
    check(&expr(K::BitWritePointer {
        pointer: Box::new(small()),
        selections: vec![nir::program::BitSelection::Index(Box::new(large()))],
        value: Box::new(small()),
    }));
    check(&expr(K::EffectComputation {
        effects: vec![nir::effects::Operation {
            declaration: DeclarationId {
                package: 1,
                local: 2,
            },
            arguments: vec![Term::String("x".repeat(2048).into())],
        }],
        body: Box::new(small()),
    }));
    let mut value = small();
    value.ty = Term::Computation {
        effects: vec![nir::effects::Operation {
            declaration: DeclarationId {
                package: 1,
                local: 2,
            },
            arguments: vec![Term::String("x".repeat(2048).into())],
        }],
        result: Box::new(Term::Primitive(P::Str)),
    };
    check(&value);
}

#[test]
fn copy_bounds_width_and_depth_before_retries() {
    let wide = expr(K::Block {
        statements: vec![small(); 512],
        result: None,
    });
    check(&wide);
    let mut deep = small();
    for _ in 0..129 {
        deep = expr(K::Discard(Box::new(deep)));
    }
    assert_eq!(
        charge_expression(&deep, &mut 100_000),
        Err(CopyBudgetExhausted::Depth)
    );
    charge_expression(&small(), &mut 32).unwrap();
}

#[test]
fn copy_shares_closure_bodies_but_charges_capture_initializers() {
    let body = std::sync::Arc::new(large());
    let mut value = expr(K::Closure {
        parameters: vec![],
        captures: vec![],
        body: body.clone(),
    });
    charge_expression(&value, &mut 32).unwrap();
    let cloned = value.clone();
    let K::Closure { body: copied, .. } = &cloned.kind else {
        unreachable!()
    };
    assert!(std::sync::Arc::ptr_eq(&body, copied));
    let K::Closure { captures, .. } = &mut value.kind else {
        unreachable!()
    };
    captures.push(nir::program::Capture {
        binding: nir::program::LocalId { owner: 1, index: 0 },
        ty: Term::Primitive(P::Str),
        value: Some(Box::new(large())),
    });
    check(&value);
}

#[test]
fn type_rewrite_charges_each_shared_closure_body_occurrence() {
    use nir::copy_budget::charge_type_rewrite;
    let body = std::sync::Arc::new(large());
    let closure = || {
        expr(K::Closure {
            parameters: vec![],
            captures: vec![],
            body: body.clone(),
        })
    };
    let value = expr(K::Tuple(vec![closure(), closure()]));
    charge_expression(&value, &mut 64).unwrap();
    assert!(charge_type_rewrite(&value, &mut 3000).is_err());
    charge_type_rewrite(&value, &mut 10_000).unwrap();
    assert_eq!(std::sync::Arc::strong_count(&body), 3);
}

#[test]
fn type_rewrite_bounds_depth_inside_shared_closure_body() {
    use nir::copy_budget::charge_type_rewrite;
    let mut body = small();
    for _ in 0..129 {
        body = expr(K::Discard(Box::new(body)));
    }
    let value = expr(K::Closure {
        parameters: vec![],
        captures: vec![],
        body: std::sync::Arc::new(body),
    });
    charge_expression(&value, &mut 32).unwrap();
    assert_eq!(
        charge_type_rewrite(&value, &mut 100_000),
        Err(CopyBudgetExhausted::Depth)
    );
}

#[test]
fn alias_body_payload_is_charged_before_copying() {
    check(&expr(K::CheckTypeAlias {
        preparations: vec![],
        bindings: None,
        order: None,
        declaration: DeclarationId {
            package: 1,
            local: 20,
        },
        arguments: vec![small()]
            .into_iter()
            .map(nir::program::NormalFormArgument::Value)
            .collect(),
        body: Some(Box::new(large())),
    }));
}

#[test]
fn captured_comptime_values_charge_payloads_and_rewrite_identities() {
    check(&expr(K::ComptimeValue(Term::String(
        "x".repeat(2048).into(),
    ))));
    let id = DeclarationId {
        package: 1,
        local: 42,
    };
    let mut value = expr(K::ComptimeValue(Term::Rigid {
        declaration: id,
        sort: nir::Sort::ComptimeValue,
    }));
    value.ty = Term::Primitive(P::Bool);
    value
        .rewrite_types(&mut |ty| {
            Ok::<_, ()>(
                if *ty
                    == (Term::Rigid {
                        declaration: id,
                        sort: nir::Sort::ComptimeValue,
                    })
                {
                    Term::Bool(true)
                } else {
                    ty.clone()
                },
            )
        })
        .unwrap();
    assert!(matches!(value.kind, K::ComptimeValue(Term::Bool(true))));
    assert!(
        value
            .to_string()
            .contains(&format!("(comptime-value {})", Term::Bool(true)))
    );
}
