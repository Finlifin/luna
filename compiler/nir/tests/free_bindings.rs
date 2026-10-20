use nir::program::{
    CallArgument, Capture, Expr, ExprKind as K, LocalId, MatchArm, Pattern as P, SourceRange,
};
use nir::{Primitive, Term};
fn id(index: u32) -> LocalId {
    LocalId { owner: 1, index }
}
fn expr(kind: K) -> Expr {
    Expr {
        kind,
        source: SourceRange { start: 1, end: 2 },
        ty: Term::Primitive(Primitive::Bool),
    }
}
fn local(index: u32) -> Expr {
    expr(K::Local(id(index)))
}
fn result(expr: &Expr) -> Vec<LocalId> {
    nir::free_bindings::collect(expr, &mut 10_000)
        .unwrap()
        .into_iter()
        .collect()
}
#[test]
fn local_scopes_do_not_hide_outer_or_sibling_reads() {
    let inner = expr(K::Block {
        statements: vec![expr(K::Let {
            binding: id(2),
            value: Box::new(local(1)),
        })],
        result: None,
    });
    let body = expr(K::Block {
        statements: vec![inner],
        result: Some(Box::new(local(2))),
    });
    assert_eq!(result(&body), vec![id(1), id(2)]);
    let body = expr(K::Block {
        statements: vec![expr(K::Let {
            binding: id(2),
            value: Box::new(local(1)),
        })],
        result: Some(Box::new(local(2))),
    });
    assert_eq!(result(&body), vec![id(1)]);
    let body = expr(K::LetPattern {
        pattern: P::Binding {
            binding: id(2),
            inner: Box::new(P::Wild),
        },
        value: Box::new(local(1)),
        guard: Some(Box::new(local(2))),
        otherwise: Some(Box::new(local(2))),
    });
    assert_eq!(result(&body), vec![id(1), id(2)]);
}

#[test]
fn normal_packet_bindings_enter_only_after_their_producer() {
    use nir::program::{NormalFormArgument, NormalFormInputPlan, NormalFormPreparation};
    let preparation = expr(K::LetPattern {
        pattern: P::Tuple(vec![P::Binding {
            binding: id(2),
            inner: Box::new(P::Wild),
        }]),
        value: Box::new(local(1)),
        guard: None,
        otherwise: None,
    });
    let mut value = expr(K::NormalFormType {
        head: nir::DeclarationId {
            package: 1,
            local: 8,
        },
        arguments: vec![NormalFormArgument::Value(local(2))],
        plan: Some(NormalFormInputPlan {
            order: vec![0],
            bindings: vec![id(3)],
            preparations: vec![NormalFormPreparation {
                before: 0,
                expression: preparation,
            }],
        }),
    });
    assert_eq!(result(&value), vec![id(1)]);
    let K::NormalFormType {
        plan: Some(plan), ..
    } = &mut value.kind
    else {
        unreachable!()
    };
    plan.preparations[0].before = 1;
    assert_eq!(result(&value), vec![id(1), id(2)]);
    let K::NormalFormType {
        plan: Some(plan), ..
    } = &mut value.kind
    else {
        unreachable!()
    };
    plan.bindings[0] = id(2);
    assert!(nir::free_bindings::collect(&value, &mut 1000).is_err());
}
#[test]
fn closure_initializers_and_body_have_separate_scopes() {
    let closure = expr(K::Closure {
        parameters: vec![id(3)],
        captures: vec![Capture {
            binding: id(2),
            ty: Term::Primitive(Primitive::Bool),
            value: Some(Box::new(local(1))),
        }],
        body: std::sync::Arc::new(expr(K::And(Box::new(local(2)), Box::new(local(3))))),
    });
    assert_eq!(result(&closure), vec![id(1)]);
    let mut implicit = closure.clone();
    let K::Closure { captures, .. } = &mut implicit.kind else {
        unreachable!()
    };
    captures[0].value = None;
    assert_eq!(result(&implicit), vec![id(2)]);
    let K::Closure { body, .. } = &mut implicit.kind else {
        unreachable!()
    };
    *body = std::sync::Arc::new(local(99));
    assert!(
        nir::free_bindings::collect(&implicit, &mut 100)
            .unwrap_err()
            .contains("undeclared capture")
    );
}
#[test]
fn explicit_call_inputs_never_use_the_defaults_parameter_frame() {
    let mut call = expr(K::Call {
        callee: Box::new(expr(K::Function(nir::DeclarationId {
            package: 1,
            local: 2,
        }))),
        arguments: vec![
            CallArgument {
                parameter: 0,
                value: local(1),
                is_default: false,
            },
            CallArgument {
                parameter: 1,
                value: local(1),
                is_default: true,
            },
        ],
        parameter_bindings: vec![id(1), id(2)],
    });
    assert_eq!(result(&call), vec![id(1)]);
    let K::Call { arguments, .. } = &mut call.kind else {
        unreachable!()
    };
    arguments[1].value = local(3);
    assert!(
        nir::free_bindings::collect(&call, &mut 100)
            .unwrap_err()
            .contains("unavailable parameter")
    );
}
#[test]
fn pattern_producers_precede_guards_and_alternatives_remain_isolated() {
    let pattern = P::Then {
        first: Box::new(P::Binding {
            binding: id(2),
            inner: Box::new(P::Wild),
        }),
        value: Box::new(local(2)),
        next: Box::new(P::Binding {
            binding: id(3),
            inner: Box::new(P::Wild),
        }),
    };
    let body = expr(K::Match {
        value: Box::new(local(1)),
        arms: vec![
            MatchArm {
                pattern,
                guard: Some(local(4)),
                body: local(3),
            },
            MatchArm {
                pattern: P::Wild,
                guard: None,
                body: local(2),
            },
        ],
    });
    assert_eq!(result(&body), vec![id(1), id(2), id(4)]);
    assert!(nir::free_bindings::collect(&body, &mut 0).is_err());
}

#[test]
fn normal_input_plans_bind_values_in_their_recorded_order() {
    use nir::program::{NormalFormArgument as A, NormalFormInputPlan};
    let mut value = expr(K::NormalFormType {
        head: nir::DeclarationId {
            package: 1,
            local: 8,
        },
        arguments: vec![A::Value(local(1)), A::Value(local(2))],
        plan: Some(NormalFormInputPlan {
            preparations: vec![],
            bindings: vec![id(2), id(3)],
            order: vec![0, 1],
        }),
    });
    assert_eq!(result(&value), vec![id(1)]);
    let K::NormalFormType { plan, .. } = &mut value.kind else {
        unreachable!()
    };
    plan.as_mut().unwrap().order = vec![1, 0];
    assert_eq!(result(&value), vec![id(1), id(2)]);
    let K::NormalFormType { plan, .. } = &mut value.kind else {
        unreachable!()
    };
    plan.as_mut().unwrap().order = vec![0, 0];
    assert!(nir::free_bindings::collect(&value, &mut 100).is_err());
}

#[test]
fn pattern_remaps_publish_common_outputs_without_leaking_arm_locals() {
    let alternative = |binding| P::Remap {
        pattern: Box::new(P::Binding {
            binding: id(binding),
            inner: Box::new(P::Wild),
        }),
        bindings: vec![(id(binding), id(5))],
    };
    let value = expr(K::Match {
        value: Box::new(local(1)),
        arms: vec![MatchArm {
            pattern: P::Or(vec![alternative(2), alternative(3)]),
            guard: None,
            body: expr(K::And(Box::new(local(5)), Box::new(local(2)))),
        }],
    });
    assert_eq!(result(&value), vec![id(1), id(2)]);
}

#[test]
fn observation_aliases_bind_only_inside_the_pattern_and_visit_computations() {
    let pattern = P::Observe {
        binding: id(2),
        inner: Box::new(P::Then {
            first: Box::new(P::Wild),
            value: Box::new(local(2)),
            next: Box::new(P::Binding {
                binding: id(3),
                inner: Box::new(P::Wild),
            }),
        }),
    };
    let body = expr(K::Match {
        value: Box::new(local(1)),
        arms: vec![MatchArm {
            pattern: pattern.clone(),
            guard: None,
            body: expr(K::And(Box::new(local(2)), Box::new(local(3)))),
        }],
    });
    assert_eq!(result(&body), vec![id(1), id(2)]);
    assert_eq!(
        pattern.to_string(),
        "(observe-pattern (subject (local 1 2)) (inner (and-is (first (wild)) (value (expr (type (primitive bool)) (source 1 2) (local 1 2))) (next (binding (local 1 3) (wild))))))"
    );
    let mut references = vec![];
    nir::visit::expression(&body, &mut 1000, |expr, _| {
        if let K::Local(binding) = expr.kind {
            references.push(binding);
        }
        Ok(())
    })
    .unwrap();
    assert_eq!(
        references
            .iter()
            .filter(|binding| **binding == id(2))
            .count(),
        2
    );
}

#[test]
fn nominal_default_bindings_are_local_to_one_constructor() {
    use nir::program::NormalFormArgument as A;
    let nominal = expr(K::NominalType {
        preparations: vec![],
        declaration: nir::DeclarationId {
            package: 1,
            local: 8,
        },
        arguments: vec![A::Value(local(1)), A::Value(local(2))],
        order: Some(vec![0, 1]),
        bindings: Some(vec![id(2), id(3)]),
    });
    assert_eq!(result(&nominal), vec![id(1)]);
    let body = expr(K::Block {
        statements: vec![nominal.clone()],
        result: Some(Box::new(local(2))),
    });
    assert_eq!(result(&body), vec![id(1), id(2)]);
    let mut malformed = nominal;
    let K::NominalType { bindings, .. } = &mut malformed.kind else {
        unreachable!()
    };
    *bindings = Some(vec![id(2), id(2)]);
    assert!(nir::free_bindings::collect(&malformed, &mut 10_000).is_err());
}

#[test]
fn nominal_packet_bindings_exist_only_after_their_producer() {
    use nir::program::{NormalFormArgument as A, NormalFormPreparation};
    let mut value = expr(K::NominalType {
        declaration: nir::DeclarationId {
            package: 1,
            local: 8,
        },
        arguments: vec![A::Value(local(2))],
        order: None,
        bindings: None,
        preparations: vec![NormalFormPreparation {
            before: 0,
            expression: expr(K::LetPattern {
                pattern: P::Tuple(vec![P::Binding {
                    binding: id(2),
                    inner: Box::new(P::Wild),
                }]),
                value: Box::new(local(1)),
                guard: None,
                otherwise: None,
            }),
        }],
    });
    assert_eq!(result(&value), vec![id(1)]);
    assert!(value.to_string().contains("(prepare (before 0)"));
    let K::NominalType { preparations, .. } = &mut value.kind else {
        unreachable!()
    };
    preparations[0].before = 1;
    assert_eq!(result(&value), vec![id(1), id(2)]);
    let K::NominalType { preparations, .. } = &mut value.kind else {
        unreachable!()
    };
    preparations[0].before = 2;
    assert!(nir::free_bindings::collect(&value, &mut 10_000).is_err());
}

#[test]
fn alias_body_is_visible_to_free_binding_analysis_and_rewrites() {
    let mut value = expr(K::CheckTypeAlias {
        preparations: vec![],
        bindings: None,
        order: None,
        declaration: nir::DeclarationId {
            package: 1,
            local: 20,
        },
        arguments: vec![local(1)]
            .into_iter()
            .map(nir::program::NormalFormArgument::Value)
            .collect(),
        body: Some(Box::new(local(2))),
    });
    assert_eq!(result(&value), vec![id(1), id(2)]);
    value
        .rewrite_types(&mut |ty| {
            Ok::<_, ()>(if *ty == Term::Primitive(Primitive::Bool) {
                Term::Primitive(Primitive::Char)
            } else {
                ty.clone()
            })
        })
        .unwrap();
    let K::CheckTypeAlias {
        body: Some(body), ..
    } = &value.kind
    else {
        panic!()
    };
    assert_eq!(body.ty, Term::Primitive(Primitive::Char));
    assert!(value.to_string().contains("(body (expr"));
}

#[test]
fn contextual_alias_input_bindings_are_local_to_its_body() {
    let value = expr(K::CheckTypeAlias {
        declaration: nir::DeclarationId {
            package: 1,
            local: 20,
        },
        preparations: vec![],
        bindings: Some(vec![id(2)]),
        order: Some(vec![0]),
        arguments: vec![nir::program::NormalFormArgument::Value(local(1))],
        body: Some(Box::new(local(2))),
    });
    assert_eq!(result(&value), vec![id(1)]);
    let sibling = expr(K::Tuple(vec![value, local(2)]));
    assert_eq!(result(&sibling), vec![id(1), id(2)]);
}

#[test]
fn pattern_code_evidence_binds_only_declared_success_outputs() {
    let arm = MatchArm {
        pattern: P::CodeEvidence {
            outputs: vec![(id(1), Term::Primitive(Primitive::Bool))],
        },
        guard: Some(local(1)),
        body: local(2),
    };
    let free = nir::free_bindings::collect_arm(&arm, &mut 100).unwrap();
    assert_eq!(free.into_iter().collect::<Vec<_>>(), vec![id(2)]);
    assert!(nir::free_bindings::collect_arm(&arm, &mut 0).is_err());
    let dump = arm.pattern.to_string();
    assert!(
        dump.starts_with("(pattern-code-evidence (outputs (output "),
        "{dump}"
    );
}
