use nir::program::TypePredicate;
use nir::{DeclarationId as Id, Primitive as P, Sort, Term};
use type_resolution::implementation::{
    ImplementationHead, Index, InherentHead, InherentIndex, Selection, TypeAssessment,
};
use type_resolution::ptc::{Budget, Context, Formula};
fn id(local: u32) -> Id {
    Id { package: 0, local }
}
fn var(n: u32) -> Term {
    Term::Rigid {
        declaration: id(n),
        sort: Sort::Type,
    }
}
fn boxed(t: Term) -> Term {
    Term::Nominal {
        declaration: id(10),
        arguments: vec![t],
    }
}
fn head(n: u32, subject: Term, parameters: Vec<Id>, names: &[&str]) -> InherentHead {
    InherentHead {
        parameter_types: vec![],
        declaration: id(n),
        subject,
        type_parameters: parameters,
        requirements: vec![],
        members: names.iter().map(|s| (*s).to_owned()).collect(),
    }
}
fn context() -> Context {
    let mut c = Context::default();
    c.admit_nominal_constructor(id(10));
    c
}

#[test]
fn trait_and_inherent_domains_refute_closed_heads_before_subject_eligibility() {
    let concrete = Term::Primitive(P::Bool);
    let mut inherent = InherentIndex::for_builtin_packages(None, [0]);
    inherent
        .insert(
            head(20, concrete.clone(), vec![], &["read"]),
            Budget::default(),
        )
        .unwrap();
    let mut traits = Index::for_builtin_packages(None, [0]);
    traits
        .insert(
            ImplementationHead {
                declaration: id(20),
                subject: concrete.clone(),
                interface: Some(id(100)),
                interface_arguments: vec![],
                parameter_types: vec![],
                type_parameters: vec![],
                requirements: vec![],
            },
            Budget::default(),
        )
        .unwrap();
    for subject in [
        var(90),
        Term::NormalForm {
            head: id(91),
            sort: Sort::Type,
            arguments: vec![var(92)],
        },
    ] {
        let resolve = |context: &Context| {
            [
                inherent.resolve_types("read", &subject, context, Budget::default()),
                traits.resolve_types(id(100), &[], &subject, context, Budget::default()),
            ]
        };
        let unknown = Context::default();
        for result in resolve(&unknown) {
            assert!(matches!(
                result.unwrap().selection,
                Selection::Deferred { .. }
            ));
        }
        let mut different = unknown.fork();
        different.assume(Formula::equal(subject.clone(), concrete.clone()).negate());
        for result in resolve(&different) {
            let result = result.unwrap();
            assert_eq!(result.selection, Selection::NoApplicableCandidate);
            let TypeAssessment::Excluded(proof) = &result.assessments[&id(20)] else {
                panic!("a closed-head exclusion must retain its proof");
            };
            assert_eq!(
                proof.goal,
                Formula::equal(concrete.clone(), subject.clone()).negate()
            );
            different.verify_proof(proof, Budget::default()).unwrap();
            assert!(unknown.verify_proof(proof, Budget::default()).is_err());
        }
        // A separate query can supply positive equality. Earlier exclusion
        // evidence is neither cached as a fact nor reused for the new premise.
        let mut equal = unknown.fork();
        equal.assume(Formula::equal(subject.clone(), concrete.clone()));
        for (position, result) in resolve(&equal).into_iter().enumerate() {
            let result = result.unwrap();
            assert_eq!(result.selection, Selection::Selected(id(20)));
            let TypeAssessment::Applicable(evidence) = &result.assessments[&id(20)] else {
                panic!("selection requires applicable evidence");
            };
            if position == 0 {
                inherent
                    .verify_evidence("read", evidence, &subject, &equal, Budget::default())
                    .unwrap();
                assert!(
                    inherent
                        .verify_evidence("read", evidence, &subject, &unknown, Budget::default())
                        .is_err()
                );
            } else {
                traits
                    .head(id(20))
                    .unwrap()
                    .verify_type_evidence(evidence, &subject, &[], &equal, Budget::default())
                    .unwrap();
                assert!(
                    traits
                        .head(id(20))
                        .unwrap()
                        .verify_type_evidence(evidence, &subject, &[], &unknown, Budget::default())
                        .is_err()
                );
            }
        }
        for result in resolve(&unknown) {
            assert!(matches!(
                result.unwrap().selection,
                Selection::Deferred { .. }
            ));
        }
    }
}

#[test]
fn inherent_closed_exclusion_remains_proof_backed_across_budget_boundaries() {
    let mut index = InherentIndex::for_builtin_packages(None, [0]);
    let concrete = Term::Primitive(P::Bool);
    index
        .insert(
            head(20, concrete.clone(), vec![], &["read"]),
            Budget::default(),
        )
        .unwrap();
    let subject = var(90);
    let mut context = Context::default();
    context.assume(Formula::equal(subject.clone(), concrete).negate());
    let mut incomplete = 0;
    let mut excluded = 0;
    for steps in [0, 1, 8, 32, 128, 512, 2048, 8192, 100_000] {
        match index.resolve_types(
            "read",
            &subject,
            &context,
            Budget {
                steps,
                ..Budget::default()
            },
        ) {
            Err(type_resolution::implementation::Error::BudgetExhausted) => incomplete += 1,
            Ok(result) if matches!(result.selection, Selection::Deferred { .. }) => {
                incomplete += 1;
            }
            Ok(result) => {
                assert_eq!(result.selection, Selection::NoApplicableCandidate);
                let TypeAssessment::Excluded(proof) = &result.assessments[&id(20)] else {
                    panic!("an incomplete query cannot exclude the candidate");
                };
                context.verify_proof(proof, Budget::default()).unwrap();
                excluded += 1;
            }
            Err(failure) => panic!("unexpected failure at {steps} steps: {failure:?}"),
        }
    }
    assert!(incomplete > 0 && excluded > 0);
    assert_eq!(
        index
            .resolve_types("read", &subject, &context, Budget::default())
            .unwrap()
            .selection,
        Selection::NoApplicableCandidate
    );
}
#[test]
fn inherent_selection_filters_names_before_precision() {
    let mut index = InherentIndex::default();
    index
        .insert(
            head(20, boxed(var(1)), vec![id(1)], &["get", "read"]),
            Budget::default(),
        )
        .unwrap();
    index
        .insert(
            head(21, boxed(Term::Primitive(P::Bool)), vec![], &["read"]),
            Budget::default(),
        )
        .unwrap();
    index
        .check_coherence(&context(), Budget::default())
        .unwrap();
    let subject = boxed(Term::Primitive(P::Bool));
    let get = index
        .resolve_types("get", &subject, &context(), Budget::default())
        .unwrap();
    assert_eq!(get.selection, Selection::Selected(id(20)));
    let TypeAssessment::Applicable(evidence) = &get.assessments[&id(20)] else {
        panic!()
    };
    assert_eq!(evidence.type_arguments, vec![Term::Primitive(P::Bool)]);
    index
        .verify_evidence("get", evidence, &subject, &context(), Budget::default())
        .unwrap();
    assert!(
        index
            .verify_evidence("missing", evidence, &subject, &context(), Budget::default())
            .is_err()
    );
    let mut corrupted = evidence.clone();
    corrupted.type_arguments[0] = Term::Primitive(P::Char);
    assert!(
        index
            .verify_evidence("get", &corrupted, &subject, &context(), Budget::default())
            .is_err()
    );
    assert_eq!(
        index
            .resolve_types("read", &subject, &context(), Budget::default())
            .unwrap()
            .selection,
        Selection::Selected(id(21))
    );
    assert_eq!(
        index
            .resolve_types("missing", &subject, &context(), Budget::default())
            .unwrap()
            .selection,
        Selection::NoApplicableCandidate
    );
    assert!(matches!(
        index
            .resolve_types("read", &boxed(var(2)), &context(), Budget::default())
            .unwrap()
            .selection,
        Selection::Deferred { .. }
    ));
}
#[test]
fn inherent_requires_are_proved_and_coherence_ignores_caller_assumptions() {
    let mut index = InherentIndex::default();
    for (n, p) in [(20, P::Bool), (21, P::Char)] {
        let mut h = head(n, boxed(var(1)), vec![id(1)], &["read"]);
        h.requirements
            .push(TypePredicate::Equal(var(1), Term::Primitive(p)));
        index.insert(h, Budget::default()).unwrap();
    }
    index
        .check_coherence(&context(), Budget::default())
        .unwrap();
    assert_eq!(
        index
            .resolve_types(
                "read",
                &boxed(Term::Primitive(P::Bool)),
                &context(),
                Budget::default()
            )
            .unwrap()
            .selection,
        Selection::Selected(id(20))
    );
    index
        .insert(
            head(22, boxed(var(2)), vec![id(2)], &["read"]),
            Budget::default(),
        )
        .unwrap();
    let mut impossible = context();
    impossible.assume(Formula::False);
    assert!(
        index
            .check_coherence(&impossible, Budget::default())
            .is_err()
    );
}
#[test]
fn inherent_admission_is_atomic_and_resolution_tracks_world_identity() {
    let mut index = InherentIndex::default();
    index
        .insert(
            head(20, boxed(var(1)), vec![id(1)], &["get"]),
            Budget::default(),
        )
        .unwrap();
    let resolution = index
        .resolve_types(
            "get",
            &boxed(Term::Primitive(P::Bool)),
            &context(),
            Budget::default(),
        )
        .unwrap();
    let before = index.to_string();
    let foreign = Term::Nominal {
        declaration: Id {
            package: 1,
            local: 10,
        },
        arguments: vec![],
    };
    assert!(
        index
            .insert(head(21, foreign, vec![], &["get"]), Budget::default())
            .is_err()
    );
    assert!(
        index
            .insert(
                head(22, boxed(var(1)), vec![id(1)], &["get"]),
                Budget {
                    steps: 0,
                    ..Budget::default()
                }
            )
            .is_err()
    );
    assert_eq!(index.to_string(), before);
    assert!(index.resolution_is_current(&resolution));
    assert!(!InherentIndex::default().resolution_is_current(&resolution));
    index
        .insert(
            head(23, boxed(var(1)), vec![id(1)], &["other"]),
            Budget::default(),
        )
        .unwrap();
    assert!(!index.resolution_is_current(&resolution));
    assert!(
        index
            .check_coherence(
                &context(),
                Budget {
                    steps: 0,
                    ..Budget::default()
                }
            )
            .is_err()
    );
}
#[test]
fn inherent_matching_does_not_resolve_an_open_trait_head() {
    let head = ImplementationHead {
        parameter_types: vec![],
        declaration: id(20),
        subject: boxed(var(1)),
        type_parameters: vec![id(1)],
        requirements: vec![],
        interface: None,
        interface_arguments: vec![],
    };
    assert!(matches!(
        head.assess_types(
            &boxed(Term::Primitive(P::Bool)),
            &[],
            &context(),
            Budget::default()
        ),
        TypeAssessment::Pending(_)
    ));
    let mut index = InherentIndex::default();
    index.insert(super_head(), Budget::default()).unwrap();
    let dump = index.to_string();
    assert!(dump.contains("inherent-head"));
    assert!(dump.contains("quote\\\"newline\\n"), "{dump}");
}
fn super_head() -> InherentHead {
    head(20, boxed(var(1)), vec![id(1)], &["quote\"newline\n"])
}

#[test]
fn external_value_domains_control_inherent_matching_and_replay() {
    let value = |n| Term::Rigid {
        declaration: id(n),
        sort: Sort::ComptimeValue,
    };
    let mut h = head(20, boxed(value(1)), vec![id(1)], &["read"]);
    h.parameter_types = vec![Term::Primitive(P::Bool)];
    let mut index = InherentIndex::default();
    index.insert(h.clone(), Budget::default()).unwrap();
    let subject = boxed(value(2));
    let before = index
        .resolve_types("read", &subject, &context(), Budget::default())
        .unwrap();
    assert!(!matches!(before.selection, Selection::Selected(_)));
    index
        .register_parameter_domain(id(2), Term::Primitive(P::Bool))
        .unwrap();
    assert!(!index.resolution_is_current(&before));
    let result = index
        .resolve_types("read", &subject, &context(), Budget::default())
        .unwrap();
    assert!(matches!(result.selection, Selection::Selected(_)));
    let TypeAssessment::Applicable(evidence) = &result.assessments[&id(20)] else {
        panic!("{result}")
    };
    index
        .verify_evidence("read", evidence, &subject, &context(), Budget::default())
        .unwrap();
    assert!(
        index
            .verify_evidence("other", evidence, &subject, &context(), Budget::default())
            .is_err()
    );
    assert!(
        index
            .verify_evidence(
                "read",
                evidence,
                &subject,
                &context(),
                Budget {
                    steps: 0,
                    ..Default::default()
                }
            )
            .is_err()
    );
    assert!(
        index
            .register_parameter_domain(id(2), Term::Primitive(P::Char))
            .is_err()
    );
    assert!(index.resolution_is_current(&result));
    let mut wrong = InherentIndex::default();
    wrong.insert(h, Budget::default()).unwrap();
    wrong
        .register_parameter_domain(id(2), Term::Primitive(P::Char))
        .unwrap();
    assert!(
        wrong
            .verify_evidence("read", evidence, &subject, &context(), Budget::default())
            .is_err()
    );
    assert!(!matches!(
        wrong
            .resolve_types("read", &subject, &context(), Budget::default())
            .unwrap()
            .selection,
        Selection::Selected(_)
    ));
    assert!(index.to_string().contains("(parameter-domain"));
}

#[test]
fn nominal_payload_layouts_govern_selection_replay_and_revision() {
    let config = Term::Nominal {
        declaration: id(30),
        arguments: vec![],
    };
    let mut h = head(
        20,
        boxed(Term::Rigid {
            declaration: id(1),
            sort: Sort::ComptimeValue,
        }),
        vec![id(1)],
        &["read"],
    );
    h.parameter_types = vec![config.clone()];
    let mut index = InherentIndex::default();
    index.insert(h, Budget::default()).unwrap();
    let value = Term::NominalValue {
        ty: Box::new(config),
        variant: None,
        fields: vec![Term::Bool(true)],
    };
    let subject = boxed(value.clone());
    let before = index
        .resolve_types("read", &subject, &context(), Budget::default())
        .unwrap();
    assert!(!matches!(before.selection, Selection::Selected(_)));
    let mut layout = nir::program::Struct {
        declaration: id(30),
        name: "Config".into(),
        type_parameters: vec![],
        parameter_types: vec![],
        fields: vec![("flag".into(), Term::Primitive(P::Bool))],
        requirements: vec![],
        capabilities: vec![],
    };
    index.set_nominal_layouts(vec![layout.clone()], vec![], vec![]);
    assert!(!index.resolution_is_current(&before));
    let selected = index
        .resolve_types("read", &subject, &context(), Budget::default())
        .unwrap();
    assert_eq!(selected.selection, Selection::Selected(id(20)));
    let TypeAssessment::Applicable(evidence) = &selected.assessments[&id(20)] else {
        panic!("{selected}")
    };
    index
        .verify_evidence("read", evidence, &subject, &context(), Budget::default())
        .unwrap();
    index.set_nominal_layouts(vec![layout.clone()], vec![], vec![]);
    assert!(index.resolution_is_current(&selected));
    assert!(index.to_string().contains("(nominal-layouts (struct"));
    for fields in [vec![], vec![Term::Char('x')]] {
        let mut forged = value.clone();
        if let Term::NominalValue {
            fields: payload, ..
        } = &mut forged
        {
            *payload = fields;
        }
        let invalid = index
            .resolve_types("read", &boxed(forged), &context(), Budget::default())
            .unwrap();
        assert!(!matches!(invalid.selection, Selection::Selected(_)));
    }
    layout.fields[0].1 = Term::Primitive(P::Char);
    index.set_nominal_layouts(vec![layout], vec![], vec![]);
    assert!(!index.resolution_is_current(&selected));
    assert!(
        index
            .verify_evidence("read", evidence, &subject, &context(), Budget::default())
            .is_err()
    );
}

#[test]
fn trusted_list_excludes_optional_member_without_solving_its_payload() {
    let list = Term::NormalForm {
        head: id(100),
        sort: Sort::Type,
        arguments: vec![Term::Primitive(P::Bool)],
    };
    let mut index = InherentIndex::for_builtin_packages(None, [0]);
    index
        .insert(
            head(
                20,
                Term::Optional(Box::new(var(1))),
                vec![id(1)],
                &["zip_with"],
            ),
            Budget::default(),
        )
        .unwrap();
    let mut context = Context::default();
    context.admit_list_inference_head(id(100));
    let resolution = index
        .resolve_types("zip_with", &list, &context, Budget::default())
        .unwrap();
    assert_eq!(resolution.selection, Selection::NoApplicableCandidate);
    let optional = Term::Optional(Box::new(Term::Primitive(P::Bool)));
    assert_eq!(
        index
            .resolve_types("zip_with", &optional, &context, Budget::default())
            .unwrap()
            .selection,
        Selection::Selected(id(20))
    );
    // Removing trusted family classification must not invent apartness for
    // opaque factories. The same name remains undecidable in that world.
    assert!(matches!(
        index
            .resolve_types("zip_with", &list, &Context::default(), Budget::default())
            .unwrap()
            .selection,
        Selection::Deferred { .. }
    ));
}

#[test]
fn inherent_matching_and_replay_preserve_caller_type_scope() {
    let mut index = InherentIndex::default();
    index
        .insert(
            head(20, boxed(var(1)), vec![id(1)], &["get"]),
            Budget::default(),
        )
        .unwrap();
    let nested = context().theory_in_scope(vec![vec![Sort::Type], vec![Sort::Type]]);
    let closed_subject = boxed(Term::Primitive(P::Bool));
    let closed = index
        .resolve_types("get", &closed_subject, &nested, Budget::default())
        .unwrap();
    assert_eq!(closed.selection, Selection::Selected(id(20)));
    let scoped = context().theory_in_scope(vec![vec![Sort::Type]]);
    let input = Term::Bound {
        depth: 0,
        index: 0,
        sort: Sort::Type,
    };
    let subject = boxed(input.clone());
    let result = index
        .resolve_types("get", &subject, &scoped, Budget::default())
        .unwrap();
    assert_eq!(
        result.selection,
        Selection::Selected(id(20)),
        "{:?}",
        result.assessments
    );
    let TypeAssessment::Applicable(evidence) = &result.assessments[&id(20)] else {
        panic!()
    };
    assert_eq!(evidence.type_arguments, vec![input]);
    index
        .verify_evidence("get", evidence, &subject, &scoped, Budget::default())
        .unwrap();
    assert!(
        index
            .verify_evidence("get", evidence, &subject, &context(), Budget::default())
            .is_err()
    );
    let wrong_sort = context().theory_in_scope(vec![vec![Sort::ComptimeValue]]);
    assert!(
        index
            .verify_evidence("get", evidence, &subject, &wrong_sort, Budget::default())
            .is_err()
    );
}

#[test]
fn inherent_candidates_preserve_distinct_nested_binders_and_empty_frames() {
    let pair = |left, right| Term::Nominal {
        declaration: id(10),
        arguments: vec![left, right],
    };
    let mut index = InherentIndex::default();
    index
        .insert(
            head(20, pair(var(1), var(2)), vec![id(1), id(2)], &["get"]),
            Budget::default(),
        )
        .unwrap();
    for inner in [vec![Sort::Type], vec![]] {
        let context = context().theory_in_scope(vec![vec![Sort::Type], inner.clone()]);
        let outer = Term::Bound {
            depth: 1,
            index: 0,
            sort: Sort::Type,
        };
        let second = if inner.is_empty() {
            Term::Primitive(P::Bool)
        } else {
            Term::Bound {
                depth: 0,
                index: 0,
                sort: Sort::Type,
            }
        };
        let subject = pair(outer.clone(), second.clone());
        let result = index
            .resolve_types("get", &subject, &context, Budget::default())
            .unwrap();
        assert_eq!(
            result.selection,
            Selection::Selected(id(20)),
            "{:?}",
            result.assessments
        );
        let TypeAssessment::Applicable(evidence) = &result.assessments[&id(20)] else {
            panic!()
        };
        assert_eq!(evidence.type_arguments, vec![outer, second]);
        index
            .verify_evidence("get", evidence, &subject, &context, Budget::default())
            .unwrap();
        let mut corrupted = evidence.clone();
        corrupted.type_arguments.swap(0, 1);
        assert!(
            index
                .verify_evidence("get", &corrupted, &subject, &context, Budget::default())
                .is_err()
        );
    }
}
