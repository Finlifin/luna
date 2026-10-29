use nir::{Primitive as P, Sort, Term};
use type_resolution::{
    InferenceTerm as I,
    matching::MatchError,
    obligations::{Engine, Error, State},
    ptc::{Budget, Outcome},
};
fn stable(p: P) -> I {
    I::Stable(Term::Primitive(p))
}
fn slot(index: usize) -> Term {
    Term::Bound {
        depth: 0,
        index,
        sort: Sort::Type,
    }
}
fn finish(engine: &mut Engine) {
    engine.advance(100, Budget::default());
}

#[test]
fn tuple_pack_inference_preserves_head_proofs_and_transaction_isolation() {
    use nir::normal_forms::{Head, Kind, Parameter, Role};
    let head = nir::DeclarationId {
        package: 55,
        local: 1,
    };
    let tuple = |input| Term::NormalForm {
        head,
        sort: Sort::Type,
        arguments: vec![input],
    };
    let package = nir::program::Package {
        normal_forms: vec![
            Head {
                declaration: head,
                role: Role::Tuple,
                requirements: vec![],
                parameters: vec![Parameter {
                    declaration: nir::DeclarationId {
                        package: 55,
                        local: 2,
                    },
                    binding: nir::program::LocalId { owner: 1, index: 0 },
                    kind: Kind::Value,
                    ty: Term::NormalForm {
                        head: nir::DeclarationId {
                            package: 55,
                            local: 3,
                        },
                        sort: nir::Sort::Type,
                        arguments: vec![Term::Primitive(P::Type)],
                    },
                }],
            },
            Head {
                declaration: nir::DeclarationId {
                    package: 55,
                    local: 3,
                },
                role: nir::normal_forms::Role::TypeList,
                requirements: vec![],
                parameters: vec![Parameter {
                    declaration: nir::DeclarationId {
                        package: 55,
                        local: 4,
                    },
                    binding: nir::program::LocalId { owner: 3, index: 0 },
                    kind: nir::normal_forms::Kind::Type,
                    ty: Term::Primitive(nir::Primitive::Type),
                }],
            },
        ],
        ..Default::default()
    };
    let context = type_resolution::ptc::Context::from_package(&package).unwrap();
    for fields in [
        vec![],
        vec![Term::Primitive(P::Bool)],
        vec![Term::Primitive(P::Bool), tuple(Term::TypeListValue(vec![]))],
    ] {
        let mut engine = Engine::with_context(context.fork());
        let pack = engine.fresh(Sort::ComptimeValue);
        let template = I::Instantiate {
            template: tuple(Term::Bound {
                depth: 0,
                index: 0,
                sort: Sort::ComptimeValue,
            }),
            arguments: vec![I::Meta(pack)],
        };
        let input = Term::TypeListValue(fields);
        let (mut candidate, id) = engine
            .match_types(
                template.clone(),
                I::Stable(tuple(input.clone())),
                Budget::default(),
            )
            .unwrap();
        finish(&mut candidate);
        assert!(candidate.state(id).unwrap().is_proven());
        assert!(engine.freeze(&I::Meta(pack)).is_err());
        engine.commit(candidate).unwrap();
        assert_eq!(engine.freeze(&I::Meta(pack)), Ok(input.clone()));
        assert_eq!(engine.freeze(&template), Ok(tuple(input)));
    }
    // No Tuple role means no pack inference through an opaque normal-form head.
    let mut engine = Engine::default();
    let pack = engine.fresh(Sort::ComptimeValue);
    let template = I::Instantiate {
        template: tuple(Term::Bound {
            depth: 0,
            index: 0,
            sort: Sort::ComptimeValue,
        }),
        arguments: vec![I::Meta(pack)],
    };
    let (mut candidate, id) = engine
        .match_types(
            template,
            I::Stable(tuple(Term::TypeListValue(vec![]))),
            Budget::default(),
        )
        .unwrap();
    finish(&mut candidate);
    assert!(!candidate.state(id).unwrap().is_proven());
    assert_eq!(engine.commit(candidate), Err(Error::Unsettled(id)));
    engine
        .assign(pack, I::Stable(Term::TypeListValue(vec![])))
        .unwrap();
    assert_eq!(
        engine.freeze(&I::Meta(pack)),
        Ok(Term::TypeListValue(vec![]))
    );
}

#[test]
fn record_fields_infer_only_with_matching_names_and_consistent_types() {
    for (names, types, accepted) in [
        (["first", "second"], [P::Bool, P::Bool], true),
        (["first", "second"], [P::Bool, P::Char], false),
        (["second", "first"], [P::Bool, P::Bool], false),
        (["first", "other"], [P::Bool, P::Bool], false),
    ] {
        let mut engine = Engine::default();
        let a = engine.fresh(Sort::Type);
        let template = I::Instantiate {
            template: Term::Record(vec![("first".into(), slot(0)), ("second".into(), slot(0))]),
            arguments: vec![I::Meta(a)],
        };
        let actual = I::Stable(Term::Record(
            names
                .into_iter()
                .zip(types)
                .map(|(name, ty)| (name.into(), Term::Primitive(ty)))
                .collect(),
        ));
        let (mut candidate, id) = engine
            .match_types(template, actual, Budget::default())
            .unwrap();
        finish(&mut candidate);
        assert_eq!(candidate.state(id).unwrap().is_proven(), accepted);
        if accepted {
            engine.commit(candidate).unwrap();
            assert_eq!(engine.freeze(&I::Meta(a)), Ok(Term::Primitive(P::Bool)));
        } else {
            assert_eq!(engine.commit(candidate), Err(Error::Unsettled(id)));
            engine.assign(a, stable(P::U32)).unwrap();
            assert_eq!(engine.freeze(&I::Meta(a)), Ok(Term::Primitive(P::U32)));
        }
    }
}

#[test]
fn nested_tuple_pointer_inference_is_committed_with_original_proof() {
    let mut engine = Engine::default();
    let a = engine.fresh(Sort::Type);
    let b = engine.fresh(Sort::Type);
    let template = I::Instantiate {
        template: Term::Tuple(vec![slot(0), Term::Pointer(Box::new(slot(1)))]),
        arguments: vec![I::Meta(a), I::Meta(b)],
    };
    let actual = I::Stable(Term::Tuple(vec![
        Term::Primitive(P::Bool),
        Term::Pointer(Box::new(Term::Primitive(P::U32))),
    ]));
    let (mut candidate, id) = engine
        .match_types(template, actual, Budget::default())
        .unwrap();
    finish(&mut candidate);
    assert!(candidate.state(id).unwrap().is_proven());
    engine.commit(candidate).unwrap();
    assert_eq!(engine.freeze(&I::Meta(a)), Ok(Term::Primitive(P::Bool)));
    assert_eq!(engine.freeze(&I::Meta(b)), Ok(Term::Primitive(P::U32)));
}

#[test]
fn repeated_slot_conflict_rolls_back_all_proposals() {
    let mut engine = Engine::default();
    let a = engine.fresh(Sort::Type);
    let template = I::Instantiate {
        template: Term::Tuple(vec![slot(0), slot(0)]),
        arguments: vec![I::Meta(a)],
    };
    let actual = I::Stable(Term::Tuple(vec![
        Term::Primitive(P::Bool),
        Term::Primitive(P::Char),
    ]));
    let (mut candidate, id) = engine
        .match_types(template, actual, Budget::default())
        .unwrap();
    finish(&mut candidate);
    assert!(matches!(
        candidate.state(id),
        Some(State::Solved(Outcome::NotEntailed(_)))
    ));
    assert_eq!(engine.commit(candidate), Err(Error::Unsettled(id)));
    engine.assign(a, stable(P::U32)).unwrap();
    assert_eq!(engine.freeze(&I::Meta(a)), Ok(Term::Primitive(P::U32)));
}

#[test]
fn opaque_heads_never_infer_their_arguments_by_injectivity() {
    let mut engine = Engine::default();
    let a = engine.fresh(Sort::Type);
    let head = nir::DeclarationId {
        package: 1,
        local: 2,
    };
    for nominal in [false, true] {
        let build = |arg| {
            if nominal {
                Term::Nominal {
                    declaration: head,
                    arguments: vec![arg],
                }
            } else {
                Term::NormalForm {
                    head,
                    sort: Sort::Type,
                    arguments: vec![arg],
                }
            }
        };
        let left = I::Instantiate {
            template: build(slot(0)),
            arguments: vec![I::Meta(a)],
        };
        let right = I::Stable(build(Term::Primitive(P::Bool)));
        let (mut candidate, id) = engine.match_types(left, right, Budget::default()).unwrap();
        finish(&mut candidate);
        assert!(matches!(candidate.state(id), Some(State::Waiting(ids)) if ids.contains(&a)));
        assert_eq!(engine.commit(candidate), Err(Error::Unsettled(id)));
    }
    engine.assign(a, stable(P::U32)).unwrap();
}

#[test]
fn function_binders_allow_closed_inference_but_never_capture_local_parameters() {
    let mut engine = Engine::default();
    let a = engine.fresh(Sort::Type);
    let left = I::Instantiate {
        template: Term::Function {
            parameters: vec![],
            result: Box::new(Term::Bound {
                depth: 1,
                index: 0,
                sort: Sort::Type,
            }),
        },
        arguments: vec![I::Meta(a)],
    };
    let right = I::Stable(Term::Function {
        parameters: vec![],
        result: Box::new(Term::Primitive(P::Bool)),
    });
    let (mut candidate, id) = engine.match_types(left, right, Budget::default()).unwrap();
    finish(&mut candidate);
    assert!(candidate.state(id).unwrap().is_proven());
    engine.commit(candidate).unwrap();
    assert_eq!(engine.freeze(&I::Meta(a)), Ok(Term::Primitive(P::Bool)));

    let mut engine = Engine::default();
    let a = engine.fresh(Sort::Type);
    let parameters = vec![nir::Parameter {
        sort: Sort::Type,
        ty: Term::Primitive(P::Type),
    }];
    let left = I::Instantiate {
        template: Term::Function {
            parameters: parameters.clone(),
            result: Box::new(Term::Bound {
                depth: 1,
                index: 0,
                sort: Sort::Type,
            }),
        },
        arguments: vec![I::Meta(a)],
    };
    let right = I::Stable(Term::Function {
        parameters,
        result: Box::new(slot(0)),
    });
    let (mut candidate, id) = engine.match_types(left, right, Budget::default()).unwrap();
    finish(&mut candidate);
    assert!(matches!(candidate.state(id), Some(State::Waiting(_))));
}

#[test]
fn occurs_and_capacity_failures_leave_parent_untouched() {
    let mut engine = Engine::default();
    let a = engine.fresh(Sort::Type);
    let recursive = I::Instantiate {
        template: Term::Pointer(Box::new(slot(0))),
        arguments: vec![I::Meta(a)],
    };
    assert!(matches!(
        engine.match_types(I::Meta(a), recursive, Budget::default()),
        Err(MatchError::Inference(
            type_resolution::InferenceError::Occurs(_)
        ))
    ));
    assert!(matches!(
        engine.match_types(
            I::Meta(a),
            stable(P::Bool),
            Budget {
                steps: 0,
                max_depth: 128
            }
        ),
        Err(MatchError::BudgetExhausted)
    ));
    engine.assign(a, stable(P::Char)).unwrap();
}

#[test]
fn assigned_aliases_and_existing_waiters_join_the_candidate() {
    let mut engine = Engine::default();
    let a = engine.fresh(Sort::Type);
    let b = engine.fresh(Sort::Type);
    engine.assign(a, I::Meta(b)).unwrap();
    let waiting = engine.require(type_resolution::obligations::Requirement::Kind(
        I::Meta(a),
        type_resolution::ptc::TypeKind::Integer,
    ));
    finish(&mut engine);
    let (mut candidate, relation) = engine
        .match_types(I::Meta(a), stable(P::U32), Budget::default())
        .unwrap();
    finish(&mut candidate);
    assert!(candidate.state(waiting).unwrap().is_proven());
    assert!(candidate.state(relation).unwrap().is_proven());
    engine.commit(candidate).unwrap();
    assert_eq!(engine.freeze(&I::Meta(b)), Ok(Term::Primitive(P::U32)));
}

#[test]
fn two_open_structures_share_evidence_without_freezing_unrelated_slots() {
    let mut root = Engine::default();
    let a = root.fresh(Sort::Type);
    let b = root.fresh(Sort::Type);
    let c = root.fresh(Sort::Type);
    let left = I::Instantiate {
        template: Term::Tuple(vec![slot(0), Term::Pointer(Box::new(slot(1)))]),
        arguments: vec![I::Meta(a), I::Meta(b)],
    };
    let right = I::Instantiate {
        template: Term::Tuple(vec![slot(0), Term::Pointer(Box::new(slot(1)))]),
        arguments: vec![stable(P::Bool), I::Meta(c)],
    };
    let (mut candidate, _) = root
        .match_many(
            &[(left.clone(), right.clone()), (I::Meta(c), stable(P::Char))],
            Budget::default(),
        )
        .unwrap();
    finish(&mut candidate);
    assert_eq!(candidate.freeze(&I::Meta(a)), Ok(Term::Primitive(P::Bool)));
    assert_eq!(candidate.freeze(&I::Meta(b)), Ok(Term::Primitive(P::Char)));
    assert_eq!(candidate.freeze(&left), candidate.freeze(&right));
    root.commit(candidate).unwrap();
}

#[test]
fn open_function_templates_remove_local_binders_before_assigning_types() {
    let mut root = Engine::default();
    let a = root.fresh(Sort::Type);
    let b = root.fresh(Sort::Type);
    let callback = Term::Function {
        parameters: vec![nir::Parameter {
            sort: Sort::Runtime,
            ty: Term::Bound {
                depth: 1,
                index: 0,
                sort: Sort::Type,
            },
        }],
        result: Box::new(Term::Bound {
            depth: 1,
            index: 1,
            sort: Sort::Type,
        }),
    };
    let left = I::Instantiate {
        template: callback.clone(),
        arguments: vec![I::Meta(a), stable(P::Char)],
    };
    let right = I::Instantiate {
        template: callback,
        arguments: vec![stable(P::Bool), I::Meta(b)],
    };
    let (mut candidate, _) = root.match_types(left, right, Budget::default()).unwrap();
    finish(&mut candidate);
    assert_eq!(candidate.freeze(&I::Meta(a)), Ok(Term::Primitive(P::Bool)));
    assert_eq!(candidate.freeze(&I::Meta(b)), Ok(Term::Primitive(P::Char)));
    root.commit(candidate).unwrap();
}

#[test]
fn open_structural_conflicts_cannot_commit_inference_proposals() {
    let mut root = Engine::default();
    let a = root.fresh(Sort::Type);
    let b = root.fresh(Sort::Type);
    let left = I::Instantiate {
        template: Term::Tuple(vec![slot(0), slot(0)]),
        arguments: vec![I::Meta(a)],
    };
    let right = I::Instantiate {
        template: Term::Tuple(vec![Term::Primitive(P::Bool), slot(0)]),
        arguments: vec![I::Meta(b)],
    };
    let (mut candidate, _) = root
        .match_many(
            &[(left, right), (I::Meta(b), stable(P::Char))],
            Budget::default(),
        )
        .unwrap();
    finish(&mut candidate);
    assert!(root.commit(candidate).is_err());
    root.assign(a, stable(P::U32)).unwrap();
    assert_eq!(root.freeze(&I::Meta(a)), Ok(Term::Primitive(P::U32)));
}

#[test]
fn two_open_opaque_heads_do_not_gain_injectivity() {
    for nominal in [false, true] {
        let mut root = Engine::default();
        let a = root.fresh(Sort::Type);
        let b = root.fresh(Sort::Type);
        let head = nir::DeclarationId {
            package: 4,
            local: 7,
        };
        let template = if nominal {
            Term::Nominal {
                declaration: head,
                arguments: vec![slot(0)],
            }
        } else {
            Term::NormalForm {
                head,
                sort: Sort::Type,
                arguments: vec![slot(0)],
            }
        };
        let (mut candidate, id) = root
            .match_types(
                I::Instantiate {
                    template: template.clone(),
                    arguments: vec![I::Meta(a)],
                },
                I::Instantiate {
                    template,
                    arguments: vec![I::Meta(b)],
                },
                Budget::default(),
            )
            .unwrap();
        candidate.assign(b, stable(P::Bool)).unwrap();
        finish(&mut candidate);
        assert!(matches!(candidate.state(id), Some(State::Waiting(ids)) if ids.contains(&a)));
        assert_eq!(root.commit(candidate), Err(Error::Unsettled(id)));
        root.assign(a, stable(P::Char)).unwrap();
    }
}

#[test]
fn two_open_functions_cannot_export_local_type_binders() {
    let mut root = Engine::default();
    let a = root.fresh(Sort::Type);
    let b = root.fresh(Sort::Type);
    let generic = |index| Term::Bound {
        depth: 1,
        index,
        sort: Sort::Type,
    };
    let callback = |argument, result| Term::Function {
        parameters: vec![
            nir::Parameter {
                sort: Sort::Type,
                ty: Term::Primitive(P::Type),
            },
            nir::Parameter {
                sort: Sort::Runtime,
                ty: argument,
            },
        ],
        result: Box::new(result),
    };
    let (mut candidate, id) = root
        .match_types(
            I::Instantiate {
                template: callback(Term::Primitive(P::Char), generic(0)),
                arguments: vec![I::Meta(a)],
            },
            I::Instantiate {
                template: callback(generic(0), slot(0)),
                arguments: vec![I::Meta(b)],
            },
            Budget::default(),
        )
        .unwrap();
    finish(&mut candidate);
    assert!(matches!(candidate.state(id), Some(State::Waiting(ids)) if ids.contains(&a)));
    assert_eq!(root.commit(candidate), Err(Error::Unsettled(id)));
}

#[test]
fn structural_inference_ignores_unused_self_slots_in_shared_environments() {
    let mut root = Engine::default();
    let a = root.fresh(Sort::Type);
    let b = root.fresh(Sort::Type);
    let arguments = vec![I::Meta(a), I::Meta(b)];
    let left = I::Instantiate {
        template: Term::Tuple(vec![slot(0), slot(1)]),
        arguments: arguments.clone(),
    };
    let right = I::Instantiate {
        template: Term::Tuple(vec![
            Term::Pointer(Box::new(slot(1))),
            Term::Primitive(P::Bool),
        ]),
        arguments,
    };
    let (mut candidate, _) = root.match_types(left, right, Budget::default()).unwrap();
    finish(&mut candidate);
    assert_eq!(
        candidate.freeze(&I::Meta(a)),
        Ok(Term::Pointer(Box::new(Term::Primitive(P::Bool))))
    );
    root.commit(candidate).unwrap();
}

#[test]
fn metered_multi_match_reports_failed_trial_work_without_committing_assignments() {
    let mut parent = Engine::default();
    let variable = parent.fresh(Sort::Type);
    let relations = vec![
        (I::Meta(variable), stable(P::Bool)),
        (stable(P::Char), stable(P::Char)),
    ];
    let mut small = Budget {
        steps: 2,
        ..Default::default()
    };
    assert!(parent.match_many_metered(&relations, &mut small).is_err());
    assert!(small.steps < 2);
    assert!(parent.freeze(&I::Meta(variable)).is_err());
    let mut enough = Budget::default();
    let before = enough.steps;
    let (mut candidate, ids) = parent.match_many_metered(&relations, &mut enough).unwrap();
    assert!(enough.steps < before);
    candidate.advance_metered(ids.len(), &mut enough);
    assert!(
        ids.iter()
            .all(|id| candidate.state(*id).unwrap().is_proven())
    );
    parent.commit(candidate).unwrap();
    assert_eq!(
        parent.freeze(&I::Meta(variable)).unwrap(),
        Term::Primitive(P::Bool)
    );
}

#[test]
fn copying_wide_matching_inputs_is_charged_before_structural_shortcuts() {
    let parent = Engine::default();
    let input = I::Stable(Term::Tuple(vec![Term::Primitive(P::Bool); 256]));
    let relations = vec![(input.clone(), input)];
    let mut small = Budget {
        steps: 16,
        ..Default::default()
    };
    assert!(matches!(
        parent.match_many_metered(&relations, &mut small),
        Err(MatchError::BudgetExhausted)
    ));
    let mut enough = Budget::default();
    let before = enough.steps;
    let (mut candidate, ids) = parent.match_many_metered(&relations, &mut enough).unwrap();
    assert!(enough.steps + 512 < before);
    candidate.advance_metered(ids.len(), &mut enough);
    assert!(
        ids.iter()
            .all(|id| candidate.state(*id).unwrap().is_proven())
    );
}

#[test]
fn matching_copy_budget_does_not_evaluate_unused_substitution_inputs() {
    let parent = Engine::default();
    let mut foreign = Engine::default();
    let unused = foreign.fresh(Sort::Type);
    let input = I::Instantiate {
        template: Term::Primitive(P::Bool),
        arguments: vec![I::Meta(unused); 64],
    };
    let relations = vec![(input, stable(P::Bool))];
    let mut small = Budget {
        steps: 16,
        ..Default::default()
    };
    assert!(matches!(
        parent.match_many_metered(&relations, &mut small),
        Err(MatchError::BudgetExhausted)
    ));
    let mut enough = Budget::default();
    let (mut candidate, ids) = parent.match_many_metered(&relations, &mut enough).unwrap();
    candidate.advance_metered(ids.len(), &mut enough);
    assert!(
        ids.iter()
            .all(|id| candidate.state(*id).unwrap().is_proven())
    );
}

#[test]
fn bounded_alias_resolution_preserves_unresolved_inputs_and_recovers() {
    let mut context = type_resolution::InferenceContext::default();
    let first = context.fresh(Sort::Type);
    let mut previous = first;
    for _ in 0..64 {
        let next = context.fresh(Sort::Type);
        context.assign(previous, I::Meta(next)).unwrap();
        previous = next;
    }
    let mut small = 8;
    assert!(matches!(
        context.resolve_with_limits(&I::Meta(first), &mut small, 128),
        Err(type_resolution::InferenceError::Budget)
    ));
    let mut enough = 10_000;
    assert_eq!(
        context
            .resolve_with_limits(&I::Meta(first), &mut enough, 128)
            .unwrap(),
        I::Meta(previous)
    );
    context.assign(previous, stable(P::Bool)).unwrap();
    assert_eq!(
        context
            .resolve_with_limits(&I::Meta(first), &mut enough, 128)
            .unwrap(),
        stable(P::Bool)
    );
    assert!(enough < 10_000);
}

#[test]
fn bounded_assignment_does_not_publish_on_exhaustion_or_occurs_failure() {
    let mut context = type_resolution::InferenceContext::default();
    let target = context.fresh(Sort::Type);
    let first = context.fresh(Sort::Type);
    let mut previous = first;
    for _ in 0..32 {
        let next = context.fresh(Sort::Type);
        context.assign(previous, I::Meta(next)).unwrap();
        previous = next;
    }
    let mut small = 4;
    assert!(matches!(
        context.assign_with_limits(target, I::Meta(first), &mut small, 128),
        Err(type_resolution::InferenceError::Budget)
    ));
    assert!(context.freeze(&I::Meta(target)).is_err());
    context.assign(previous, I::Meta(target)).unwrap();
    let mut enough = 10_000;
    assert!(
        matches!(context.assign_with_limits(target,I::Meta(first),&mut enough,128),Err(type_resolution::InferenceError::Occurs(id)) if id==target)
    );
    context
        .assign_with_limits(target, stable(P::Bool), &mut enough, 128)
        .unwrap();
    assert_eq!(
        context.freeze(&I::Meta(first)).unwrap(),
        Term::Primitive(P::Bool)
    );
}
