use nir::{DeclarationId, Primitive, Sort, Term};
use ptc::{Budget, Context, Formula as F, Outcome, TypeInputOutcome};
fn id(local: u32) -> DeclarationId {
    DeclarationId { package: 7, local }
}
fn var(local: u32) -> Term {
    Term::Rigid {
        declaration: id(local),
        sort: Sort::Type,
    }
}
fn head(arg: Term) -> Term {
    Term::NormalForm {
        head: id(100),
        sort: Sort::Type,
        arguments: vec![arg],
    }
}
fn boolean() -> Term {
    Term::Primitive(Primitive::Bool)
}

#[test]
fn opaque_models_supply_checked_symbolic_type_input_solutions() {
    let mut context = Context::default();
    context.admit_congruence(id(100));
    let constraint = F::And(vec![
        F::equal(var(0), head(var(1))),
        F::equal(var(1), var(9)),
    ]);
    let inputs = [id(0), id(1)];
    let outcome = context.search_type_inputs(&inputs, constraint.clone(), &mut Budget::default());
    let TypeInputOutcome::Unique(solution) = outcome else {
        panic!("{outcome}")
    };
    assert_eq!(solution.bindings[&id(0)].0, head(var(9)));
    assert_eq!(solution.bindings[&id(1)].0, var(9));
    let problem = context
        .type_input_context(&inputs, constraint, &mut Budget::default())
        .unwrap();
    problem
        .verify_witness(&solution.model, Budget::default())
        .unwrap();
    for (_, proof) in solution.bindings.values() {
        problem.verify_proof(proof, Budget::default()).unwrap();
    }
    assert!(!solution.model.applications.is_empty());
    assert!(context.assumptions().is_empty());
}

#[test]
fn aggregate_type_leaves_are_interpreted_without_freezing_ambient_inputs() {
    let mut context = Context::default();
    context.admit_congruence(id(100));
    let packet = |ty| {
        Term::RecordValue(vec![(
            "configuration".into(),
            Term::TupleValue(vec![Term::Bool(true), ty]),
        )])
    };
    let inputs = [id(0), id(1)];
    let constraint = F::And(vec![
        F::equal(var(0), head(packet(var(1)))),
        F::equal(var(1), var(9)),
    ]);
    let outcome = context.search_type_inputs(&inputs, constraint.clone(), &mut Budget::default());
    let TypeInputOutcome::Unique(solution) = outcome else {
        panic!("{outcome}")
    };
    assert_eq!(solution.bindings[&id(0)].0, head(packet(var(9))));
    assert_eq!(solution.bindings[&id(1)].0, var(9));
    let problem = context
        .type_input_context(&inputs, constraint, &mut Budget::default())
        .unwrap();
    problem
        .verify_witness(&solution.model, Budget::default())
        .unwrap();
    for (_, proof) in solution.bindings.values() {
        problem.verify_proof(proof, Budget::default()).unwrap();
    }
    let mut corrupt = solution.model.clone();
    corrupt.applications[0].arguments[0] = packet(var(9));
    assert!(problem.verify_witness(&corrupt, Budget::default()).is_err());
    assert!(context.assumptions().is_empty());
}

#[test]
fn aggregate_type_arguments_preserve_ambiguity_and_budget_recovery() {
    let mut context = Context::default();
    context.admit_congruence(id(100));
    let constraint = F::equal(var(0), head(Term::TupleValue(vec![var(1)])));
    for steps in [0, 1, 20] {
        assert!(matches!(
            context.search_type_inputs(
                &[id(0), id(1)],
                constraint.clone(),
                &mut Budget {
                    steps,
                    ..Budget::default()
                }
            ),
            TypeInputOutcome::Unknown(ptc::UnknownReason::BudgetExhausted)
        ));
    }
    let outcome =
        context.search_type_inputs(&[id(0), id(1)], constraint.clone(), &mut Budget::default());
    let TypeInputOutcome::Ambiguous {
        candidate,
        alternative,
        ..
    } = outcome
    else {
        panic!("{outcome}")
    };
    let problem = context
        .type_input_context(&[id(0), id(1)], constraint, &mut Budget::default())
        .unwrap();
    problem
        .verify_witness(&candidate, Budget::default())
        .unwrap();
    problem
        .verify_witness(&alternative, Budget::default())
        .unwrap();
    assert!(context.assumptions().is_empty());
}

#[test]
fn opaque_model_tables_are_checked_and_do_not_grant_injectivity() {
    let mut context = Context::default();
    context.admit_congruence(id(100));
    context.assume(F::equal(head(var(0)), head(var(1))));
    let outcome = context.prove(F::equal(var(0), var(1)), Budget::default());
    let Outcome::NotEntailed(witness) = outcome else {
        panic!("{outcome}")
    };
    context.verify_witness(&witness, Budget::default()).unwrap();
    assert_ne!(witness.types[&id(0)], witness.types[&id(1)]);
    let mut missing = witness.clone();
    missing.applications.clear();
    assert!(context.verify_witness(&missing, Budget::default()).is_err());
    let mut duplicate = witness.clone();
    duplicate
        .applications
        .push(duplicate.applications[0].clone());
    assert!(
        context
            .verify_witness(&duplicate, Budget::default())
            .is_err()
    );
    let mut changed = witness.clone();
    changed.applications[0].result = boolean();
    assert!(context.verify_witness(&changed, Budget::default()).is_err());
    let mut foreign = witness.clone();
    foreign.applications[0].head = id(101);
    assert!(context.verify_witness(&foreign, Budget::default()).is_err());
    let dump = Outcome::NotEntailed(witness).to_string();
    assert!(dump.contains("(applications (type-application (head"));
}

#[test]
fn nested_opaque_calls_replay_and_unadmitted_heads_remain_unknown() {
    let goal = F::equal(head(head(var(0))), boolean());
    assert!(matches!(
        Context::default().prove(goal.clone(), Budget::default()),
        Outcome::Unknown { .. }
    ));
    let mut context = Context::default();
    context.admit_congruence(id(100));
    let outcome = context.prove(goal, Budget::default());
    let Outcome::NotEntailed(witness) = outcome else {
        panic!("{outcome}")
    };
    context.verify_witness(&witness, Budget::default()).unwrap();
    assert_eq!(witness.applications.len(), 2);
}

#[test]
fn model_interpretations_obey_budgets_and_do_not_escape_failed_search() {
    let mut context = Context::default();
    context.admit_congruence(id(100));
    let original = context.clone();
    for steps in [0, 1, 10, 100, 1000, 100_000] {
        let outcome = context.prove(
            F::equal(head(var(0)), boolean()),
            Budget {
                steps,
                ..Budget::default()
            },
        );
        match outcome {
            Outcome::NotEntailed(witness) => {
                context.verify_witness(&witness, Budget::default()).unwrap();
                assert!(matches!(
                    context.verify_witness(
                        &witness,
                        Budget {
                            steps: 1,
                            ..Budget::default()
                        }
                    ),
                    Err(ptc::ProofError::BudgetExhausted)
                ));
            }
            Outcome::Unknown {
                reason: ptc::UnknownReason::BudgetExhausted,
                ..
            } => {}
            other => panic!("{steps}: {other}"),
        }
        assert_eq!(context, original);
    }
}

#[test]
fn cyclic_opaque_equations_have_finite_interpretations_not_recursive_types() {
    for wrapped in [false, true] {
        let mut context = Context::default();
        context.admit_congruence(id(100));
        let value = head(var(0));
        let value = if wrapped {
            Term::Pointer(Box::new(value))
        } else {
            value
        };
        context.assume(F::equal(var(0), value));
        let outcome = context.prove(F::False, Budget::default());
        let Outcome::NotEntailed(witness) = outcome else {
            panic!("{outcome}")
        };
        context.verify_witness(&witness, Budget::default()).unwrap();
        assert_eq!(witness.applications.len(), 1);
        assert_eq!(
            witness.applications[0].arguments,
            vec![witness.types[&id(0)].clone()]
        );
    }
}

#[test]
fn completed_value_arguments_keep_identity_in_symbolic_solutions() {
    use std::sync::Arc;
    let values = vec![
        Term::BitVec(
            comptime_runtime::bitvec::BitVec::from_words(9, &[0x155], &mut Default::default())
                .unwrap(),
        ),
        Term::Unit,
        Term::Bool(true),
        Term::Char('🦀'),
        Term::Integer(comptime_runtime::Integer::from_i128(123456789)),
        Term::String(Arc::from("a\n\"🦀")),
        Term::Symbol(Arc::from("payload")),
        Term::TupleValue(vec![]),
        Term::RecordValue(vec![]),
        Term::TupleValue(vec![
            Term::Bool(true),
            Term::RecordValue(vec![(
                "nested".into(),
                Term::TupleValue(vec![Term::Symbol(Arc::from("item")), Term::Char('x')]),
            )]),
        ]),
    ];
    for value in values {
        let mut context = Context::default();
        context.admit_congruence(id(100));
        let apply = |ty| Term::NormalForm {
            head: id(100),
            sort: Sort::Type,
            arguments: vec![ty, value.clone()],
        };
        let inputs = [id(0), id(1)];
        let constraint = F::And(vec![
            F::equal(var(0), apply(var(1))),
            F::equal(var(1), var(9)),
        ]);
        let outcome =
            context.search_type_inputs(&inputs, constraint.clone(), &mut Budget::default());
        let TypeInputOutcome::Unique(solution) = outcome else {
            panic!("{value}: {outcome}")
        };
        assert_eq!(solution.bindings[&id(0)].0, apply(var(9)));
        let problem = context
            .type_input_context(&inputs, constraint, &mut Budget::default())
            .unwrap();
        problem
            .verify_witness(&solution.model, Budget::default())
            .unwrap();
        for (_, proof) in solution.bindings.values() {
            problem.verify_proof(proof, Budget::default()).unwrap();
        }
        assert!(
            solution
                .model
                .applications
                .iter()
                .all(|entry| entry.arguments[1] == value)
        );
    }
}

#[test]
fn unsupported_value_domains_and_non_type_results_cannot_forge_a_model() {
    let mut context = Context::default();
    context.admit_congruence(id(100));
    let apply = |value| Term::NormalForm {
        head: id(100),
        sort: Sort::Type,
        arguments: vec![value],
    };
    for value in [
        Term::Float64(0),
        Term::Value { store: 1, index: 0 },
        Term::TupleValue(vec![Term::Float64(0)]),
        Term::RecordValue(vec![("pending".into(), Term::Value { store: 1, index: 0 })]),
        Term::Rigid {
            declaration: id(8),
            sort: Sort::ComptimeValue,
        },
    ] {
        let outcome = context.prove(F::equal(apply(value.clone()), boolean()), Budget::default());
        assert!(
            matches!(outcome, Outcome::Unknown { .. }),
            "{value}: {outcome}"
        );
    }
    let duplicate = Term::RecordValue(vec![
        ("duplicate".into(), Term::Bool(true)),
        ("duplicate".into(), Term::Bool(false)),
    ]);
    assert!(matches!(
        context.prove(F::equal(apply(duplicate), boolean()), Budget::default()),
        Outcome::IllFormed(_)
    ));
    let outcome = context.prove(
        F::equal(apply(Term::Bool(true)), boolean()),
        Budget::default(),
    );
    let Outcome::NotEntailed(witness) = outcome else {
        panic!("{outcome}")
    };
    context.verify_witness(&witness, Budget::default()).unwrap();
    let mut wrong_result = witness.clone();
    wrong_result.applications[0].result = Term::Bool(false);
    assert!(
        context
            .verify_witness(&wrong_result, Budget::default())
            .is_err()
    );
    let mut wrong_input = witness;
    wrong_input.applications[0].arguments[0] = Term::Float64(0);
    assert!(
        context
            .verify_witness(&wrong_input, Budget::default())
            .is_err()
    );
}

#[test]
fn aggregate_argument_models_preserve_structure_without_inverting_the_head() {
    let mut context = Context::default();
    context.admit_congruence(id(100));
    let left = Term::RecordValue(vec![
        (
            "first".into(),
            Term::TupleValue(vec![Term::Bool(true), Term::Unit]),
        ),
        ("second".into(), Term::Bool(false)),
    ]);
    let right = Term::RecordValue(vec![
        ("second".into(), Term::Bool(false)),
        (
            "first".into(),
            Term::TupleValue(vec![Term::Bool(true), Term::Unit]),
        ),
    ]);
    // Ordered records are distinct completed packets. Their opaque Type
    // results may nevertheless coincide; no inverse equation is justified.
    context.assume(F::equal(head(left.clone()), head(right.clone())));
    let outcome = context.prove(F::False, Budget::default());
    let Outcome::NotEntailed(witness) = outcome else {
        panic!("{outcome}")
    };
    context.verify_witness(&witness, Budget::default()).unwrap();
    assert_eq!(witness.applications.len(), 2);
    assert_eq!(
        witness.applications[0].result,
        witness.applications[1].result
    );
    let mut damaged = witness.clone();
    damaged.applications[0].arguments[0] = Term::TupleValue(vec![Term::Bool(false)]);
    assert!(context.verify_witness(&damaged, Budget::default()).is_err());
    assert!(matches!(
        context.prove(F::equal(head(left), boolean()), Budget::default()),
        Outcome::NotEntailed(_)
    ));
}

#[test]
fn fixed_arguments_preserve_congruence_without_inventing_injectivity() {
    let mut context = Context::default();
    context.admit_congruence(id(100));
    let equality = F::equal(head(Term::Bool(true)), head(Term::Bool(false)));
    for goal in [equality.clone(), equality.negate()] {
        let outcome = context.prove(goal, Budget::default());
        let Outcome::NotEntailed(witness) = outcome else {
            panic!("{outcome}")
        };
        context.verify_witness(&witness, Budget::default()).unwrap();
    }
    let goal = F::equal(
        head(Term::String(String::from("same").into())),
        head(Term::String(String::from("same").into())),
    );
    let outcome = context.prove(goal, Budget::default());
    let Outcome::Entailed(proof) = outcome else {
        panic!("{outcome}")
    };
    context.verify_proof(&proof, Budget::default()).unwrap();
}

#[test]
fn large_completed_payloads_are_metered_before_model_indexing() {
    let mut context = Context::default();
    context.admit_congruence(id(100));
    let goal = F::equal(head(Term::String("x".repeat(4096).into())), boolean());
    let outcome = context.prove(
        goal.clone(),
        Budget {
            steps: 100,
            ..Budget::default()
        },
    );
    assert!(
        matches!(
            outcome,
            Outcome::Unknown {
                reason: ptc::UnknownReason::BudgetExhausted,
                ..
            }
        ),
        "{outcome}"
    );
    let outcome = context.prove(
        goal,
        Budget {
            steps: 1_000_000,
            ..Budget::default()
        },
    );
    let Outcome::NotEntailed(witness) = outcome else {
        panic!("{outcome}")
    };
    context
        .verify_witness(
            &witness,
            Budget {
                steps: 1_000_000,
                ..Budget::default()
            },
        )
        .unwrap();
    assert!(matches!(
        context.verify_witness(
            &witness,
            Budget {
                steps: 100,
                ..Budget::default()
            }
        ),
        Err(ptc::ProofError::BudgetExhausted)
    ));
}

#[test]
fn closed_opaque_applications_remain_symbolic_type_solutions() {
    let mut context = Context::default();
    context.admit_congruence(id(100));
    for arguments in [vec![], vec![Term::Primitive(Primitive::Bool)]] {
        let expected = Term::NormalForm {
            head: id(100),
            sort: Sort::Type,
            arguments,
        };
        let constraint = F::equal(var(0), expected.clone());
        let outcome =
            context.search_type_inputs(&[id(0)], constraint.clone(), &mut Budget::default());
        let TypeInputOutcome::Unique(solution) = outcome else {
            panic!("{outcome}")
        };
        assert_eq!(solution.bindings[&id(0)].0, expected);
        let problem = context
            .type_input_context(&[id(0)], constraint, &mut Budget::default())
            .unwrap();
        problem
            .verify_proof(&solution.bindings[&id(0)].1, Budget::default())
            .unwrap();
        problem
            .verify_witness(&solution.model, Budget::default())
            .unwrap();
    }
}
