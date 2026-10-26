use elaboration::implementation::{
    Applicability as A, Error, ImplementationHead, Index, Key, Selection,
};
use elaboration::ptc::Budget;
use nir::{DeclarationId as Id, Primitive as P, Sort, Term};
fn id(local: u32) -> Id {
    Id { package: 0, local }
}
fn var(n: u32) -> Term {
    Term::Rigid {
        declaration: id(n),
        sort: Sort::Type,
    }
}
fn ty(p: P) -> Term {
    Term::Primitive(p)
}
fn node(n: u32, args: Vec<Term>) -> Term {
    Term::Nominal {
        declaration: id(n),
        arguments: args,
    }
}
fn head(n: u32, subject: Term) -> ImplementationHead {
    ImplementationHead {
        parameter_types: vec![],
        type_parameters: vec![id(1), id(2)],
        requirements: vec![],
        declaration: id(n),
        interface: Some(id(100)),
        interface_arguments: vec![],
        subject,
    }
}
fn add(index: &mut Index, n: u32, subject: Term) {
    index.insert(head(n, subject), Budget::default()).unwrap();
}

#[test]
fn coherence_budget_keeps_generic_pairs_usable_beside_large_scalar_groups() {
    check_coherence_group_budget(false);
}

#[test]
fn coherence_budget_separates_precision_groups_within_one_interface() {
    check_coherence_group_budget(true);
}

fn check_coherence_group_budget(same_interface: bool) {
    use elaboration::ptc::Context;
    let mut index = Index::default();
    for group in 0..3 {
        for member in 0..20 {
            let mut subject = node(2000 + member, vec![]);
            if same_interface {
                // Concrete groups have precision 2, 3 and 4; the generic pair
                // below has precision 1 and must keep its own proof allowance.
                for _ in 0..=group {
                    subject = node(5000 + member, vec![subject]);
                }
            }
            let mut implementation = head(1000 + group * 20 + member, subject);
            implementation.type_parameters.clear();
            if !same_interface {
                implementation.interface = Some(id(3000 + group));
            }
            index.insert(implementation, Budget::default()).unwrap();
        }
    }
    for declaration in [4000, 4001] {
        let mut implementation = head(
            declaration,
            node(declaration + 100, vec![var(1), var(2), var(3), var(4)]),
        );
        implementation.type_parameters = (1..=4).map(id).collect();
        index.insert(implementation, Budget::default()).unwrap();
    }
    let context = Context::default();
    let report = index.check_type_coherence(&context, Budget::default());
    assert!(report.is_proved(), "{report}");
    assert!(index.verify_type_coherence(&report, &context, Budget::default()));
    let mut missing_pair = report.clone();
    missing_pair.pairs.pop();
    assert!(!index.verify_type_coherence(&missing_pair, &context, Budget::default()));

    // Enough budget to traverse all entries/pairs, but none left for proofs.
    let exhausted = Budget {
        steps: 62 * 2 + 62 * 61 / 2,
        ..Budget::default()
    };
    assert!(!index.check_type_coherence(&context, exhausted).is_proved());
    assert!(!index.verify_type_coherence(&report, &context, exhausted));
    let recovered = index.check_type_coherence(&context, Budget::default());
    assert!(index.verify_type_coherence(&recovered, &context, Budget::default()));

    // Adding a genuinely overlapping generic implementation must still fail.
    let mut duplicate = head(4002, node(4100, vec![var(1), var(2), var(3), var(4)]));
    duplicate.type_parameters = (1..=4).map(id).collect();
    index.insert(duplicate, Budget::default()).unwrap();
    assert!(
        !index
            .check_type_coherence(&context, Budget::default())
            .is_proved()
    );
    assert!(!index.verify_type_coherence(&report, &context, Budget::default()));
}

#[test]
fn record_value_indices_retain_type_candidates_and_replay_field_proofs() {
    use elaboration::ptc::{Context, Formula};
    let record = |element| {
        Term::RecordValue(vec![
            ("element".into(), element),
            ("flag".into(), Term::Bool(true)),
        ])
    };
    let ab = Term::Record(vec![("a".into(), ty(P::Bool)), ("b".into(), ty(P::Char))]);
    let ba = Term::Record(vec![("b".into(), ty(P::Char)), ("a".into(), ty(P::Bool))]);
    let mut h = head(200, ty(P::Bool));
    h.type_parameters.clear();
    h.interface_arguments = vec![record(ab.clone())];
    let query = vec![record(ba.clone())];
    let mut index = Index::default();
    index.insert(h.clone(), Budget::default()).unwrap();
    assert!(
        index
            .candidates(Some(id(100)), &query, &ty(P::Bool), Budget::default())
            .unwrap()
            .declarations()
            .contains(&id(200))
    );
    let mut context = Context::default();
    context.assume(Formula::equal(ab, ba));
    let matched = h
        .match_types(&ty(P::Bool), &query, &context, Budget::default())
        .unwrap();
    h.verify_head_match(&matched, &ty(P::Bool), &query, &context, Budget::default())
        .unwrap();
    assert!(
        h.verify_head_match(
            &matched,
            &ty(P::Bool),
            &query,
            &Context::default(),
            Budget::default()
        )
        .is_err()
    );
    assert!(
        h.verify_head_match(
            &matched,
            &ty(P::Bool),
            &[record(ty(P::Char))],
            &context,
            Budget::default()
        )
        .is_err()
    );
    let mut altered = query.clone();
    let Term::RecordValue(fields) = &mut altered[0] else {
        unreachable!()
    };
    fields[1].1 = Term::Bool(false);
    assert!(
        h.match_types(&ty(P::Bool), &altered, &context, Budget::default())
            .is_err()
    );
    assert!(
        h.verify_head_match(
            &matched,
            &ty(P::Bool),
            &altered,
            &context,
            Budget::default()
        )
        .is_err()
    );
}

#[test]
fn record_subject_keys_preserve_labels_precision_and_budget() {
    let record = |name: &str, field| Term::Record(vec![(name.into(), field)]);
    let generic = record("enabled", var(1));
    let concrete = record("enabled", ty(P::Bool));
    let key = Key::from_type(&concrete, Budget::default()).unwrap();
    assert!(
        key.precision()
            > Key::from_type(&generic, Budget::default())
                .unwrap()
                .precision()
    );
    let escaped = Key::from_type(&record("a\"b", ty(P::Bool)), Budget::default())
        .unwrap()
        .to_string();
    assert!(escaped.contains("a\\\"b"), "{escaped}");
    let mut index = Index::default();
    add(&mut index, 200, generic);
    add(&mut index, 201, concrete.clone());
    add(&mut index, 202, record("disabled", ty(P::Bool)));
    let candidates = index
        .candidates(Some(id(100)), &[], &concrete, Budget::default())
        .unwrap();
    assert!(candidates.declarations().contains(&id(200)));
    assert!(candidates.declarations().contains(&id(201)));
    assert!(!candidates.declarations().contains(&id(202)));
    let wide = record(&"x".repeat(200), ty(P::Bool));
    assert!(
        Key::from_type(
            &wide,
            Budget {
                steps: 100,
                max_depth: 32
            }
        )
        .is_err()
    );
    assert!(Key::from_type(&wide, Budget::default()).is_ok());
}

#[test]
fn associated_type_queries_do_not_claim_structural_precision_or_ownership() {
    let projection = Term::AssociatedType {
        origin: Box::new(nir::AssociatedTypeOrigin {
            interface: id(100),
            member: id(101),
        }),
        subject: Box::new(node(10, vec![])),
        arguments: vec![],
    };
    assert_eq!(
        Key::from_type(&projection, Budget::default())
            .unwrap()
            .precision(),
        0
    );
    let mut declaration = head(200, projection.clone());
    declaration.interface = Some(Id {
        package: 99,
        local: 100,
    });
    assert!(matches!(
        declaration.ownership(Budget::default()),
        Err(Error::Ownership(_))
    ));
    // An opaque outer application must also visit projection inputs before
    // cloning or deciding whether its key is closed.
    let outer = Term::NormalForm {
        head: id(300),
        sort: Sort::Type,
        arguments: vec![projection],
    };
    assert_eq!(
        Key::from_type(&outer, Budget::default())
            .unwrap()
            .precision(),
        0
    );
    let wide = Term::AssociatedType {
        origin: Box::new(nir::AssociatedTypeOrigin {
            interface: id(100),
            member: id(101),
        }),
        subject: Box::new(ty(P::Bool)),
        arguments: vec![ty(P::Bool); 100],
    };
    let outer = Term::NormalForm {
        head: id(300),
        sort: Sort::Type,
        arguments: vec![wide],
    };
    assert!(matches!(
        Key::from_type(
            &outer,
            Budget {
                steps: 20,
                max_depth: 128
            }
        ),
        Err(Error::BudgetExhausted)
    ));
}

#[test]
fn uniform_precision_stops_at_the_shallowest_unknown_branch() {
    let t = var(1);
    let u = var(2);
    let int = ty(P::I32);
    for (term, expected) in [
        (t.clone(), 0),
        (node(10, vec![t.clone()]), 1),
        (node(10, vec![node(11, vec![t.clone(), u.clone()])]), 2),
        (node(10, vec![node(11, vec![int.clone(), ty(P::U32)])]), 3),
        (node(11, vec![node(10, vec![t.clone()]), u]), 1),
        (node(11, vec![node(10, vec![t.clone()]), int]), 2),
        (node(11, vec![t.clone(), t]), 1),
    ] {
        assert_eq!(
            Key::from_type(&term, Budget::default())
                .unwrap()
                .precision(),
            expected
        );
    }
}

#[test]
fn cuts_preserve_shallow_concrete_leaves() {
    let key = Key::from_type(
        &node(11, vec![node(10, vec![ty(P::Bool)]), ty(P::Char)]),
        Budget::default(),
    )
    .unwrap();
    let cut = key.cut(2).to_string();
    assert!(cut.contains("(identity (primitive char))"));
    assert!(cut.contains("(wildcard)"));
    assert!(!cut.contains("primitive bool"));
}

#[test]
fn retrieval_contains_every_generated_substitution_including_unknown_queries() {
    let templates = [
        var(1),
        node(10, vec![var(1)]),
        node(11, vec![node(10, vec![var(1)]), var(2)]),
        node(11, vec![var(1), var(1)]),
        node(11, vec![var(1), ty(P::Char)]),
    ];
    let ground = [ty(P::Bool), ty(P::Char), node(10, vec![ty(P::Bool)])];
    let mut index = Index::default();
    for (n, term) in templates.iter().enumerate() {
        add(&mut index, n as u32 + 200, term.clone());
    }
    for (n, template) in templates.iter().enumerate() {
        for a in &ground {
            for b in &ground {
                let subject = template
                    .instantiate_types(&[id(1), id(2)], &[a.clone(), b.clone()])
                    .unwrap();
                let candidates = index
                    .candidates(Some(id(100)), &[], &subject, Budget::default())
                    .unwrap();
                assert!(
                    candidates.declarations().contains(&id(n as u32 + 200)),
                    "{template} -> {subject}"
                );
            }
        }
    }
    assert_eq!(
        index
            .candidates(Some(id(100)), &[], &var(8), Budget::default())
            .unwrap()
            .declarations()
            .len(),
        templates.len()
    );
    add(&mut index, 300, node(10, vec![ty(P::Bool)]));
    let candidates = index
        .candidates(
            Some(id(100)),
            &[],
            &node(10, vec![var(9)]),
            Budget::default(),
        )
        .unwrap();
    assert!(candidates.declarations().contains(&id(300)));
}

#[test]
fn higher_pending_and_same_level_competitors_block_commit() {
    let mut index = Index::default();
    add(&mut index, 200, var(1));
    add(&mut index, 201, node(10, vec![var(1)]));
    add(&mut index, 202, node(10, vec![ty(P::Bool)]));
    let candidates = index
        .candidates(
            Some(id(100)),
            &[],
            &node(10, vec![ty(P::Bool)]),
            Budget::default(),
        )
        .unwrap();
    let mut visited = vec![];
    assert_eq!(
        index.select(&candidates, |h| {
            visited.push(h.declaration);
            if h.declaration == id(202) {
                A::Pending
            } else {
                A::Applicable
            }
        }),
        Selection::Deferred {
            applicable: vec![],
            pending: vec![id(202)]
        }
    );
    assert_eq!(visited, vec![id(202)]);
    assert_eq!(
        index.select(&candidates, |h| if h.declaration == id(202) {
            A::Excluded
        } else {
            A::Applicable
        }),
        Selection::Selected(id(201))
    );
    add(&mut index, 203, node(10, vec![var(2)]));
    assert_eq!(
        index.select(&candidates, |_| A::Applicable),
        Selection::StaleIndex
    );
    let candidates = index
        .candidates(
            Some(id(100)),
            &[],
            &node(10, vec![ty(P::Char)]),
            Budget::default(),
        )
        .unwrap();
    assert_eq!(
        index.select(&candidates, |_| A::Applicable),
        Selection::Conflict(vec![id(201), id(203)])
    );
    assert_eq!(
        index.select(&candidates, |h| if h.declaration == id(203) {
            A::Pending
        } else {
            A::Applicable
        }),
        Selection::Deferred {
            applicable: vec![id(201)],
            pending: vec![id(203)]
        }
    );
}

#[test]
fn interface_buckets_include_open_heads_and_do_not_conflate_unknown_instances() {
    let mut index = Index::default();
    add(&mut index, 200, var(1));
    let mut other = head(201, var(1));
    other.interface = Some(id(101));
    index.insert(other, Budget::default()).unwrap();
    let mut open = head(202, var(1));
    open.interface = None;
    index.insert(open, Budget::default()).unwrap();
    let candidates = index
        .candidates(Some(id(100)), &[], &ty(P::Bool), Budget::default())
        .unwrap();
    assert_eq!(candidates.declarations(), &[id(200), id(202)]);
    let open = index
        .candidates(None, &[], &ty(P::Bool), Budget::default())
        .unwrap();
    assert_eq!(open.declarations().len(), 3);
    assert_eq!(
        index.select(&open, |_| panic!("open interface must not select")),
        Selection::UnresolvedInterface
    );
}

#[test]
fn trait_arguments_have_independent_candidates_and_do_not_raise_subject_precision() {
    let mut index = Index::default();
    for (n, arg) in [(200, var(1)), (201, ty(P::Bool)), (202, ty(P::Char))] {
        let mut h = head(n, node(10, vec![]));
        h.interface_arguments = vec![arg];
        index.insert(h, Budget::default()).unwrap();
    }
    let query = index
        .candidates(
            Some(id(100)),
            &[var(8)],
            &node(10, vec![]),
            Budget::default(),
        )
        .unwrap();
    assert_eq!(query.declarations().len(), 3);
    assert_eq!(
        index.select(&query, |_| A::Applicable),
        Selection::UnresolvedInterface
    );
    let query = index
        .candidates(
            Some(id(100)),
            &[ty(P::Bool)],
            &node(10, vec![]),
            Budget::default(),
        )
        .unwrap();
    assert_eq!(
        index.select(&query, |_| A::Applicable),
        Selection::Conflict(vec![id(200), id(201)])
    );
}

#[test]
fn opaque_heads_do_not_become_invertible_nominal_constructors() {
    let opaque = |arg| Term::NormalForm {
        head: id(10),
        sort: Sort::Type,
        arguments: vec![arg],
    };
    assert_eq!(
        Key::from_type(&opaque(var(1)), Budget::default()).unwrap(),
        Key::Wildcard
    );
    let key = Key::from_type(&opaque(ty(P::Bool)), Budget::default()).unwrap();
    assert!(matches!(key, Key::Leaf(Term::NormalForm { .. })));
    let mut index = Index::default();
    add(&mut index, 200, opaque(ty(P::Bool)));
    assert_eq!(
        index
            .candidates(Some(id(100)), &[], &ty(P::Char), Budget::default())
            .unwrap()
            .declarations(),
        &[id(200)]
    );
}

#[test]
fn failed_insert_is_atomic_and_foreign_world_snapshots_are_rejected() {
    let mut index = Index::default();
    add(&mut index, 200, var(1));
    let revision = index.revision();
    assert_eq!(
        index.insert(head(200, ty(P::Bool)), Budget::default()),
        Err(Error::DuplicateDeclaration(id(200)))
    );
    assert_eq!(
        index.insert(
            head(201, ty(P::Bool)),
            Budget {
                steps: 0,
                max_depth: 128
            }
        ),
        Err(Error::BudgetExhausted)
    );
    assert_eq!(index.revision(), revision);
    let candidates = index
        .candidates(Some(id(100)), &[], &ty(P::Bool), Budget::default())
        .unwrap();
    let mut other = Index::default();
    add(&mut other, 200, var(1));
    assert_eq!(
        other.select(&candidates, |_| A::Applicable),
        Selection::StaleIndex
    );
    assert!(
        index
            .candidates(
                Some(id(100)),
                &[],
                &ty(P::Bool),
                Budget {
                    steps: 1,
                    max_depth: 128
                }
            )
            .is_err()
    );
    assert!(
        index
            .to_string()
            .starts_with("(implementation-index (revision 1)")
    );
}

#[test]
fn declaration_order_does_not_resolve_conflicts_or_choose_a_winner() {
    let mut forward = Index::default();
    let mut reverse = Index::default();
    for n in [200, 201, 202] {
        add(&mut forward, n, node(10, vec![var(1)]));
    }
    for n in [202, 201, 200] {
        add(&mut reverse, n, node(10, vec![var(1)]));
    }
    assert_eq!(forward.to_string(), reverse.to_string());
    let a = forward
        .candidates(
            Some(id(100)),
            &[],
            &node(10, vec![ty(P::Bool)]),
            Budget::default(),
        )
        .unwrap();
    let b = reverse
        .candidates(
            Some(id(100)),
            &[],
            &node(10, vec![ty(P::Bool)]),
            Budget::default(),
        )
        .unwrap();
    assert_eq!(a.to_string(), b.to_string());
    assert_eq!(
        forward.select(&a, |_| A::Applicable),
        reverse.select(&b, |_| A::Applicable)
    );
    assert_eq!(
        forward.select(&a, |_| A::Excluded),
        Selection::NoApplicableCandidate
    );
    assert!(
        forward
            .select(&a, |_| A::Pending)
            .to_string()
            .starts_with("(deferred (applicable) (pending")
    );
}

#[test]
fn total_argument_budget_and_deep_heads_fail_without_publication() {
    let mut index = Index::default();
    let mut h = head(200, ty(P::Bool));
    h.interface_arguments = vec![ty(P::Bool); 8];
    assert_eq!(
        index.insert(
            h,
            Budget {
                steps: 5,
                max_depth: 128
            }
        ),
        Err(Error::BudgetExhausted)
    );
    assert_eq!(index.revision(), 0);
    let mut term = ty(P::Bool);
    for _ in 0..150 {
        term = Term::Pointer(Box::new(term));
    }
    assert_eq!(
        index.insert(head(201, term), Budget::default()),
        Err(Error::BudgetExhausted)
    );
    assert_eq!(index.revision(), 0);
}

fn generic(n: u32, subject: Term, parameters: Vec<Id>) -> ImplementationHead {
    let mut h = head(n, subject);
    h.type_parameters = parameters;
    h
}
fn context() -> elaboration::ptc::Context {
    let mut context = elaboration::ptc::Context::default();
    for n in [10, 11, 12] {
        context.admit_nominal_constructor(id(n));
    }
    context
}

#[test]
fn fixed_scalar_indices_select_infer_and_replay_without_value_equations() {
    use elaboration::implementation::{PairAssessment, TypeAssessment};
    for (px, em) in [
        (Term::Symbol("px".into()), Term::Symbol("em".into())),
        (Term::Bool(true), Term::Bool(false)),
        (Term::Char('a'), Term::Char('b')),
        (Term::String("one".into()), Term::String("two".into())),
    ] {
        let mut h = generic(200, ty(P::Integer), vec![id(1)]);
        h.interface_arguments = vec![px.clone(), var(1)];
        let context = context();
        let inputs = [px.clone(), ty(P::U32)];
        let assessment = h.assess_types(&ty(P::Integer), &inputs, &context, Budget::default());
        let TypeAssessment::Applicable(evidence) = assessment else {
            panic!("{assessment}")
        };
        assert_eq!(evidence.type_arguments, vec![ty(P::U32)]);
        assert_eq!(evidence.head_proofs.len(), 2);
        h.verify_type_evidence(
            &evidence,
            &ty(P::Integer),
            &inputs,
            &context,
            Budget::default(),
        )
        .unwrap();
        let wrong = [em.clone(), ty(P::U32)];
        assert!(
            h.verify_type_evidence(
                &evidence,
                &ty(P::Integer),
                &wrong,
                &context,
                Budget::default()
            )
            .is_err()
        );
        let mismatch = h.assess_types(&ty(P::Integer), &wrong, &context, Budget::default());
        assert!(
            matches!(
                mismatch,
                TypeAssessment::ValueIndexMismatch { position: 0, .. }
            ),
            "{mismatch}"
        );
        assert_eq!(
            mismatch.applicability(),
            elaboration::implementation::Applicability::Excluded
        );
        assert!(mismatch.to_string().contains("(value-index-mismatch"));
        let mut other = h.clone();
        other.declaration = id(201);
        other.interface_arguments[0] = em;
        let PairAssessment::Disjoint(proof) =
            h.assess_type_overlap(&other, &context, Budget::default())
        else {
            panic!("distinct suffixes must be disjoint")
        };
        assert!(h.verify_disjointness(&other, &proof, &context, Budget::default()));
        other.interface_arguments[0] = px;
        assert!(!h.verify_disjointness(&other, &proof, &context, Budget::default()));
        assert!(
            !h.assess_type_overlap(&other, &context, Budget::default())
                .is_proved()
        );
        assert!(matches!(
            h.assess_types(
                &ty(P::Integer),
                &inputs,
                &context,
                Budget {
                    steps: 1,
                    ..Budget::default()
                }
            ),
            TypeAssessment::Pending(_)
        ));
        assert!(matches!(
            h.assess_types(&ty(P::Integer), &inputs, &context, Budget::default()),
            TypeAssessment::Applicable(_)
        ));
        let wide = Term::Symbol("x".repeat(10000).into());
        h.interface_arguments[0] = wide.clone();
        assert!(matches!(
            h.assess_types(
                &ty(P::Integer),
                &[wide, ty(P::U32)],
                &context,
                Budget {
                    steps: 1000,
                    ..Budget::default()
                }
            ),
            TypeAssessment::Pending(_)
        ));
    }
}

#[test]
fn real_candidate_matching_solves_subject_and_trait_arguments_together() {
    use elaboration::implementation::TypeAssessment;
    let mut h = generic(200, node(11, vec![var(1), var(2)]), vec![id(1), id(2)]);
    h.interface_arguments = vec![var(2), var(1)];
    let context = context();
    let assessment = h.assess_types(
        &node(11, vec![ty(P::Bool), ty(P::Char)]),
        &[ty(P::Char), ty(P::Bool)],
        &context,
        Budget::default(),
    );
    let TypeAssessment::Applicable(evidence) = assessment else {
        panic!("{assessment}");
    };
    assert_eq!(evidence.type_arguments, vec![ty(P::Bool), ty(P::Char)]);
    for proof in evidence.head_proofs {
        context.verify_proof(&proof, Budget::default()).unwrap();
    }
    let bad = h.assess_types(
        &node(11, vec![ty(P::Bool), ty(P::Char)]),
        &[ty(P::Bool), ty(P::Bool)],
        &context,
        Budget::default(),
    );
    let TypeAssessment::Excluded(proof) = bad else {
        panic!("{bad}");
    };
    context.verify_proof(&proof, Budget::default()).unwrap();
}

#[test]
fn requires_are_proved_not_assumed_and_countermodels_do_not_exclude_open_requirements() {
    use elaboration::implementation::TypeAssessment;
    use nir::program::TypePredicate as R;
    let mut h = generic(200, node(10, vec![var(1)]), vec![id(1)]);
    h.requirements.push(R::Equal(var(1), ty(P::Bool)));
    let mut context = context();
    let yes = h.assess_types(
        &node(10, vec![ty(P::Bool)]),
        &[],
        &context,
        Budget::default(),
    );
    let TypeAssessment::Applicable(evidence) = yes else {
        panic!("{yes}");
    };
    assert_eq!(evidence.requirement_proofs.len(), 1);
    context
        .verify_proof(&evidence.requirement_proofs[0], Budget::default())
        .unwrap();
    let no = h.assess_types(
        &node(10, vec![ty(P::Char)]),
        &[],
        &context,
        Budget::default(),
    );
    let TypeAssessment::Excluded(proof) = no else {
        panic!("{no}");
    };
    context.verify_proof(&proof, Budget::default()).unwrap();
    assert!(matches!(
        h.assess_types(&node(10, vec![var(90)]), &[], &context, Budget::default()),
        TypeAssessment::Pending(_)
    ));
    context.assume(elaboration::ptc::Formula::equal(var(90), ty(P::Bool)));
    assert!(matches!(
        h.assess_types(&node(10, vec![var(90)]), &[], &context, Budget::default()),
        TypeAssessment::Applicable(_)
    ));
    assert!(matches!(
        h.assess_types(
            &node(10, vec![var(90)]),
            &[],
            &self::context(),
            Budget::default()
        ),
        TypeAssessment::Pending(_)
    ));
}

#[test]
fn repeated_parameters_are_real_equalities_and_do_not_escape_failed_candidates() {
    use elaboration::implementation::TypeAssessment;
    let repeated = generic(200, node(11, vec![var(1), var(1)]), vec![id(1)]);
    let separate = generic(201, node(11, vec![var(1), var(2)]), vec![id(1), id(2)]);
    let context = context();
    let query = node(11, vec![ty(P::Bool), ty(P::Char)]);
    assert!(matches!(
        repeated.assess_types(&query, &[], &context, Budget::default()),
        TypeAssessment::Excluded(_)
    ));
    let good = separate.assess_types(&query, &[], &context, Budget::default());
    let TypeAssessment::Applicable(evidence) = good else {
        panic!("{good}");
    };
    assert_eq!(evidence.type_arguments, vec![ty(P::Bool), ty(P::Char)]);
    assert!(matches!(
        repeated.assess_types(
            &node(11, vec![ty(P::Bool), ty(P::Bool)]),
            &[],
            &context,
            Budget::default()
        ),
        TypeAssessment::Applicable(_)
    ));
}

#[test]
fn blanket_user_implementations_check_final_subject_eligibility() {
    use elaboration::implementation::TypeAssessment;
    let h = generic(200, var(1), vec![id(1)]);
    let context = context();
    let closure = Term::Closure {
        environment: vec![ty(P::Bool)],
        signature: Box::new(Term::Function {
            parameters: vec![],
            result: Box::new(ty(P::Bool)),
        }),
    };
    assert!(matches!(
        h.assess_types(&closure, &[], &context, Budget::default()),
        TypeAssessment::ForbiddenSubject(_)
    ));
    assert!(matches!(
        h.assess_types(&var(90), &[], &context, Budget::default()),
        TypeAssessment::Pending(_)
    ));
    assert!(matches!(
        h.assess_types(&node(10, vec![closure]), &[], &context, Budget::default()),
        TypeAssessment::Applicable(_)
    ));
    let mut known = context.fork();
    known.assume(elaboration::ptc::Formula::equal(
        var(90),
        node(10, vec![ty(P::Bool)]),
    ));
    let good = h.assess_types(&var(90), &[], &known, Budget::default());
    let TypeAssessment::Applicable(evidence) = good else {
        panic!("{good}");
    };
    known
        .verify_proof(
            evidence.eligibility_proof.as_ref().unwrap(),
            Budget::default(),
        )
        .unwrap();
}

#[test]
fn tuple_kind_conflict_excludes_uninferable_generic_candidates() {
    use elaboration::{implementation::TypeAssessment, ptc::Context};
    use nir::normal_forms::{Head, Kind, Parameter, Role};
    let mut package = nir::program::Package {
        normal_forms: vec![Head {
            declaration: id(990),
            role: Role::Tuple,
            requirements: vec![],
            parameters: vec![Parameter {
                declaration: id(991),
                binding: nir::program::LocalId {
                    owner: 990,
                    index: 0,
                },
                kind: Kind::TypeList,
                ty: node(992, vec![ty(P::Type)]),
            }],
        }],
        ..Default::default()
    };
    let tuple = |field| Term::NormalForm {
        head: id(990),
        sort: Sort::Type,
        arguments: vec![Term::TypeListValue(vec![field])],
    };
    let mut trusted = Context::from_package(&package).unwrap();
    trusted.admit_nominal_constructor(id(10));
    let candidates = [
        (
            generic(200, node(10, vec![var(1)]), vec![id(1)]),
            tuple(ty(P::Bool)),
        ),
        (
            generic(201, tuple(var(1)), vec![id(1)]),
            node(10, vec![ty(P::Bool)]),
        ),
        (
            generic(202, Term::Pointer(Box::new(var(1))), vec![id(1)]),
            tuple(ty(P::Bool)),
        ),
        (
            generic(203, tuple(var(1)), vec![id(1)]),
            Term::Pointer(Box::new(ty(P::Bool))),
        ),
    ];
    package.normal_forms[0].role = Role::Opaque;
    let mut opaque = Context::from_package(&package).unwrap();
    opaque.admit_nominal_constructor(id(10));
    for (candidate, subject) in candidates {
        let TypeAssessment::Excluded(proof) =
            candidate.assess_types(&subject, &[], &trusted, Budget::default())
        else {
            panic!("kind conflict must not wait for generic arguments");
        };
        trusted.verify_proof(&proof, Budget::default()).unwrap();
        assert!(opaque.verify_proof(&proof, Budget::default()).is_err());
        assert!(matches!(
            candidate.assess_types(&subject, &[], &opaque, Budget::default()),
            TypeAssessment::Pending(_)
        ));
        assert!(matches!(
            candidate.assess_types(
                &subject,
                &[],
                &trusted,
                Budget {
                    steps: 0,
                    ..Budget::default()
                }
            ),
            TypeAssessment::Pending(_)
        ));
        assert!(matches!(
            candidate.assess_types(&subject, &[], &trusted, Budget::default()),
            TypeAssessment::Excluded(_)
        ));
    }
}

#[test]
fn tuple_subject_eligibility_requires_trusted_metadata_at_selection_and_replay() {
    use elaboration::{implementation::TypeAssessment, ptc::Context};
    use nir::normal_forms::{Head, Kind, Parameter, Role};
    let mut package = nir::program::Package {
        normal_forms: vec![Head {
            declaration: id(990),
            role: Role::Tuple,
            requirements: vec![],
            parameters: vec![Parameter {
                declaration: id(991),
                binding: nir::program::LocalId {
                    owner: 990,
                    index: 0,
                },
                kind: Kind::TypeList,
                ty: node(992, vec![ty(P::Type)]),
            }],
        }],
        ..Default::default()
    };
    let subject = Term::NormalForm {
        head: id(990),
        sort: Sort::Type,
        arguments: vec![Term::TypeListValue(vec![ty(P::Bool)])],
    };
    let implementation = generic(200, var(1), vec![id(1)]);
    let trusted = Context::from_package(&package).unwrap();
    let TypeAssessment::Applicable(evidence) =
        implementation.assess_types(&subject, &[], &trusted, Budget::default())
    else {
        panic!("bound Tuple must be an eligible subject");
    };
    for context in [
        trusted.fork(),
        trusted.theory_context(),
        trusted.theory_in_scope(vec![]),
    ] {
        implementation
            .verify_type_evidence(&evidence, &subject, &[], &context, Budget::default())
            .unwrap();
    }
    assert!(trusted.to_string().contains("(tuple-heads"));
    package.normal_forms[0].role = Role::Opaque;
    let opaque = Context::from_package(&package).unwrap();
    assert!(matches!(
        implementation.assess_types(&subject, &[], &opaque, Budget::default()),
        TypeAssessment::Pending(_)
    ));
    assert!(
        implementation
            .verify_type_evidence(&evidence, &subject, &[], &opaque, Budget::default())
            .is_err()
    );
    let mut equal = trusted.fork();
    equal.assume(elaboration::ptc::Formula::equal(var(995), subject));
    let TypeAssessment::Applicable(evidence) =
        implementation.assess_types(&var(995), &[], &equal, Budget::default())
    else {
        panic!("equality-backed Tuple must be eligible");
    };
    assert!(evidence.eligibility_proof.is_some());
    implementation
        .verify_type_evidence(&evidence, &var(995), &[], &equal, Budget::default())
        .unwrap();
}

#[test]
fn real_type_resolution_waits_for_finer_contracts_then_selects_or_reports_conflict() {
    use nir::program::TypePredicate as R;
    let mut index = Index::default();
    index
        .insert(generic(200, var(1), vec![id(1)]), Budget::default())
        .unwrap();
    let mut precise = generic(201, node(10, vec![var(1)]), vec![id(1)]);
    precise.requirements.push(R::Equal(var(1), ty(P::Bool)));
    index.insert(precise, Budget::default()).unwrap();
    let context = context();
    let pending = index
        .resolve_types(
            id(100),
            &[],
            &node(10, vec![var(90)]),
            &context,
            Budget::default(),
        )
        .unwrap();
    assert_eq!(
        pending.selection,
        Selection::Deferred {
            applicable: vec![],
            pending: vec![id(201)]
        }
    );
    let yes = index
        .resolve_types(
            id(100),
            &[],
            &node(10, vec![ty(P::Bool)]),
            &context,
            Budget::default(),
        )
        .unwrap();
    assert_eq!(yes.selection, Selection::Selected(id(201)), "{yes}");
    let fallback = index
        .resolve_types(
            id(100),
            &[],
            &node(10, vec![ty(P::Char)]),
            &context,
            Budget::default(),
        )
        .unwrap();
    assert_eq!(
        fallback.selection,
        Selection::Selected(id(200)),
        "{fallback}"
    );
    index
        .insert(
            generic(202, node(10, vec![var(2)]), vec![id(2)]),
            Budget::default(),
        )
        .unwrap();
    let conflict = index
        .resolve_types(
            id(100),
            &[],
            &node(10, vec![ty(P::Bool)]),
            &context,
            Budget::default(),
        )
        .unwrap();
    assert_eq!(
        conflict.selection,
        Selection::Conflict(vec![id(201), id(202)]),
        "{conflict}"
    );
}

#[test]
fn unresolved_parameters_and_budget_exhaustion_are_not_applicability() {
    use elaboration::implementation::TypeAssessment;
    let unused = generic(200, ty(P::Bool), vec![id(1)]);
    assert!(matches!(
        unused.assess_types(&ty(P::Bool), &[], &context(), Budget::default()),
        TypeAssessment::Pending(_)
    ));
    let mut index = Index::default();
    index
        .insert(generic(201, var(1), vec![id(1)]), Budget::default())
        .unwrap();
    assert!(matches!(
        index
            .resolve_types(
                id(100),
                &[],
                &ty(P::Bool),
                &context(),
                Budget {
                    steps: 16,
                    max_depth: 128
                }
            )
            .unwrap()
            .selection,
        Selection::Deferred { .. }
    ));
}

#[test]
fn type_evidence_replay_checks_query_substitution_contracts_and_scope() {
    use elaboration::implementation::TypeAssessment;
    use nir::program::TypePredicate as R;
    let mut h = generic(200, node(10, vec![var(1)]), vec![id(1)]);
    h.requirements.push(R::Equal(var(1), ty(P::Bool)));
    let subject = node(10, vec![ty(P::Bool)]);
    let context = context();
    let TypeAssessment::Applicable(evidence) =
        h.assess_types(&subject, &[], &context, Budget::default())
    else {
        panic!("expected type evidence");
    };
    h.verify_type_evidence(&evidence, &subject, &[], &context, Budget::default())
        .unwrap();
    let mut changed = evidence.clone();
    changed.type_arguments[0] = ty(P::Char);
    assert!(
        h.verify_type_evidence(&changed, &subject, &[], &context, Budget::default())
            .is_err()
    );
    let mut changed = evidence.clone();
    changed.requirement_proofs.clear();
    assert!(
        h.verify_type_evidence(&changed, &subject, &[], &context, Budget::default())
            .is_err()
    );
    let mut changed = evidence.clone();
    changed.implementation = id(201);
    assert!(
        h.verify_type_evidence(&changed, &subject, &[], &context, Budget::default())
            .is_err()
    );
    assert!(
        h.verify_type_evidence(
            &evidence,
            &node(10, vec![ty(P::Char)]),
            &[],
            &context,
            Budget::default()
        )
        .is_err()
    );
    assert!(
        h.verify_type_evidence(
            &evidence,
            &subject,
            &[],
            &elaboration::ptc::Context::default(),
            Budget::default()
        )
        .is_err()
    );
    assert!(
        h.verify_type_evidence(
            &evidence,
            &subject,
            &[],
            &context,
            Budget {
                steps: 0,
                max_depth: 128
            }
        )
        .is_err()
    );
}

#[test]
fn opaque_factories_are_not_inverted_by_candidate_matching() {
    use elaboration::implementation::TypeAssessment;
    let opaque = |arg| Term::NormalForm {
        head: id(500),
        sort: Sort::Type,
        arguments: vec![arg],
    };
    let mut h = generic(200, node(10, vec![opaque(var(1))]), vec![id(1)]);
    let query = node(10, vec![opaque(ty(P::Bool))]);
    assert!(matches!(
        h.assess_types(&query, &[], &context(), Budget::default()),
        TypeAssessment::Pending(_)
    ));
    // An independent interface position may fix T; full original equality is
    // still proved, rather than extracting T from the opaque type factory.
    h.interface_arguments = vec![var(1)];
    let good = h.assess_types(&query, &[ty(P::Bool)], &context(), Budget::default());
    assert!(matches!(good, TypeAssessment::Applicable(_)), "{good}");
}

#[test]
fn completed_type_resolution_is_invalidated_by_world_changes() {
    let mut index = Index::default();
    index
        .insert(generic(200, var(1), vec![id(1)]), Budget::default())
        .unwrap();
    let result = index
        .resolve_types(id(100), &[], &ty(P::Bool), &context(), Budget::default())
        .unwrap();
    assert!(matches!(result.selection, Selection::Selected(_)));
    assert!(index.type_resolution_is_current(&result));
    assert!(!Index::default().type_resolution_is_current(&result));
    index
        .insert(generic(201, ty(P::Bool), vec![]), Budget::default())
        .unwrap();
    assert!(!index.type_resolution_is_current(&result));
    assert_eq!(
        index
            .resolve_types(id(100), &[], &ty(P::Bool), &context(), Budget::default())
            .unwrap()
            .selection,
        Selection::Selected(id(201))
    );
}

fn foreign_head(subject: Term, arguments: Vec<Term>) -> ImplementationHead {
    ImplementationHead {
        parameter_types: vec![],
        declaration: Id {
            package: 1,
            local: 200,
        },
        interface: Some(id(100)),
        interface_arguments: arguments,
        ..head(200, subject)
    }
}
fn local_nominal(arguments: Vec<Term>) -> Term {
    Term::Nominal {
        declaration: Id {
            package: 1,
            local: 10,
        },
        arguments,
    }
}

#[test]
fn ownership_observes_subject_then_arguments_and_nominal_preorder() {
    use elaboration::implementation::{Ownership, OwnershipError};
    let local = local_nominal(vec![var(1)]);
    let local_id = Id {
        package: 1,
        local: 10,
    };
    for (subject, args) in [
        (local.clone(), vec![var(2)]),
        (node(10, vec![local.clone(), var(2)]), vec![]),
        (ty(P::Bool), vec![ty(P::Char), local.clone(), var(2)]),
        (Term::Pointer(Box::new(local.clone())), vec![]),
    ] {
        assert_eq!(
            foreign_head(subject, args).ownership(Budget::default()),
            Ok(Ownership::LocalNominal(local_id))
        );
    }
    for (subject, args) in [
        (node(10, vec![var(1), local.clone()]), vec![]),
        (var(1), vec![local.clone()]),
        (ty(P::Bool), vec![var(1), local.clone()]),
    ] {
        assert_eq!(
            foreign_head(subject, args).ownership(Budget::default()),
            Err(Error::Ownership(OwnershipError::OpenBeforeLocalIdentity))
        );
    }
    assert_eq!(
        foreign_head(ty(P::Bool), vec![Term::Bool(true)]).ownership(Budget::default()),
        Err(Error::Ownership(OwnershipError::NoLocalIdentity))
    );
    assert!(matches!(
        head(200, var(1)).ownership(Budget::default()),
        Ok(Ownership::LocalInterface(_))
    ));
}

#[test]
fn ownership_cannot_be_granted_by_requires_or_factory_names() {
    use elaboration::implementation::OwnershipError;
    let mut impl_head = foreign_head(var(1), vec![]);
    impl_head
        .requirements
        .push(nir::program::TypePredicate::Equal(
            var(1),
            local_nominal(vec![]),
        ));
    assert_eq!(
        impl_head.ownership(Budget::default()),
        Err(Error::Ownership(OwnershipError::OpenBeforeLocalIdentity))
    );
    impl_head.subject = Term::NormalForm {
        head: impl_head.declaration,
        sort: Sort::Type,
        arguments: vec![local_nominal(vec![])],
    };
    assert_eq!(
        impl_head.ownership(Budget::default()),
        Err(Error::Ownership(OwnershipError::OpenBeforeLocalIdentity))
    );
}

#[test]
fn inherent_ownership_does_not_extend_to_foreign_wrappers() {
    use elaboration::implementation::{OwnershipError, inherent_ownership};
    let local = local_nominal(vec![var(1)]);
    assert!(inherent_ownership(&local, 1).is_ok());
    for subject in [node(10, vec![local.clone()]), ty(P::Bool), var(1)] {
        assert_eq!(
            inherent_ownership(&subject, 1),
            Err(Error::Ownership(
                OwnershipError::InherentRequiresLocalNominal
            ))
        );
    }
    assert!(inherent_ownership(&local, 0).is_err());
}

#[test]
fn ownership_failure_is_atomic_and_preserves_candidate_snapshots() {
    let mut index = Index::default();
    add(&mut index, 201, ty(P::Bool));
    let candidates = index
        .candidates(Some(id(100)), &[], &ty(P::Bool), Budget::default())
        .unwrap();
    let revision = index.revision();
    assert!(
        index
            .insert(foreign_head(var(1), vec![]), Budget::default())
            .is_err()
    );
    assert_eq!(index.revision(), revision);
    assert_eq!(
        index.select(&candidates, |_| A::Applicable),
        Selection::Selected(id(201))
    );
    index
        .insert(
            foreign_head(local_nominal(vec![]), vec![]),
            Budget::default(),
        )
        .unwrap();
    assert_eq!(
        index.select(&candidates, |_| A::Applicable),
        Selection::StaleIndex
    );
}

#[test]
fn ownership_traversal_is_bounded_without_publishing_partial_entries() {
    let mut subject = local_nominal(vec![]);
    for _ in 0..150 {
        subject = Term::Pointer(Box::new(subject));
    }
    let mut index = Index::default();
    assert_eq!(
        index.insert(foreign_head(subject, vec![]), Budget::default()),
        Err(Error::BudgetExhausted)
    );
    assert_eq!(index.revision(), 0);
    assert_eq!(
        foreign_head(local_nominal(vec![]), vec![]).ownership(Budget {
            steps: 0,
            ..Budget::default()
        }),
        Err(Error::BudgetExhausted)
    );
}

#[test]
fn generic_same_precision_coherence_proves_disjoint_requires_and_replays_proofs() {
    use elaboration::{implementation::PairAssessment, ptc::Context};
    use nir::program::TypePredicate;
    let mut context = Context::default();
    context.admit_nominal_constructor(id(10));
    let mut left = head(200, node(10, vec![var(1)]));
    left.type_parameters = vec![id(1)];
    left.requirements = vec![TypePredicate::Equal(var(1), ty(P::Bool))];
    let mut right = head(201, node(10, vec![var(2)]));
    right.type_parameters = vec![id(2)];
    right.requirements = vec![TypePredicate::Equal(var(2), ty(P::Char))];
    let PairAssessment::Disjoint(proof) =
        left.assess_type_overlap(&right, &context, Budget::default())
    else {
        panic!("expected disjoint generic ranges")
    };
    assert!(left.verify_disjointness(&right, &proof, &context, Budget::default()));
    right.requirements.clear();
    assert!(!left.verify_disjointness(&right, &proof, &context, Budget::default()));
    assert!(
        !left
            .assess_type_overlap(&right, &context, Budget::default())
            .is_proved()
    );
}

#[test]
fn overlap_alpha_renames_each_implementations_quantified_parameters() {
    use elaboration::{implementation::PairAssessment, ptc::Context};
    let mut left = head(200, Term::Tuple(vec![var(1), ty(P::Bool)]));
    left.type_parameters = vec![id(1)];
    let mut right = head(201, Term::Tuple(vec![ty(P::Char), var(1)]));
    right.type_parameters = vec![id(1)];
    let context = Context::default();
    let result = left.assess_type_overlap(&right, &context, Budget::default());
    let PairAssessment::NotDisjoint(witness) = result else {
        panic!("independent parameters overlap at (char,bool): {result}")
    };
    context.verify_witness(&witness, Budget::default()).unwrap();
}

#[test]
fn coherence_uses_full_interface_arguments_and_never_caller_assumptions() {
    use elaboration::{
        implementation::PairAssessment,
        ptc::{Context, Formula},
    };
    let mut context = Context::default();
    context.assume(Formula::False);
    let left = head(200, ty(P::Bool));
    let right = head(201, ty(P::Bool));
    assert!(matches!(
        left.assess_type_overlap(&right, &context, Budget::default()),
        PairAssessment::NotDisjoint(_)
    ));
    let mut left = left;
    let mut right = right;
    left.interface_arguments = vec![ty(P::Bool)];
    right.interface_arguments = vec![ty(P::Char)];
    assert!(matches!(
        left.assess_type_overlap(&right, &context, Budget::default()),
        PairAssessment::Disjoint(_)
    ));
}

#[test]
fn coherence_reports_are_bounded_and_world_specific() {
    use elaboration::{implementation::PairAssessment, ptc::Context};
    let context = Context::default();
    let mut index = Index::default();
    add(&mut index, 200, var(1));
    add(&mut index, 201, node(10, vec![var(1)]));
    let report = index.check_type_coherence(&context, Budget::default());
    assert!(report.is_proved());
    assert!(matches!(
        report.pairs[0].2,
        PairAssessment::OrderedPrecision { .. }
    ));
    assert!(index.type_coherence_is_current(&report));
    assert!(!Index::default().type_coherence_is_current(&report));
    assert!(
        !index
            .check_type_coherence(
                &context,
                Budget {
                    steps: 0,
                    ..Budget::default()
                }
            )
            .is_proved()
    );
    add(&mut index, 202, node(10, vec![var(2)]));
    assert!(!index.type_coherence_is_current(&report));
    assert!(
        !index
            .check_type_coherence(&context, Budget::default())
            .is_proved()
    );
    let mut open = Index::default();
    let mut pending = head(300, ty(P::Bool));
    pending.interface = None;
    open.insert(pending, Budget::default()).unwrap();
    assert!(
        !open
            .check_type_coherence(&context, Budget::default())
            .is_proved()
    );
}

#[test]
fn coherence_renaming_does_not_capture_free_rigid_identities() {
    use elaboration::{implementation::PairAssessment, ptc::Context};
    use nir::program::TypePredicate;
    let external = Term::Rigid {
        declaration: Id {
            package: u64::MAX,
            local: 0,
        },
        sort: Sort::Type,
    };
    let mut left = head(200, ty(P::Bool));
    left.type_parameters = vec![id(1)];
    left.requirements = vec![TypePredicate::Equal(var(1), ty(P::Char))];
    let mut right = head(201, ty(P::Bool));
    right.type_parameters.clear();
    right.requirements = vec![TypePredicate::Equal(external, ty(P::Bool))];
    assert!(matches!(
        left.assess_type_overlap(&right, &Context::default(), Budget::default()),
        PairAssessment::NotDisjoint(_)
    ));
}

#[test]
fn coherence_replay_rejects_missing_pairs_forged_proofs_and_stale_worlds() {
    use elaboration::{
        implementation::PairAssessment,
        ptc::{Context, Formula},
    };
    let context = Context::default();
    let mut index = Index::default();
    add(&mut index, 200, ty(P::Bool));
    add(&mut index, 201, ty(P::Char));
    let report = index.check_type_coherence(&context, Budget::default());
    assert!(index.verify_type_coherence(&report, &context, Budget::default()));
    let mut removed = report.clone();
    removed.pairs.clear();
    assert!(!index.verify_type_coherence(&removed, &context, Budget::default()));
    let mut forged = report.clone();
    let PairAssessment::Disjoint(proof) = &mut forged.pairs[0].2 else {
        panic!("expected proof")
    };
    proof.goal = Formula::True;
    assert!(!index.verify_type_coherence(&forged, &context, Budget::default()));
    add(&mut index, 202, ty(P::Bool));
    assert!(!index.verify_type_coherence(&report, &context, Budget::default()));
}

#[test]
fn typed_interface_queries_treat_rigid_arguments_as_stable_symbols() {
    use elaboration::ptc::{Context, Formula};
    let mut index = Index::default();
    let mut generic = head(200, node(10, vec![var(1)]));
    generic.type_parameters = vec![id(1)];
    generic.interface_arguments = vec![var(1)];
    index.insert(generic.clone(), Budget::default()).unwrap();
    let mut context = Context::default();
    context.admit_nominal_constructor(id(10));
    let subject = node(10, vec![var(8)]);
    let arguments = vec![var(8)];
    let result = index
        .resolve_types(id(100), &arguments, &subject, &context, Budget::default())
        .unwrap();
    assert_eq!(result.selection, Selection::Selected(id(200)));
    let elaboration::implementation::TypeAssessment::Applicable(evidence) =
        &result.assessments[&id(200)]
    else {
        panic!()
    };
    assert_eq!(evidence.type_arguments, arguments);
    generic
        .verify_type_evidence(evidence, &subject, &arguments, &context, Budget::default())
        .unwrap();
    let mut specific = head(201, node(10, vec![ty(P::Bool)]));
    specific.type_parameters.clear();
    specific.interface_arguments = vec![ty(P::Bool)];
    index.insert(specific, Budget::default()).unwrap();
    assert!(matches!(
        index
            .resolve_types(id(100), &arguments, &subject, &context, Budget::default())
            .unwrap()
            .selection,
        Selection::Deferred { .. }
    ));
    context.assume(Formula::equal(var(8), ty(P::Char)));
    assert_eq!(
        index
            .resolve_types(id(100), &arguments, &subject, &context, Budget::default())
            .unwrap()
            .selection,
        Selection::Selected(id(200))
    );
}

#[test]
fn tuple_value_identity_does_not_prune_potentially_equal_opaque_fields() {
    let mut index = Index::default();
    let opaque = |n| Term::NormalForm {
        head: id(n),
        sort: Sort::Type,
        arguments: vec![],
    };
    let subject = |field| node(10, vec![Term::TupleValue(vec![field, Term::Bool(true)])]);
    add(&mut index, 200, subject(opaque(30)));
    let candidates = index
        .candidates(Some(id(100)), &[], &subject(opaque(31)), Budget::default())
        .unwrap();
    assert!(candidates.declarations().contains(&id(200)));
    let candidates = index
        .candidates(Some(id(100)), &[], &subject(var(9)), Budget::default())
        .unwrap();
    assert!(candidates.declarations().contains(&id(200)));
}

#[test]
fn matched_heads_retain_substituted_trait_conditions_without_claiming_applicability() {
    use elaboration::{implementation::TypeAssessment, ptc::Context};
    use nir::program::{TraitRequirement, TypePredicate as Q};
    let mut h = head(20, var(1));
    h.type_parameters = vec![id(1)];
    h.requirements = vec![Q::Not(Box::new(Q::Trait(TraitRequirement {
        interface: id(101),
        subject: var(1),
        interface_arguments: vec![var(1)],
    })))];
    let context = Context::default();
    let matched = h
        .match_types(&ty(P::Bool), &[], &context, Budget::default())
        .unwrap();
    assert_eq!(matched.type_arguments(), &[ty(P::Bool)]);
    assert_eq!(
        matched.requirements(),
        &[Q::Not(Box::new(Q::Trait(TraitRequirement {
            interface: id(101),
            subject: ty(P::Bool),
            interface_arguments: vec![ty(P::Bool)]
        })))]
    );
    h.verify_head_match(&matched, &ty(P::Bool), &[], &context, Budget::default())
        .unwrap();
    assert!(matches!(
        h.assess_types(&ty(P::Bool), &[], &context, Budget::default()),
        TypeAssessment::Pending(_)
    ));
    let mut index = Index::default();
    index.insert(h.clone(), Budget::default()).unwrap();
    assert!(matches!(
        index
            .resolve_types(id(100), &[], &ty(P::Bool), &context, Budget::default())
            .unwrap()
            .selection,
        Selection::Deferred { .. }
    ));
    println!("HEAD_MATCH_DUMP={matched}");
}

#[test]
fn matched_heads_do_not_hide_a_definitely_false_requirement() {
    use elaboration::{implementation::TypeAssessment, ptc::Context};
    let mut h = head(20, var(1));
    h.type_parameters = vec![id(1)];
    h.requirements = vec![nir::program::TypePredicate::Bool(false)];
    let context = Context::default();
    let matched = h
        .match_types(&ty(P::Bool), &[], &context, Budget::default())
        .unwrap();
    h.verify_head_match(&matched, &ty(P::Bool), &[], &context, Budget::default())
        .unwrap();
    assert!(matches!(
        h.assess_types(&ty(P::Bool), &[], &context, Budget::default()),
        TypeAssessment::Excluded(_)
    ));
}

#[test]
fn matched_head_replay_checks_declaration_query_requirements_and_scope() {
    use elaboration::ptc::{Context, Formula};
    let mut h = head(20, var(1));
    h.type_parameters = vec![id(1)];
    h.interface_arguments = vec![var(1)];
    let mut context = Context::default();
    context.assume(Formula::equal(var(9), ty(P::Bool)));
    let matched = h
        .match_types(&ty(P::Bool), &[ty(P::Bool)], &context, Budget::default())
        .unwrap();
    h.verify_head_match(
        &matched,
        &ty(P::Bool),
        &[ty(P::Bool)],
        &context,
        Budget::default(),
    )
    .unwrap();
    assert!(
        h.verify_head_match(
            &matched,
            &ty(P::Char),
            &[ty(P::Bool)],
            &context,
            Budget::default()
        )
        .is_err()
    );
    assert!(
        h.verify_head_match(
            &matched,
            &ty(P::Bool),
            &[ty(P::Char)],
            &context,
            Budget::default()
        )
        .is_err()
    );
    assert!(
        h.verify_head_match(
            &matched,
            &ty(P::Bool),
            &[ty(P::Bool)],
            &Context::default(),
            Budget::default()
        )
        .is_err()
    );
    let mut changed = h.clone();
    changed.declaration = id(21);
    assert!(
        changed
            .verify_head_match(
                &matched,
                &ty(P::Bool),
                &[ty(P::Bool)],
                &context,
                Budget::default()
            )
            .is_err()
    );
    changed = h.clone();
    changed
        .requirements
        .push(nir::program::TypePredicate::Bool(true));
    assert!(
        changed
            .verify_head_match(
                &matched,
                &ty(P::Bool),
                &[ty(P::Bool)],
                &context,
                Budget::default()
            )
            .is_err()
    );
}

#[test]
fn matched_heads_keep_unresolved_inputs_and_exhaustion_out_of_the_success_path() {
    use elaboration::{implementation::TypeAssessment, ptc::Context};
    let mut h = head(20, var(1));
    let context = Context::default();
    assert!(matches!(
        h.match_types(&ty(P::Bool), &[], &context, Budget::default()),
        Err(TypeAssessment::Pending(_))
    ));
    h.type_parameters = vec![id(1)];
    assert!(
        h.match_types(
            &ty(P::Bool),
            &[],
            &context,
            Budget {
                steps: 1,
                ..Budget::default()
            }
        )
        .is_err()
    );
    let matched = h
        .match_types(&ty(P::Bool), &[], &context, Budget::default())
        .unwrap();
    assert!(
        h.verify_head_match(
            &matched,
            &ty(P::Bool),
            &[],
            &context,
            Budget {
                steps: 0,
                ..Budget::default()
            }
        )
        .is_err()
    );
    h.verify_head_match(&matched, &ty(P::Bool), &[], &context, Budget::default())
        .unwrap();
}

#[test]
fn record_labels_charge_opaque_index_keys_before_copying() {
    for record in [
        Term::Record(vec![("x".repeat(512), ty(P::Bool))]),
        Term::RecordValue(vec![("x".repeat(512), Term::Bool(true))]),
    ] {
        let term = Term::NormalForm {
            head: id(300),
            sort: Sort::Type,
            arguments: vec![record],
        };
        assert!(matches!(
            Key::from_type(
                &term,
                Budget {
                    steps: 128,
                    max_depth: 128
                }
            ),
            Err(Error::BudgetExhausted)
        ));
        assert!(Key::from_type(&term, Budget::default()).is_ok());
    }
}

#[test]
fn record_label_budget_does_not_publish_candidate_applicability() {
    use elaboration::implementation::TypeAssessment;
    let subject = Term::Record(vec![("x".repeat(512), ty(P::Bool))]);
    let candidate = generic(200, subject.clone(), vec![]);
    assert!(matches!(
        candidate.assess_types(
            &subject,
            &[],
            &context(),
            Budget {
                steps: 128,
                max_depth: 128
            }
        ),
        TypeAssessment::Pending(_)
    ));
    assert!(matches!(
        candidate.assess_types(&subject, &[], &context(), Budget::default()),
        TypeAssessment::Applicable(_)
    ));
}

#[test]
fn type_list_inputs_keep_potentially_equal_opaque_candidates() {
    let mut index = Index::default();
    let opaque = |n| Term::NormalForm {
        head: id(n),
        sort: Sort::Type,
        arguments: vec![],
    };
    let subject = |field| node(10, vec![Term::TypeListValue(vec![field])]);
    add(&mut index, 200, subject(opaque(30)));
    for query in [subject(opaque(31)), subject(var(9))] {
        let candidates = index
            .candidates(Some(id(100)), &[], &query, Budget::default())
            .unwrap();
        assert!(candidates.declarations().contains(&id(200)));
    }
}

#[test]
fn scalar_index_keys_charge_payloads_and_keep_exact_floats_closed() {
    let mut budget = comptime_runtime::Budget::default();
    let decimal = Term::Float(
        comptime_runtime::decimal::DecimalValue::parse_literal("1.25", &mut budget).unwrap(),
    );
    let key = Key::from_type(&decimal, Budget::default()).unwrap();
    assert_eq!(key, Key::Leaf(decimal));
    for value in [
        Term::String("s".repeat(300).into()),
        Term::Symbol("s".repeat(300).into()),
        Term::Integer(
            comptime_runtime::Integer::parse_literal(&"9".repeat(1000), &mut budget).unwrap(),
        ),
        Term::Float(
            comptime_runtime::decimal::DecimalValue::parse_literal(
                &format!("1.{}", "3".repeat(300)),
                &mut budget,
            )
            .unwrap(),
        ),
    ] {
        for term in [
            value.clone(),
            Term::NormalForm {
                head: id(44),
                sort: Sort::Type,
                arguments: vec![value],
            },
        ] {
            assert_eq!(
                Key::from_type(
                    &term,
                    Budget {
                        steps: 20,
                        ..Budget::default()
                    }
                ),
                Err(Error::BudgetExhausted)
            );
            assert!(Key::from_type(&term, Budget::default()).is_ok());
        }
    }
}

#[test]
fn candidate_comparisons_charge_repeated_scalar_and_label_payloads() {
    for record_subject in [false, true] {
        let mut index = Index::default();
        let payload = |i| format!("{}-{i}", "x".repeat(256));
        for i in 0..8 {
            let mut implementation = head(
                200 + i,
                if record_subject {
                    Term::Record(vec![(payload(i), ty(P::Bool))])
                } else {
                    ty(P::Bool)
                },
            );
            implementation.type_parameters.clear();
            if !record_subject {
                implementation.interface_arguments = vec![Term::Symbol(payload(i).into())];
            }
            index.insert(implementation, Budget::default()).unwrap();
        }
        let subject = if record_subject {
            Term::Record(vec![(payload(5), ty(P::Bool))])
        } else {
            ty(P::Bool)
        };
        let arguments = if record_subject {
            vec![]
        } else {
            vec![Term::Symbol(payload(5).into())]
        };
        let revision = index.revision();
        assert!(matches!(
            index.candidates(
                Some(id(100)),
                &arguments,
                &subject,
                Budget {
                    steps: 1000,
                    ..Budget::default()
                }
            ),
            Err(Error::BudgetExhausted)
        ));
        assert_eq!(index.revision(), revision);
        let candidates = index
            .candidates(Some(id(100)), &arguments, &subject, Budget::default())
            .unwrap();
        assert_eq!(candidates.declarations(), &[id(205)]);
    }
}

#[test]
fn nominal_impl_value_parameters_infer_domains_and_replay() {
    use elaboration::implementation::TypeAssessment;
    use elaboration::ptc::Context;
    let mut context = Context::default();
    context.admit_nominal_constructor(id(800));
    let mut h = head(
        801,
        node(
            800,
            vec![Term::Rigid {
                declaration: id(1),
                sort: Sort::ComptimeValue,
            }],
        ),
    );
    h.type_parameters = vec![id(1)];
    h.parameter_types = vec![ty(P::Bool)];
    for flag in [true, false] {
        let subject = node(800, vec![Term::Bool(flag)]);
        let result = h.assess_types(&subject, &[], &context, Budget::default());
        let TypeAssessment::Applicable(evidence) = result else {
            panic!("{result:?}")
        };
        assert_eq!(evidence.type_arguments, vec![Term::Bool(flag)]);
        h.verify_type_evidence(&evidence, &subject, &[], &context, Budget::default())
            .unwrap();
        let mut changed = h.clone();
        changed.parameter_types = vec![ty(P::Char)];
        assert!(
            changed
                .verify_type_evidence(&evidence, &subject, &[], &context, Budget::default())
                .is_err()
        );
        assert!(!matches!(
            changed.assess_types(&subject, &[], &context, Budget::default()),
            TypeAssessment::Applicable(_)
        ));
        let mut forged = evidence.clone();
        forged.type_arguments = vec![Term::Bool(!flag)];
        assert!(
            h.verify_type_evidence(&forged, &subject, &[], &context, Budget::default())
                .is_err()
        );
    }
}

#[test]
fn nominal_impl_value_inference_preserves_dependent_domains_and_repeated_indices() {
    use elaboration::implementation::TypeAssessment;
    use elaboration::ptc::Context;
    let mut context = Context::default();
    context.admit_nominal_constructor(id(800));
    let index = Term::Rigid {
        declaration: id(2),
        sort: Sort::ComptimeValue,
    };
    let mut h = head(801, node(800, vec![var(1), index.clone(), index]));
    h.parameter_types = vec![ty(P::Type), var(1)];
    let subject = node(800, vec![ty(P::Bool), Term::Bool(true), Term::Bool(true)]);
    let result = h.assess_types(&subject, &[], &context, Budget::default());
    let TypeAssessment::Applicable(evidence) = result else {
        panic!("{result:?}")
    };
    assert_eq!(evidence.type_arguments, vec![ty(P::Bool), Term::Bool(true)]);
    h.verify_type_evidence(&evidence, &subject, &[], &context, Budget::default())
        .unwrap();
    for invalid in [
        node(800, vec![ty(P::Char), Term::Bool(true), Term::Bool(true)]),
        node(800, vec![ty(P::Bool), Term::Bool(true), Term::Bool(false)]),
    ] {
        assert!(!matches!(
            h.assess_types(&invalid, &[], &context, Budget::default()),
            TypeAssessment::Applicable(_)
        ));
    }
    assert!(!matches!(
        h.assess_types(&subject, &[], &Context::default(), Budget::default()),
        TypeAssessment::Applicable(_)
    ));
    assert!(!matches!(
        h.assess_types(
            &subject,
            &[],
            &context,
            Budget {
                steps: 0,
                ..Budget::default()
            }
        ),
        TypeAssessment::Applicable(_)
    ));
}

#[test]
fn direct_trait_value_parameters_infer_without_value_propositions() {
    use elaboration::implementation::TypeAssessment;
    use elaboration::ptc::Context;
    let context = Context::default();
    let mut h = head(801, ty(P::Bool));
    h.type_parameters = vec![id(1)];
    h.parameter_types = vec![ty(P::Bool)];
    h.interface_arguments = vec![Term::Rigid {
        declaration: id(1),
        sort: Sort::ComptimeValue,
    }];
    for flag in [true, false] {
        let arguments = vec![Term::Bool(flag)];
        let result = h.assess_types(&ty(P::Bool), &arguments, &context, Budget::default());
        let TypeAssessment::Applicable(evidence) = result else {
            panic!("{result:?}")
        };
        assert_eq!(evidence.type_arguments, arguments);
        assert_eq!(evidence.head_proofs.len(), 1);
        h.verify_type_evidence(
            &evidence,
            &ty(P::Bool),
            &arguments,
            &context,
            Budget::default(),
        )
        .unwrap();
        assert!(
            h.verify_type_evidence(
                &evidence,
                &ty(P::Bool),
                &[Term::Bool(!flag)],
                &context,
                Budget::default()
            )
            .is_err()
        );
    }
    assert!(!matches!(
        h.assess_types(
            &ty(P::Bool),
            &[Term::Char('x')],
            &context,
            Budget::default()
        ),
        TypeAssessment::Applicable(_)
    ));
}

#[test]
fn interface_value_inference_checks_repeated_and_record_inputs() {
    use elaboration::implementation::TypeAssessment;
    use elaboration::ptc::Context;
    let context = Context::default();
    let mut h = head(801, ty(P::Bool));
    let value = Term::Rigid {
        declaration: id(2),
        sort: Sort::ComptimeValue,
    };
    h.parameter_types = vec![ty(P::Type), var(1)];
    let record = |a, b| Term::RecordValue(vec![("element".into(), a), ("flag".into(), b)]);
    h.interface_arguments = vec![record(var(1), value.clone()), value];
    let query = vec![record(ty(P::Bool), Term::Bool(true)), Term::Bool(true)];
    let result = h.assess_types(&ty(P::Bool), &query, &context, Budget::default());
    let TypeAssessment::Applicable(evidence) = result else {
        panic!("{result:?}")
    };
    assert_eq!(evidence.type_arguments, vec![ty(P::Bool), Term::Bool(true)]);
    h.verify_type_evidence(&evidence, &ty(P::Bool), &query, &context, Budget::default())
        .unwrap();
    let wrong = vec![query[0].clone(), Term::Bool(false)];
    assert!(!matches!(
        h.assess_types(&ty(P::Bool), &wrong, &context, Budget::default()),
        TypeAssessment::Applicable(_)
    ));
    assert!(
        h.verify_type_evidence(&evidence, &ty(P::Bool), &wrong, &context, Budget::default())
            .is_err()
    );
}

#[test]
fn whole_record_value_binders_replay_their_type_leaves() {
    use elaboration::implementation::TypeAssessment;
    use elaboration::ptc::Context;
    let context = Context::default();
    let mut h = head(801, ty(P::Bool));
    h.type_parameters = vec![id(1)];
    h.parameter_types = vec![Term::Record(vec![
        ("element".into(), ty(P::Type)),
        ("flag".into(), ty(P::Bool)),
    ])];
    let value = Term::Rigid {
        declaration: id(1),
        sort: Sort::ComptimeValue,
    };
    h.interface_arguments = vec![value.clone(), value];
    let actual = Term::RecordValue(vec![
        ("element".into(), ty(P::Char)),
        ("flag".into(), Term::Bool(true)),
    ]);
    let query = vec![actual.clone(), actual];
    let result = h.assess_types(&ty(P::Bool), &query, &context, Budget::default());
    let TypeAssessment::Applicable(evidence) = result else {
        panic!("{result:?}")
    };
    assert_eq!(evidence.head_proofs.len(), 3);
    h.verify_type_evidence(&evidence, &ty(P::Bool), &query, &context, Budget::default())
        .unwrap();
}

#[test]
fn unresolved_record_value_leaves_do_not_refute_candidates() {
    use elaboration::implementation::TypeAssessment;
    use elaboration::ptc::Context;
    let record = |value| Term::RecordValue(vec![("flag".into(), value)]);
    let mut h = head(801, ty(P::Bool));
    h.type_parameters.clear();
    h.interface_arguments = vec![record(Term::NormalForm {
        head: id(900),
        sort: Sort::ComptimeValue,
        arguments: vec![],
    })];
    assert!(matches!(
        h.assess_types(
            &ty(P::Bool),
            &[record(Term::Bool(true))],
            &Context::default(),
            Budget::default()
        ),
        TypeAssessment::Pending(_)
    ));
}

#[test]
fn partially_symbolic_record_indices_prove_and_replay_disjointness() {
    use elaboration::implementation::PairAssessment;
    use elaboration::ptc::Context;
    let context = Context::default();
    let record = |tag, leaf| {
        Term::RecordValue(vec![(
            "nested".into(),
            Term::RecordValue(vec![
                ("tag".into(), Term::Bool(tag)),
                ("value".into(), leaf),
            ]),
        )])
    };
    let mut left = head(801, ty(P::Bool));
    left.type_parameters = vec![id(1)];
    left.parameter_types = vec![ty(P::Bool)];
    let value = Term::Rigid {
        declaration: id(1),
        sort: Sort::ComptimeValue,
    };
    left.interface_arguments = vec![record(true, value.clone())];
    let mut right = left.clone();
    right.declaration = id(802);
    right.interface_arguments = vec![record(false, value.clone())];
    let PairAssessment::Disjoint(proof) =
        left.assess_type_overlap(&right, &context, Budget::default())
    else {
        panic!("known tag must separate open records")
    };
    assert!(left.verify_disjointness(&right, &proof, &context, Budget::default()));
    right.interface_arguments = vec![record(true, value)];
    assert!(!left.verify_disjointness(&right, &proof, &context, Budget::default()));
    assert!(
        !left
            .assess_type_overlap(&right, &context, Budget::default())
            .is_proved()
    );
    right.interface_arguments = vec![Term::Rigid {
        declaration: id(1),
        sort: Sort::ComptimeValue,
    }];
    assert!(
        !left
            .assess_type_overlap(&right, &context, Budget::default())
            .is_proved()
    );
    assert!(
        !left
            .assess_type_overlap(
                &right,
                &context,
                Budget {
                    steps: 0,
                    ..Budget::default()
                }
            )
            .is_proved()
    );
}

#[test]
fn disjoint_open_record_impls_select_and_infer_their_value_arguments() {
    use elaboration::implementation::TypeAssessment;
    use elaboration::ptc::Context;
    let context = Context::default();
    let record = |tag, value| {
        Term::RecordValue(vec![
            ("tag".into(), Term::Bool(tag)),
            ("value".into(), value),
        ])
    };
    let mut index = Index::default();
    for (declaration, tag) in [(801, true), (802, false)] {
        let mut h = head(declaration, ty(P::Bool));
        h.type_parameters = vec![id(1)];
        h.parameter_types = vec![ty(P::Bool)];
        h.interface_arguments = vec![record(
            tag,
            Term::Rigid {
                declaration: id(1),
                sort: Sort::ComptimeValue,
            },
        )];
        index.insert(h, Budget::default()).unwrap();
    }
    let coherence = index.check_type_coherence(&context, Budget::default());
    assert!(index.verify_type_coherence(&coherence, &context, Budget::default()));
    for (declaration, tag) in [(801, true), (802, false)] {
        for value in [true, false] {
            let resolution = index
                .resolve_types(
                    id(100),
                    &[record(tag, Term::Bool(value))],
                    &ty(P::Bool),
                    &context,
                    Budget::default(),
                )
                .unwrap();
            assert_eq!(resolution.selection, Selection::Selected(id(declaration)));
            let TypeAssessment::Applicable(evidence) = &resolution.assessments[&id(declaration)]
            else {
                panic!("missing selected evidence")
            };
            assert_eq!(evidence.type_arguments, vec![Term::Bool(value)]);
        }
    }
}

#[test]
fn external_domain_head_replay_preserves_pending_requirements() {
    use elaboration::ptc::Context;
    use nir::program::TypePredicate;
    let value = |n| Term::Rigid {
        declaration: id(n),
        sort: Sort::ComptimeValue,
    };
    let mut h = head(200, node(10, vec![value(1)]));
    h.type_parameters = vec![id(1)];
    h.parameter_types = vec![ty(P::Bool)];
    h.interface_arguments = vec![value(1)];
    h.requirements = vec![TypePredicate::Bool(false)];
    let subject = node(10, vec![value(2)]);
    let args = [value(2)];
    let domains = std::collections::BTreeMap::from([(id(2), ty(P::Bool))]);
    let context = Context::default();
    let matched = h
        .match_types_with_domains(&subject, &args, &context, Budget::default(), &domains)
        .unwrap();
    assert_eq!(matched.requirements(), &[TypePredicate::Bool(false)]);
    h.verify_head_match_with_domains(
        &matched,
        &subject,
        &args,
        &context,
        Budget::default(),
        &domains,
    )
    .unwrap();
    assert!(
        h.verify_head_match(&matched, &subject, &args, &context, Budget::default())
            .is_err()
    );
    let wrong = std::collections::BTreeMap::from([(id(2), ty(P::Char))]);
    assert!(
        h.verify_head_match_with_domains(
            &matched,
            &subject,
            &args,
            &context,
            Budget::default(),
            &wrong
        )
        .is_err()
    );
    assert!(
        h.verify_head_match_with_domains(
            &matched,
            &subject,
            &args,
            &context,
            Budget {
                steps: 0,
                ..Default::default()
            },
            &domains
        )
        .is_err()
    );
    let mut changed = h.clone();
    changed.requirements.clear();
    assert!(
        changed
            .verify_head_match_with_domains(
                &matched,
                &subject,
                &args,
                &context,
                Budget::default(),
                &domains
            )
            .is_err()
    );
    let mut index = Index::default();
    index.insert(h.clone(), Budget::default()).unwrap();
    index.register_parameter_domain(id(2), ty(P::Bool)).unwrap();
    assert!(!matches!(
        index
            .resolve_types(id(100), &args, &subject, &context, Budget::default())
            .unwrap()
            .selection,
        Selection::Selected(_)
    ));
    h.verify_head_match_with_domains(
        &matched,
        &subject,
        &args,
        &context,
        Budget::default(),
        &domains,
    )
    .unwrap();
}

#[test]
fn dependent_domain_equality_is_rechecked_during_proof_replay() {
    use elaboration::ptc::{Context, Formula};
    let value = |n| Term::Rigid {
        declaration: id(n),
        sort: Sort::ComptimeValue,
    };
    let mut h = head(200, ty(P::Bool));
    h.type_parameters = vec![id(1), id(2)];
    h.parameter_types = vec![ty(P::Type), var(1)];
    h.interface_arguments = vec![var(1), value(2)];
    let args = [var(3), value(5)];
    let domains = std::collections::BTreeMap::from([(id(5), var(4))]);
    let mut context = Context::default();
    assert!(
        h.match_types_with_domains(&ty(P::Bool), &args, &context, Budget::default(), &domains)
            .is_err()
    );
    context.assume(Formula::equal(var(3), var(4)));
    let matched = h
        .match_types_with_domains(&ty(P::Bool), &args, &context, Budget::default(), &domains)
        .unwrap();
    h.verify_head_match_with_domains(
        &matched,
        &ty(P::Bool),
        &args,
        &context,
        Budget::default(),
        &domains,
    )
    .unwrap();
    assert!(
        h.verify_head_match_with_domains(
            &matched,
            &ty(P::Bool),
            &args,
            &Context::default(),
            Budget::default(),
            &domains
        )
        .is_err()
    );
    let mut index = Index::default();
    index.insert(h.clone(), Budget::default()).unwrap();
    index.register_parameter_domain(id(5), var(4)).unwrap();
    let selected = index
        .resolve_types(id(100), &args, &ty(P::Bool), &context, Budget::default())
        .unwrap();
    let elaboration::implementation::TypeAssessment::Applicable(evidence) =
        &selected.assessments[&id(200)]
    else {
        panic!("{selected}")
    };
    h.verify_type_evidence_with_domains(
        evidence,
        &ty(P::Bool),
        &args,
        &context,
        Budget::default(),
        &domains,
    )
    .unwrap();
    assert!(
        h.verify_type_evidence_with_domains(
            evidence,
            &ty(P::Bool),
            &args,
            &Context::default(),
            Budget::default(),
            &domains
        )
        .is_err()
    );
    assert!(
        h.verify_type_evidence_with_domains(
            evidence,
            &ty(P::Bool),
            &args,
            &context,
            Budget {
                steps: 0,
                ..Default::default()
            },
            &domains
        )
        .is_err()
    );
    h.verify_type_evidence_with_domains(
        evidence,
        &ty(P::Bool),
        &args,
        &context,
        Budget::default(),
        &domains,
    )
    .unwrap();
}

#[test]
fn repeated_symbolic_value_inputs_remain_pending_in_both_orders() {
    use elaboration::{implementation::TypeAssessment, ptc::Context};
    let value = |n| Term::Rigid {
        declaration: id(n),
        sort: Sort::ComptimeValue,
    };
    let mut h = head(200, ty(P::Bool));
    h.type_parameters = vec![id(1)];
    h.parameter_types = vec![ty(P::Bool)];
    h.interface_arguments = vec![value(1), value(1)];
    let domains = std::collections::BTreeMap::from([(id(2), ty(P::Bool))]);
    for args in [[value(2), Term::Bool(true)], [Term::Bool(true), value(2)]] {
        assert!(
            matches!(
                h.match_types_with_domains(
                    &ty(P::Bool),
                    &args,
                    &Context::default(),
                    Budget::default(),
                    &domains
                ),
                Err(TypeAssessment::Pending(_))
            ),
            "{args:?}"
        );
    }
    assert!(matches!(
        h.match_types_with_domains(
            &ty(P::Bool),
            &[Term::Bool(true), Term::Bool(false)],
            &Context::default(),
            Budget::default(),
            &domains
        ),
        Err(TypeAssessment::ValueIndexMismatch { .. })
    ));
    let matched = h
        .match_types_with_domains(
            &ty(P::Bool),
            &[value(2), value(2)],
            &Context::default(),
            Budget::default(),
            &domains,
        )
        .unwrap();
    h.verify_head_match_with_domains(
        &matched,
        &ty(P::Bool),
        &[value(2), value(2)],
        &Context::default(),
        Budget::default(),
        &domains,
    )
    .unwrap();
}

#[test]
fn symbolic_value_shared_by_subject_and_interface_is_not_refuted() {
    use elaboration::{implementation::TypeAssessment, ptc::Context};
    let value = |n| Term::Rigid {
        declaration: id(n),
        sort: Sort::ComptimeValue,
    };
    let mut h = head(200, node(10, vec![value(1)]));
    h.type_parameters = vec![id(1)];
    h.parameter_types = vec![ty(P::Bool)];
    h.interface_arguments = vec![value(1)];
    let domains = std::collections::BTreeMap::from([(id(2), ty(P::Bool))]);
    let context = Context::default();
    for (subject, args) in [
        (node(10, vec![value(2)]), vec![Term::Bool(true)]),
        (node(10, vec![Term::Bool(true)]), vec![value(2)]),
    ] {
        assert!(matches!(
            h.match_types_with_domains(&subject, &args, &context, Budget::default(), &domains),
            Err(TypeAssessment::Pending(_))
        ));
    }
    let subject = node(10, vec![value(2)]);
    let args = [value(2)];
    let matched = h
        .match_types_with_domains(&subject, &args, &context, Budget::default(), &domains)
        .unwrap();
    h.verify_head_match_with_domains(
        &matched,
        &subject,
        &args,
        &context,
        Budget::default(),
        &domains,
    )
    .unwrap();
}

#[test]
fn whole_symbolic_record_indices_replay_and_keep_unknown_relations_pending() {
    use elaboration::{implementation::TypeAssessment, ptc::Context};
    let value = |n| Term::Rigid {
        declaration: id(n),
        sort: Sort::ComptimeValue,
    };
    let record = |flag| Term::RecordValue(vec![("flag".into(), flag)]);
    let mut h = head(200, ty(P::Bool));
    h.type_parameters = vec![id(1)];
    h.parameter_types = vec![Term::Record(vec![("flag".into(), ty(P::Bool))])];
    h.interface_arguments = vec![value(1), value(1)];
    let domains = std::collections::BTreeMap::from([(id(2), ty(P::Bool))]);
    let context = Context::default();
    let args = [record(value(2)), record(value(2))];
    let matched = h
        .match_types_with_domains(&ty(P::Bool), &args, &context, Budget::default(), &domains)
        .unwrap();
    h.verify_head_match_with_domains(
        &matched,
        &ty(P::Bool),
        &args,
        &context,
        Budget::default(),
        &domains,
    )
    .unwrap();
    assert!(
        h.verify_head_match(&matched, &ty(P::Bool), &args, &context, Budget::default())
            .is_err()
    );
    for args in [
        [record(value(2)), record(Term::Bool(true))],
        [record(Term::Bool(true)), record(value(2))],
    ] {
        assert!(matches!(
            h.match_types_with_domains(&ty(P::Bool), &args, &context, Budget::default(), &domains),
            Err(TypeAssessment::Pending(_))
        ));
    }
}

#[test]
fn known_record_tags_refute_repeated_inputs_despite_other_unknown_fields() {
    use elaboration::{implementation::TypeAssessment, ptc::Context};
    let value = |n| Term::Rigid {
        declaration: id(n),
        sort: Sort::ComptimeValue,
    };
    let record =
        |flag, tag| Term::RecordValue(vec![("flag".into(), flag), ("tag".into(), Term::Bool(tag))]);
    let mut h = head(200, ty(P::Bool));
    h.type_parameters = vec![id(1)];
    h.parameter_types = vec![Term::Record(vec![
        ("flag".into(), ty(P::Bool)),
        ("tag".into(), ty(P::Bool)),
    ])];
    h.interface_arguments = vec![value(1), value(1)];
    let domains = std::collections::BTreeMap::from([(id(2), ty(P::Bool))]);
    for args in [
        [record(value(2), true), record(Term::Bool(true), false)],
        [record(Term::Bool(true), false), record(value(2), true)],
    ] {
        assert!(matches!(
            h.match_types_with_domains(
                &ty(P::Bool),
                &args,
                &Context::default(),
                Budget::default(),
                &domains
            ),
            Err(TypeAssessment::ValueIndexMismatch { .. })
        ));
    }
}

#[test]
fn known_value_conflicts_are_not_hidden_by_earlier_unknown_inputs() {
    use elaboration::{implementation::TypeAssessment, ptc::Context};
    let value = Term::Rigid {
        declaration: id(2),
        sort: Sort::ComptimeValue,
    };
    let record = |a, z| Term::RecordValue(vec![("a".into(), a), ("z".into(), z)]);
    let domains = std::collections::BTreeMap::from([(id(2), ty(P::Bool))]);
    for (templates, arguments) in [
        (
            vec![record(Term::Bool(true), Term::Bool(false))],
            vec![record(value.clone(), Term::Bool(true))],
        ),
        (
            vec![Term::Bool(true), Term::Bool(false)],
            vec![value.clone(), Term::Bool(true)],
        ),
        (
            vec![Term::Bool(false), Term::Bool(true)],
            vec![Term::Bool(true), value.clone()],
        ),
    ] {
        let mut h = head(200, ty(P::Bool));
        h.type_parameters.clear();
        h.interface_arguments = templates;
        assert!(matches!(
            h.match_types_with_domains(
                &ty(P::Bool),
                &arguments,
                &Context::default(),
                Budget::default(),
                &domains
            ),
            Err(TypeAssessment::ValueIndexMismatch { .. })
        ));
    }
}

#[test]
fn nominal_value_subjects_require_layouts_for_selection_and_replay() {
    use elaboration::implementation::TypeAssessment;
    use elaboration::ptc::Context;
    let config = node(30, vec![]);
    let mut h = head(
        200,
        node(
            40,
            vec![Term::Rigid {
                declaration: id(1),
                sort: Sort::ComptimeValue,
            }],
        ),
    );
    h.type_parameters = vec![id(1)];
    h.parameter_types = vec![config.clone()];
    let value = Term::NominalValue {
        ty: Box::new(config),
        variant: None,
        fields: vec![Term::Bool(true)],
    };
    let subject = node(40, vec![value.clone()]);
    let mut context = Context::default();
    context.admit_nominal_constructor(id(40));
    let domains = Default::default();
    let mut package = nir::program::Package::default();
    package.structs.push(nir::program::Struct {
        declaration: id(30),
        name: "Config".into(),
        type_parameters: vec![],
        parameter_types: vec![],
        fields: vec![("flag".into(), ty(P::Bool))],
        requirements: vec![],
        capabilities: vec![],
    });
    assert!(
        h.match_types(&subject, &[], &context, Budget::default())
            .is_err()
    );
    let matched = h
        .match_types_with_layouts(
            &subject,
            &[],
            &context,
            Budget::default(),
            &domains,
            Some(&package),
        )
        .unwrap();
    h.verify_head_match_with_layouts(
        &matched,
        &subject,
        &[],
        &context,
        Budget::default(),
        &domains,
        Some(&package),
    )
    .unwrap();
    assert!(
        h.verify_head_match(&matched, &subject, &[], &context, Budget::default())
            .is_err()
    );
    let mut index = Index::default();
    index.insert(h.clone(), Budget::default()).unwrap();
    index.set_nominal_layouts(package.structs.clone(), vec![], vec![]);
    let selected = index
        .resolve_types(id(100), &[], &subject, &context, Budget::default())
        .unwrap();
    assert_eq!(selected.selection, Selection::Selected(id(200)));
    let TypeAssessment::Applicable(evidence) = &selected.assessments[&id(200)] else {
        panic!("{selected}")
    };
    h.verify_type_evidence_with_layouts(
        evidence,
        &subject,
        &[],
        &context,
        Budget::default(),
        &domains,
        Some(&package),
    )
    .unwrap();
    for fields in [vec![], vec![Term::Char('x')]] {
        let Term::NominalValue { ty, .. } = &value else {
            unreachable!()
        };
        let forged = node(
            40,
            vec![Term::NominalValue {
                ty: ty.clone(),
                variant: None,
                fields,
            }],
        );
        let result = index
            .resolve_types(id(100), &[], &forged, &context, Budget::default())
            .unwrap();
        assert!(!matches!(result.selection, Selection::Selected(_)));
    }
    index.set_nominal_layouts(package.structs.clone(), vec![], vec![]);
    assert!(index.type_resolution_is_current(&selected));
    package.structs[0].fields[0].1 = ty(P::Char);
    index.set_nominal_layouts(package.structs.clone(), vec![], vec![]);
    assert!(!index.type_resolution_is_current(&selected));
    assert!(
        h.verify_type_evidence_with_layouts(
            evidence,
            &subject,
            &[],
            &context,
            Budget::default(),
            &domains,
            Some(&package)
        )
        .is_err()
    );
}

#[test]
fn partially_symbolic_nominal_indices_prove_and_replay_disjointness() {
    use elaboration::implementation::PairAssessment;
    use elaboration::ptc::Context;
    let context = Context::default();
    let record = |tag, leaf| Term::NominalValue {
        ty: Box::new(node(900, vec![])),
        variant: Some(0),
        fields: vec![Term::TupleValue(vec![Term::Bool(tag), leaf])],
    };
    let mut left = head(801, ty(P::Bool));
    left.type_parameters = vec![id(1)];
    left.parameter_types = vec![ty(P::Bool)];
    let value = Term::Rigid {
        declaration: id(1),
        sort: Sort::ComptimeValue,
    };
    left.interface_arguments = vec![record(true, value.clone())];
    let mut right = left.clone();
    right.declaration = id(802);
    right.interface_arguments = vec![record(false, value.clone())];
    let PairAssessment::Disjoint(proof) =
        left.assess_type_overlap(&right, &context, Budget::default())
    else {
        panic!("known tag must separate open records")
    };
    assert!(left.verify_disjointness(&right, &proof, &context, Budget::default()));
    right.interface_arguments = vec![record(true, value)];
    assert!(!left.verify_disjointness(&right, &proof, &context, Budget::default()));
    assert!(
        !left
            .assess_type_overlap(&right, &context, Budget::default())
            .is_proved()
    );
    right.interface_arguments = vec![Term::Rigid {
        declaration: id(1),
        sort: Sort::ComptimeValue,
    }];
    assert!(
        !left
            .assess_type_overlap(&right, &context, Budget::default())
            .is_proved()
    );
    assert!(
        !left
            .assess_type_overlap(
                &right,
                &context,
                Budget {
                    steps: 0,
                    ..Budget::default()
                }
            )
            .is_proved()
    );
}

#[test]
fn disjoint_open_nominal_impls_select_and_infer_their_value_arguments() {
    use elaboration::implementation::TypeAssessment;
    use elaboration::ptc::Context;
    let mut context = Context::default();
    context.admit_nominal_constructor(id(900));
    let record = |tag, value| Term::NominalValue {
        ty: Box::new(node(900, vec![])),
        variant: None,
        fields: vec![Term::Bool(tag), value],
    };
    let mut index = Index::default();
    for (declaration, tag) in [(801, true), (802, false)] {
        let mut h = head(declaration, ty(P::Bool));
        h.type_parameters = vec![id(1)];
        h.parameter_types = vec![ty(P::Bool)];
        h.interface_arguments = vec![record(
            tag,
            Term::Rigid {
                declaration: id(1),
                sort: Sort::ComptimeValue,
            },
        )];
        index.insert(h, Budget::default()).unwrap();
    }
    let coherence = index.check_type_coherence(&context, Budget::default());
    assert!(index.verify_type_coherence(&coherence, &context, Budget::default()));
    for (declaration, tag) in [(801, true), (802, false)] {
        for value in [true, false] {
            let resolution = index
                .resolve_types(
                    id(100),
                    &[record(tag, Term::Bool(value))],
                    &ty(P::Bool),
                    &context,
                    Budget::default(),
                )
                .unwrap();
            assert_eq!(resolution.selection, Selection::Selected(id(declaration)));
            let TypeAssessment::Applicable(evidence) = &resolution.assessments[&id(declaration)]
            else {
                panic!("missing selected evidence")
            };
            assert_eq!(evidence.type_arguments, vec![Term::Bool(value)]);
        }
    }
}

#[test]
fn opaque_index_keys_see_open_inputs_inside_error_results() {
    let opaque = |argument| Term::NormalForm {
        head: id(91),
        sort: Sort::Type,
        arguments: vec![argument],
    };
    for argument in [
        Term::error_result(vec![var(1)], ty(P::Bool)),
        Term::error_result(vec![ty(P::Bool)], var(1)),
    ] {
        assert_eq!(
            Key::from_type(&opaque(argument), Budget::default()).unwrap(),
            Key::Wildcard
        );
    }
    assert_ne!(
        Key::from_type(
            &opaque(Term::error_result(vec![ty(P::Bool)], ty(P::Bool))),
            Budget::default()
        )
        .unwrap(),
        Key::Wildcard
    );
}

#[test]
fn explicit_error_prefix_excludes_uninferable_ordinary_candidates() {
    use elaboration::{implementation::TypeAssessment, ptc::Context};
    let mut context = Context::default();
    context.admit_nominal_constructor(id(10));
    let qualified = |result| Term::error_result(vec![ty(P::Bool)], result);
    let error_head = generic(200, qualified(var(1)), vec![id(1)]);
    for (candidate, subject) in [
        (error_head.clone(), node(10, vec![ty(P::Char)])),
        (
            generic(201, node(10, vec![var(1)]), vec![id(1)]),
            qualified(ty(P::Char)),
        ),
    ] {
        let TypeAssessment::Excluded(proof) =
            candidate.assess_types(&subject, &[], &context, Budget::default())
        else {
            panic!("known error prefix must not wait for an uninferred parameter");
        };
        context.verify_proof(&proof, Budget::default()).unwrap();
    }
    let subject = qualified(ty(P::Char));
    let TypeAssessment::Applicable(evidence) =
        error_head.assess_types(&subject, &[], &context, Budget::default())
    else {
        panic!("matching error subject must remain applicable");
    };
    error_head
        .verify_type_evidence(&evidence, &subject, &[], &context, Budget::default())
        .unwrap();
    assert_eq!(evidence.type_arguments, vec![ty(P::Char)]);
}

#[test]
fn coherence_proof_budget_is_not_diluted_by_unrelated_interfaces() {
    let mut index = Index::default();
    add(&mut index, 200, ty(P::Bool));
    add(&mut index, 201, ty(P::Char));
    for n in 0..30 {
        let mut unrelated = head(300 + n, ty(P::Bool));
        unrelated.interface = Some(id(500 + n));
        index.insert(unrelated, Budget::default()).unwrap();
    }
    let context = elaboration::ptc::Context::default();
    let budget = Budget {
        steps: 2000,
        ..Budget::default()
    };
    let report = index.check_type_coherence(&context, budget);
    assert_eq!(report.pairs.len(), 32 * 31 / 2);
    assert!(report.is_proved(), "{report}");
    assert!(index.verify_type_coherence(&report, &context, budget));
    let exhausted = Budget {
        steps: 500,
        ..budget
    };
    assert!(!index.check_type_coherence(&context, exhausted).is_proved());
    assert!(!index.verify_type_coherence(&report, &context, exhausted));
    add(&mut index, 202, ty(P::Bool));
    assert!(!index.check_type_coherence(&context, budget).is_proved());
    assert!(!index.verify_type_coherence(&report, &context, budget));
}

#[test]
fn optional_candidates_preserve_payload_inference_and_distinct_outer_heads() {
    use elaboration::ptc::Context;
    let optional = |t| Term::Optional(Box::new(t));
    let opaque = Term::NormalForm {
        head: id(500),
        sort: Sort::Type,
        arguments: vec![optional(var(1))],
    };
    assert_eq!(
        Key::from_type(&opaque, Budget::default())
            .unwrap()
            .precision(),
        0
    );
    let mut generic = head(200, optional(var(1)));
    generic.type_parameters = vec![id(1)];
    let concrete = optional(ty(P::Bool));
    let mut index = Index::default();
    index.insert(generic.clone(), Budget::default()).unwrap();
    add(&mut index, 201, Term::Pointer(Box::new(var(1))));
    let candidates = index
        .candidates(Some(id(100)), &[], &concrete, Budget::default())
        .unwrap();
    assert!(candidates.declarations().contains(&id(200)));
    assert!(!candidates.declarations().contains(&id(201)));
    let context = Context::default();
    let matched = generic
        .match_types(&concrete, &[], &context, Budget::default())
        .unwrap();
    generic
        .verify_head_match(&matched, &concrete, &[], &context, Budget::default())
        .unwrap();
    assert!(
        generic
            .verify_head_match(
                &matched,
                &optional(ty(P::Char)),
                &[],
                &context,
                Budget::default()
            )
            .is_err()
    );
    assert!(
        Key::from_type(&concrete, Budget::default())
            .unwrap()
            .to_string()
            .contains("optional")
    );
}

#[test]
fn optional_value_indices_infer_type_leaves_and_replay_presence() {
    use elaboration::{implementation::TypeAssessment, ptc::Context};
    let context = Context::default();
    let wrap = |value| Term::OptionalValue {
        ty: Box::new(Term::Optional(Box::new(ty(P::Type)))),
        value,
    };
    let mut h = head(801, ty(P::Bool));
    h.type_parameters = vec![id(1)];
    h.parameter_types = vec![ty(P::Type)];
    h.interface_arguments = vec![wrap(Some(Box::new(var(1))))];
    let query = vec![wrap(Some(Box::new(ty(P::Char))))];
    let result = h.assess_types(&ty(P::Bool), &query, &context, Budget::default());
    let TypeAssessment::Applicable(evidence) = result else {
        panic!("{result:?}")
    };
    assert_eq!(evidence.type_arguments, vec![ty(P::Char)]);
    h.verify_type_evidence(&evidence, &ty(P::Bool), &query, &context, Budget::default())
        .unwrap();
    let absent = vec![wrap(None)];
    assert!(!matches!(
        h.assess_types(&ty(P::Bool), &absent, &context, Budget::default()),
        TypeAssessment::Applicable(_)
    ));
    assert!(
        h.verify_type_evidence(
            &evidence,
            &ty(P::Bool),
            &absent,
            &context,
            Budget::default()
        )
        .is_err()
    );
    assert_eq!(
        Key::from_type(&wrap(Some(Box::new(var(1)))), Budget::default())
            .unwrap()
            .precision(),
        0
    );
}
