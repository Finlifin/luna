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
    for preparation in [
        expr(K::Let {
            binding: id(2),
            value: Box::new(local(1)),
        }),
        expr(K::LetPattern {
            pattern: P::Tuple(vec![P::Binding {
                binding: id(2),
                inner: Box::new(P::Wild),
            }]),
            value: Box::new(local(1)),
            guard: None,
            otherwise: None,
        }),
    ] {
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
    assert_eq!(
        nir::free_bindings::collect(&implicit, &mut 100),
        Err(nir::free_bindings::Error::Invalid(
            "closure body references an undeclared capture"
        )),
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
                passing: Default::default(),
                parameter: 0,
                value: local(1),
                is_default: false,
            },
            CallArgument {
                passing: Default::default(),
                parameter: 1,
                value: local(1),
                is_default: true,
            },
        ],
        parameter_bindings: vec![id(1), id(2)],
        env_arguments: vec![],
    });
    assert_eq!(result(&call), vec![id(1)]);
    let K::Call { arguments, .. } = &mut call.kind else {
        unreachable!()
    };
    arguments[1].value = local(3);
    assert_eq!(
        nir::free_bindings::collect(&call, &mut 100),
        Err(nir::free_bindings::Error::Invalid(
            "call default references an unavailable parameter"
        )),
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
fn alternatives_intersect_net_changes_to_bindings_that_were_already_in_scope() {
    // Only one alternative translates the outer binding 2 into 3. Neither
    // name is guaranteed on success. Both alternatives produce binding 4.
    let bind = |index| P::Binding {
        binding: id(index),
        inner: Box::new(P::Wild),
    };
    let alternatives = P::Or(vec![
        P::Remap {
            pattern: Box::new(bind(4)),
            bindings: vec![(id(2), id(3))],
        },
        bind(4),
    ]);
    let guard = expr(K::Tuple(vec![local(2), local(3), local(4)]));
    let inner = expr(K::Match {
        value: Box::new(local(2)),
        arms: vec![MatchArm {
            pattern: alternatives,
            guard: Some(guard),
            body: local(4),
        }],
    });
    let body = expr(K::Block {
        statements: vec![
            expr(K::Let {
                binding: id(2),
                value: Box::new(local(0)),
            }),
            inner,
        ],
        result: Some(Box::new(local(2))),
    });
    assert_eq!(result(&body), [id(0), id(2), id(3)]);
}

#[test]
fn nested_scope_work_tracks_its_own_edits_independently_of_the_outer_prefix() {
    let cost = |width, levels| {
        let mut tail = local(0);
        for index in 0..levels {
            tail = expr(K::Block {
                statements: vec![expr(K::Let {
                    binding: id(width + index),
                    value: Box::new(local(0)),
                })],
                result: Some(Box::new(tail)),
            });
        }
        let body = expr(K::Block {
            statements: (0..width)
                .map(|index| {
                    expr(K::Let {
                        binding: id(index),
                        value: Box::new(expr(K::Unit)),
                    })
                })
                .collect(),
            result: Some(Box::new(tail)),
        });
        let mut fuel = 100_000;
        assert!(
            nir::free_bindings::collect(&body, &mut fuel)
                .unwrap()
                .is_empty()
        );
        100_000 - fuel
    };
    assert_eq!(cost(1, 32) - cost(1, 0), cost(1024, 32) - cost(1024, 0));
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

#[test]
fn symbolic_code_reads_respect_guard_scope_and_keep_external_dependencies() {
    use std::collections::{BTreeMap, BTreeSet};
    let dependencies = BTreeMap::from([(id(9), BTreeSet::from([id(1), id(2)]))]);
    let arm = MatchArm {
        pattern: P::Binding {
            binding: id(1),
            inner: Box::new(P::Wild),
        },
        guard: Some(local(9)),
        body: local(1),
    };
    let free =
        nir::free_bindings::collect_arm_with_dependencies(&arm, &dependencies, &mut 1000).unwrap();
    assert_eq!(free, BTreeSet::from([id(2)]));
    let value = expr(K::Tuple(vec![
        expr(K::Match {
            value: Box::new(local(3)),
            arms: vec![arm],
        }),
        local(9),
    ]));
    let free =
        nir::free_bindings::collect_with_dependencies(&value, &dependencies, &mut 1000).unwrap();
    assert_eq!(free, BTreeSet::from([id(1), id(2), id(3)]));
    assert!(nir::free_bindings::collect_with_dependencies(&value, &dependencies, &mut 0).is_err());
    let cycle = BTreeMap::from([
        (id(9), BTreeSet::from([id(8)])),
        (id(8), BTreeSet::from([id(9)])),
    ]);
    assert!(nir::free_bindings::collect_with_dependencies(&local(9), &cycle, &mut 1000).is_err());
}

#[test]
fn symbolic_diamonds_expand_each_edge_once_per_read_and_stop_at_lexical_binders() {
    use nir::free_bindings::{Error, collect_with_dependencies};
    use std::collections::{BTreeMap, BTreeSet};
    let levels = 1024;
    let leaf = id(levels * 2);
    let mut graph = BTreeMap::new();
    for level in 0..levels {
        let next = id(2 * level + 2);
        let edges = if next == leaf {
            BTreeSet::from([leaf])
        } else {
            BTreeSet::from([next, id(2 * level + 3)])
        };
        graph.insert(id(2 * level), edges.clone());
        graph.insert(id(2 * level + 1), edges);
    }
    let check = |root: &Expr, graph: &BTreeMap<_, _>, fuel: &mut usize| {
        collect_with_dependencies(root, graph, fuel)
    };
    assert!(matches!(
        check(&local(0), &graph, &mut 100),
        Err(Error::Budget(_))
    ));
    assert_eq!(check(&local(0), &graph, &mut 5000).unwrap(), [leaf].into());
    graph.insert(leaf, [id(0)].into());
    assert_eq!(
        check(&local(0), &graph, &mut 5000),
        Err(Error::Invalid("cyclic symbolic capture dependency"))
    );
    let masked = expr(K::Block {
        statements: vec![expr(K::Let {
            binding: leaf,
            value: Box::new(local(9000)),
        })],
        result: Some(Box::new(local(0))),
    });
    assert_eq!(
        check(&masked, &graph, &mut 5000).unwrap(),
        [id(9000)].into()
    );
}

#[test]
fn binding_failures_distinguish_budget_cycles_and_invalid_scope() {
    use nir::free_bindings::Error;
    use std::collections::{BTreeMap, BTreeSet};
    let value = local(1);
    let mut fuel = 1;
    assert!(matches!(
        nir::free_bindings::collect(&value, &mut fuel),
        Err(Error::Budget(_))
    ));
    assert_eq!(fuel, 0);
    assert_eq!(result(&value), vec![id(1)]);

    let cycle = BTreeMap::from([(id(1), BTreeSet::from([id(1)]))]);
    let mut fuel = 1000;
    assert!(matches!(
        nir::free_bindings::collect_with_dependencies(&value, &cycle, &mut fuel),
        Err(Error::Invalid("cyclic symbolic capture dependency"))
    ));
    assert!(fuel > 0 && fuel < 1000);
    assert_eq!(result(&value), vec![id(1)]);

    let invalid = expr(K::Closure {
        captures: vec![],
        parameters: vec![],
        body: std::sync::Arc::new(value),
    });
    assert_eq!(
        nir::free_bindings::collect(&invalid, &mut 100),
        Err(Error::Invalid(
            "closure body references an undeclared capture"
        ))
    );
}

#[test]
fn lexical_work_uses_one_budget_across_deep_expressions_patterns_and_operands() {
    use nir::free_bindings::{Error, collect};
    use nir::program::NormalFormArgument as A;

    // The worker only borrows the trees. Their construction and Rust drop glue
    // have a different stack contract from the lexical analysis under test.
    let source = local(0).source;
    let mut condition = local(1);
    let mut operand = A::Value(local(4));
    let mut pattern = P::Binding {
        binding: id(2),
        inner: Box::new(P::Binding {
            binding: id(4),
            inner: Box::new(P::Wild),
        }),
    };
    for _ in 0..384 {
        condition = expr(K::Block {
            statements: vec![],
            result: Some(Box::new(condition)),
        });
        pattern = P::OptionalPresent(Box::new(pattern));
        operand = A::Block {
            source,
            statements: vec![],
            result: Some(Box::new(operand)),
        };
    }
    // The condition reads an outer local. Its pattern binding is visible
    // through the entire contextual operand, but is gone in the sibling arm.
    let head = nir::DeclarationId {
        package: 1,
        local: 8,
    };
    let value = expr(K::Match {
        value: Box::new(local(3)),
        arms: vec![
            MatchArm {
                pattern,
                guard: Some(condition),
                body: expr(K::NormalFormType {
                    head,
                    arguments: vec![operand],
                    plan: None,
                }),
            },
            MatchArm {
                pattern: P::Wild,
                guard: None,
                body: local(2),
            },
        ],
    });
    let invalid = expr(K::Closure {
        captures: vec![],
        parameters: vec![],
        body: std::sync::Arc::new(value),
    });
    let K::Closure { body, .. } = &invalid.kind else {
        unreachable!()
    };
    std::thread::scope(|scope| {
        std::thread::Builder::new()
            .stack_size(256 * 1024)
            .spawn_scoped(scope, || {
                let expected = [id(1), id(2), id(3)].into_iter().collect();
                assert!(matches!(collect(body, &mut 300), Err(Error::Budget(_))));
                assert_eq!(collect(body, &mut 100_000).unwrap(), expected);
                assert_eq!(
                    collect(&invalid, &mut 100_000),
                    Err(Error::Invalid(
                        "closure body references an undeclared capture"
                    ))
                );
                assert_eq!(collect(body, &mut 100_000).unwrap(), expected);
            })
            .unwrap()
            .join()
            .unwrap();
    });
}

#[test]
fn typed_traversal_preserves_callback_errors_and_structural_failures() {
    use nir::visit::{Error, expression_checked};
    let unit = expr(K::Unit);
    // Diagnostic text cannot change a callback's explicit error category.
    let callback_error = Error::Invalid("budget exhausted is merely text here");
    assert_eq!(
        expression_checked(&unit, &mut 100, |_, _| Err(callback_error)),
        Err(callback_error)
    );
    assert!(matches!(
        expression_checked(&unit, &mut 0, |_, _| Ok::<_, Error>(())),
        Err(Error::Budget(_))
    ));
    let mut value = expr(K::LetPattern {
        pattern: P::AnonymousRecord {
            names: vec!["x".repeat(100)],
            fields: vec![P::Wild],
        },
        value: Box::new(unit),
        guard: None,
        otherwise: None,
    });
    let mut fuel = 10;
    assert!(matches!(
        expression_checked(&value, &mut fuel, |_, _| Ok::<_, Error>(())),
        Err(Error::Budget(_))
    ));
    assert!(fuel > 0 && fuel < 10);
    expression_checked(&value, &mut 1000, |_, _| Ok::<_, Error>(())).unwrap();
    let K::LetPattern {
        pattern: P::AnonymousRecord { names, .. },
        ..
    } = &mut value.kind
    else {
        unreachable!()
    };
    names.clear();
    assert_eq!(
        expression_checked(&value, &mut 1000, |_, _| Ok::<_, Error>(())),
        Err(Error::Invalid("anonymous record pattern arity mismatch"))
    );
}

#[test]
fn clause_operands_read_preparation_locals_without_exporting_them() {
    let instance = expr(K::ComptimeFunctionInstance {
        declaration: nir::DeclarationId {
            package: 1,
            local: 9,
        },
        inputs: nir::program::ClauseOperands {
            observations: Default::default(),
            statements: vec![expr(K::Let {
                binding: id(2),
                value: Box::new(local(1)),
            })],
            arguments: vec![local(2), local(3)],
        },
    });
    assert_eq!(result(&instance), vec![id(1), id(3)]);
    let caller = expr(K::Block {
        statements: vec![instance],
        result: Some(Box::new(local(2))),
    });
    assert_eq!(result(&caller), vec![id(1), id(2), id(3)]);
}

#[test]
fn a_contextual_pattern_guard_invalidates_only_the_input_it_writes() {
    use nir::program::{NormalFormArgument, NormalMatchArm, Place};
    use std::collections::BTreeMap;

    let guard = expr(K::Block {
        statements: vec![expr(K::Assign {
            place: Place {
                root: id(1),
                fields: vec![],
            },
            value: Box::new(local(2)),
        })],
        result: Some(Box::new(local(2))),
    });
    let arg = NormalFormArgument::Match {
        source: guard.source,
        value: local(2),
        arms: vec![NormalMatchArm {
            pattern: P::Guard {
                pattern: Box::new(P::Wild),
                condition: Box::new(guard),
            },
            guard: None,
            body: NormalFormArgument::Value(local(2)),
        }],
    };
    let mut live = BTreeMap::from([(id(1), ()), (id(2), ())]);
    nir::free_bindings::retain_unmodified_argument(&arg, &mut live, &mut 128).unwrap();
    assert_eq!(live, BTreeMap::from([(id(2), ())]));
}
