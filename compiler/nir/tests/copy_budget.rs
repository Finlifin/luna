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
    for _ in 0..nir::copy_budget::MAX_COPY_DEPTH + 1 {
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
    for _ in 0..nir::copy_budget::MAX_COPY_DEPTH + 1 {
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

fn function() -> nir::program::Function {
    nir::program::Function {
        phase: nir::program::FunctionPhase::Runtime,
        inline: false,
        pure: false,
        is_unsafe: false,
        preconditions: vec![],
        postconditions: vec![],
        result_binding: None,
        entry_snapshots: vec![],
        capabilities: vec![],
        requirements: vec![],
        type_parameters: vec![],
        declaration: DeclarationId {
            package: 1,
            local: 0,
        },
        name: "example".into(),
        signature: Term::Function {
            parameters: vec![],
            result: Box::new(Term::Primitive(P::Str)),
        },
        parameters: vec![],
        body: small(),
        defaults: vec![],
    }
}

#[test]
fn function_copy_includes_contracts_defaults_and_metadata() {
    use nir::copy_budget::charge_function_substitution as charge;
    for slot in 0..6 {
        let mut value = function();
        match slot {
            0 => value.preconditions.push(large()),
            1 => value.postconditions.push(large()),
            2 => value.defaults.push((0, large())),
            3 => {
                value.signature = Term::Function {
                    parameters: vec![],
                    result: Box::new(Term::Tuple(vec![Term::Primitive(P::Bool); 2048])),
                }
            }
            4 => value.requirements.push(nir::program::TypePredicate::Equal(
                Term::String("x".repeat(2048).into()),
                Term::String("x".repeat(2048).into()),
            )),
            _ => value.name = "x".repeat(2048),
        }
        assert_eq!(
            charge(&value, &[], &[], &mut 128),
            Err(CopyBudgetExhausted::Steps),
            "slot {slot}"
        );
        charge(&value, &[], &[], &mut 20_000).unwrap();
    }
    let mut value = function();
    let mut predicate = nir::program::TypePredicate::Bool(true);
    for _ in 0..nir::copy_budget::MAX_COPY_DEPTH + 1 {
        predicate = nir::program::TypePredicate::Not(Box::new(predicate));
    }
    value.requirements.push(predicate);
    assert_eq!(
        charge(&value, &[], &[], &mut 20_000),
        Err(CopyBudgetExhausted::Depth)
    );
    value.requirements.clear();
    charge(&value, &[], &[], &mut 128).unwrap();
}

#[test]
fn function_copy_counts_substitution_in_type_only_contracts() {
    use nir::copy_budget::charge_function_substitution as charge;
    let input = nir::DeclarationParameter {
        declaration: DeclarationId {
            package: 1,
            local: 1,
        },
        sort: nir::Sort::Type,
    };
    let mut value = function();
    let subject = Term::Rigid {
        declaration: input.declaration,
        sort: input.sort,
    };
    value.capabilities = (0..20)
        .map(|_| nir::program::TraitRequirement {
            interface: DeclarationId {
                package: 1,
                local: 2,
            },
            subject: subject.clone(),
            interface_arguments: vec![subject.clone()],
        })
        .collect();
    let argument = Term::Tuple(vec![Term::Primitive(P::Bool); 128]);
    assert_eq!(
        charge(&value, &[input], &[argument.clone()], &mut 1000),
        Err(CopyBudgetExhausted::Steps)
    );
    charge(&value, &[input], &[argument], &mut 20_000).unwrap();
}

#[test]
fn admitted_copy_depth_is_safe_on_a_small_worker_stack() {
    std::thread::Builder::new()
        .name("copy-depth-boundary".into())
        .stack_size(2 * 1024 * 1024)
        .spawn(|| {
            let mut value = small();
            for index in 0..nir::copy_budget::MAX_COPY_DEPTH - 2 {
                value = expr(match index % 6 {
                    0 => K::Discard(Box::new(value)),
                    1 => K::And(Box::new(value), Box::new(small())),
                    2 => K::CallPreparation {
                        statements: vec![small()],
                        result: Some(Box::new(value)),
                    },
                    3 => K::If {
                        condition: Box::new(small()),
                        then_branch: Box::new(value),
                        else_branch: Some(Box::new(small())),
                    },
                    4 => K::Tuple(vec![small(), value]),
                    _ => K::Return(Some(Box::new(value))),
                });
            }
            nir::copy_budget::charge_type_rewrite(&value, &mut 100_000).unwrap();
            let mut copied = value.clone();
            copied
                .rewrite_types(&mut |ty| Ok::<_, String>(ty.clone()))
                .unwrap();
            drop(copied);
            value = expr(K::Discard(Box::new(value)));
            value = expr(K::Discard(Box::new(value)));
            assert_eq!(
                charge_expression(&value, &mut 100_000),
                Err(CopyBudgetExhausted::Depth)
            );
            charge_expression(&small(), &mut 32).unwrap();
        })
        .unwrap()
        .join()
        .unwrap();
}

#[test]
fn expression_scope_allowance_does_not_raise_type_term_depth() {
    let mut ty = Term::Primitive(P::Bool);
    for _ in 0..129 {
        ty = Term::Pointer(Box::new(ty));
    }
    let mut value = small();
    value.ty = ty;
    assert!(charge_expression(&value, &mut 100_000).is_err());
    charge_expression(&small(), &mut 32).unwrap();
}

#[test]
fn nested_shared_closure_rewrite_uses_a_small_worker_stack() {
    std::thread::Builder::new()
        .name("closure-rewrite-depth".into())
        .stack_size(2 * 1024 * 1024)
        .spawn(|| {
            let mut value = small();
            for index in 0..nir::copy_budget::MAX_COPY_DEPTH - 3 {
                value = expr(K::Closure {
                    parameters: vec![],
                    captures: vec![nir::program::Capture {
                        binding: nir::program::LocalId {
                            owner: 1,
                            index: index as u32,
                        },
                        ty: Term::Primitive(P::Str),
                        value: Some(Box::new(small())),
                    }],
                    body: std::sync::Arc::new(value),
                });
            }
            nir::copy_budget::charge_type_rewrite(&value, &mut 100_000).unwrap();
            let original = value.clone();
            value
                .rewrite_types(&mut |_| Ok::<_, ()>(Term::Primitive(P::Bool)))
                .unwrap();
            let (mut before, mut after) = (&original, &value);
            let mut levels = 0;
            while let (
                K::Closure { body: a, .. },
                K::Closure {
                    body: b, captures, ..
                },
            ) = (&before.kind, &after.kind)
            {
                assert!(!std::sync::Arc::ptr_eq(a, b));
                assert_eq!(a.ty, Term::Primitive(P::Str));
                assert_eq!(b.ty, Term::Primitive(P::Bool));
                assert_eq!(captures[0].ty, Term::Primitive(P::Bool));
                before = a;
                after = b;
                levels += 1;
            }
            assert_eq!(levels, nir::copy_budget::MAX_COPY_DEPTH - 3);
        })
        .unwrap()
        .join()
        .unwrap();
}

#[test]
fn struct_reflection_charges_subject_copy_before_allocation() {
    check(&expr(K::BuildStruct(Box::new(large()))));
    check(&expr(K::StructFieldTypes(Box::new(large()))));
    check(&expr(K::StructFieldNames(Box::new(large()))));
}
