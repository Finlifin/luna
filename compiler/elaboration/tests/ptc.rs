use elaboration::ptc::{Budget, Context, Formula as F, Outcome, UnknownReason};
use nir::{DeclarationId, Parameter, Primitive as P, Sort, Term};
fn ty(p: P) -> Term {
    Term::Primitive(p)
}
fn variable(index: u32) -> Term {
    Term::Rigid {
        declaration: DeclarationId {
            package: 0,
            local: index,
        },
        sort: Sort::Type,
    }
}
fn head(index: u32, arguments: Vec<Term>) -> Term {
    Term::NormalForm {
        head: DeclarationId {
            package: 1,
            local: index,
        },
        sort: Sort::Type,
        arguments,
    }
}
fn eq(a: Term, b: Term) -> F {
    F::equal(a, b)
}
fn proved(context: &Context, goal: F) {
    let outcome = context.prove(goal, Budget::default());
    assert!(matches!(outcome, Outcome::Entailed(_)), "{outcome}");
    if let Outcome::Entailed(proof) = outcome {
        assert_eq!(context.verify_proof(&proof, Budget::default()), Ok(()));
    }
}

fn associated(subject: Term, arguments: Vec<Term>, member: u32) -> Term {
    Term::AssociatedType {
        origin: Box::new(nir::AssociatedTypeOrigin {
            interface: DeclarationId {
                package: 9,
                local: 1,
            },
            member: DeclarationId {
                package: 9,
                local: member,
            },
        }),
        subject: Box::new(subject),
        arguments,
    }
}

#[test]
fn associated_type_congruence_preserves_origin_without_injectivity() {
    let mut context = Context::default();
    context.assume(eq(variable(1), ty(P::Bool)));
    context.assume(eq(variable(2), ty(P::Char)));
    proved(
        &context,
        eq(
            associated(variable(1), vec![variable(2)], 2),
            associated(ty(P::Bool), vec![ty(P::Char)], 2),
        ),
    );
    assert!(!matches!(
        context.prove(
            eq(
                associated(variable(1), vec![], 2),
                associated(variable(1), vec![], 3)
            ),
            Budget::default()
        ),
        Outcome::Entailed(_)
    ));

    let mut reverse = Context::default();
    reverse.assume(eq(
        associated(variable(1), vec![variable(2)], 2),
        associated(variable(3), vec![variable(4)], 2),
    ));
    for (a, b) in [(1, 3), (2, 4)] {
        assert!(!matches!(
            reverse.prove(eq(variable(a), variable(b)), Budget::default()),
            Outcome::Entailed(_)
        ));
    }
    // An associated result may equal any concrete type. Neither a primitive
    // nor a distinct projection is automatically disjoint from it.
    let unresolved = associated(ty(P::Bool), vec![], 2);
    for concrete in [ty(P::Bool), ty(P::Char)] {
        assert!(!matches!(
            Context::default().prove(
                F::Not(Box::new(eq(unresolved.clone(), concrete))),
                Budget::default()
            ),
            Outcome::Entailed(_)
        ));
    }
}

#[test]
fn associated_type_inputs_cannot_hide_from_solver_budgets() {
    let wide = associated(ty(P::Bool), vec![ty(P::Char); 100], 2);
    let result = Context::default().prove(
        eq(wide.clone(), wide),
        Budget {
            steps: 20,
            max_depth: 128,
        },
    );
    assert!(matches!(result, Outcome::Unknown { .. }), "{result}");
}

#[test]
fn proof_replay_rejects_forged_leaves_and_foreign_scope_support() {
    use elaboration::ptc::{ProofError, ProofTree};
    let mut context = Context::default();
    let goal = eq(variable(1), ty(P::Bool));
    context.assume(goal.clone());
    let Outcome::Entailed(mut proof) = context.prove(goal, Budget::default()) else {
        panic!("expected proof")
    };
    assert_eq!(context.verify_proof(&proof, Budget::default()), Ok(()));
    assert_eq!(
        Context::default().verify_proof(&proof, Budget::default()),
        Err(ProofError::Invalid)
    );
    assert_eq!(
        context.verify_proof(
            &proof,
            Budget {
                steps: 0,
                max_depth: 128
            }
        ),
        Err(ProofError::BudgetExhausted)
    );
    proof.tree = ProofTree::BooleanConflict;
    assert_eq!(
        context.verify_proof(&proof, Budget::default()),
        Err(ProofError::Invalid)
    );
    proof.tree = ProofTree::TheoryConflict(vec![]);
    assert_eq!(
        context.verify_proof(&proof, Budget::default()),
        Err(ProofError::Invalid)
    );
    proof.tree = ProofTree::Split {
        atom: elaboration::ptc::Atom::TypeEqual(elaboration::ptc::Equality(
            ty(P::Char),
            ty(P::Bool),
        )),
        negative: Box::new(ProofTree::BooleanConflict),
        positive: Box::new(ProofTree::BooleanConflict),
    };
    assert_eq!(
        context.verify_proof(&proof, Budget::default()),
        Err(ProofError::Invalid)
    );
}

#[test]
fn finite_domain_disjunction_and_negative_literals_remain_joint() {
    let t = variable(1);
    let mut context = Context::default();
    context.assume(F::Or(vec![
        eq(t.clone(), ty(P::U32)),
        eq(t.clone(), ty(P::F32)),
    ]));
    context.assume(eq(t.clone(), ty(P::U32)).negate());
    proved(&context, eq(t, ty(P::F32)));
}
#[test]
fn equality_is_symmetric_transitive_and_congruent_for_structural_types() {
    let (a, b, c) = (variable(1), variable(2), variable(3));
    let mut context = Context::default();
    context.assume(eq(a.clone(), b.clone()));
    context.assume(eq(b.clone(), c.clone()));
    proved(&context, eq(c.clone(), a.clone()));
    proved(
        &context,
        eq(
            Term::Tuple(vec![a.clone(), Term::Pointer(Box::new(c))]),
            Term::Tuple(vec![b.clone(), Term::Pointer(Box::new(b))]),
        ),
    );
}
#[test]
fn structural_injectivity_and_disjoint_closed_constructors_explain_conflicts() {
    let t = variable(1);
    let mut context = Context::default();
    context.assume(eq(
        Term::Pointer(Box::new(t.clone())),
        Term::Pointer(Box::new(ty(P::Bool))),
    ));
    proved(&context, eq(t, ty(P::Bool)));
    context.assume(eq(ty(P::Bool), ty(P::Char)));
    proved(&context, F::False);
}
#[test]
fn normal_form_bodies_are_not_unfolded_and_congruence_requires_admission() {
    let (a, b) = (variable(1), variable(2));
    let mut context = Context::default();
    context.assume(eq(a.clone(), b.clone()));
    let goal = eq(head(7, vec![a.clone()]), head(7, vec![b.clone()]));
    assert!(matches!(
        context.prove(goal.clone(), Budget::default()),
        Outcome::Unknown {
            reason: UnknownReason::UnvalidatedModel,
            ..
        }
    ));
    context.admit_congruence(DeclarationId {
        package: 1,
        local: 7,
    });
    proved(&context, goal);
    assert!(matches!(
        context.prove(eq(head(7, vec![a.clone()]), a), Budget::default()),
        Outcome::Unknown { .. }
    ));
}
#[test]
fn congruent_calls_are_not_injective_or_disjoint_without_a_contract() {
    let (a, b) = (variable(1), variable(2));
    let mut context = Context::default();
    context.admit_congruence(DeclarationId {
        package: 1,
        local: 7,
    });
    context.assume(eq(head(7, vec![a.clone()]), head(7, vec![b.clone()])));
    assert!(!matches!(
        context.prove(eq(a, b), Budget::default()),
        Outcome::Entailed(_)
    ));
    let mut different = Context::default();
    different.assume(eq(head(7, vec![]), head(8, vec![])));
    assert!(!matches!(
        different.prove(F::False, Budget::default()),
        Outcome::Entailed(_)
    ));
}
#[test]
fn branch_facts_and_theory_merges_do_not_escape_their_query() {
    let root = Context::default();
    let t = variable(1);
    let mut branch = root.fork();
    branch.assume(eq(t.clone(), ty(P::Bool)));
    proved(&branch, eq(t.clone(), ty(P::Bool)));
    assert!(matches!(
        root.prove(eq(t.clone(), ty(P::Bool)), Budget::default()),
        Outcome::NotEntailed(_)
    ));
    let mut alternatives = root.fork();
    alternatives.assume(F::Or(vec![
        F::And(vec![eq(t.clone(), ty(P::Bool)), eq(t.clone(), ty(P::Char))]),
        eq(t.clone(), ty(P::U32)),
    ]));
    proved(&alternatives, eq(t, ty(P::U32)));
}
#[test]
fn counterexamples_contain_real_type_values_and_are_not_negation_proofs() {
    let mut context = Context::default();
    let t = variable(1);
    context.assume(F::Or(vec![
        eq(t.clone(), ty(P::Bool)),
        eq(t.clone(), ty(P::Char)),
    ]));
    let Outcome::NotEntailed(witness) =
        context.prove(eq(t.clone(), ty(P::Bool)), Budget::default())
    else {
        panic!("expected source witness")
    };
    assert_eq!(
        witness.types[&DeclarationId {
            package: 0,
            local: 1
        }],
        ty(P::Char)
    );
    assert!(matches!(
        context.prove(eq(t, ty(P::Bool)).negate(), Budget::default()),
        Outcome::NotEntailed(_)
    ));
}
#[test]
fn budget_exhaustion_is_unknown_and_later_queries_recover() {
    let context = Context::default();
    let goal = eq(ty(P::Bool), ty(P::Bool));
    assert!(matches!(
        context.prove(
            goal.clone(),
            Budget {
                steps: 0,
                max_depth: 128
            }
        ),
        Outcome::Unknown {
            reason: UnknownReason::BudgetExhausted,
            ..
        }
    ));
    proved(&context, goal);
}
#[test]
fn malformed_terms_are_rejected_even_under_a_boolean_tautology() {
    for bad in [
        Term::Bool(true),
        Term::Hole {
            index: 0,
            sort: Sort::Type,
        },
        Term::Bound {
            depth: 0,
            index: 0,
            sort: Sort::Type,
        },
        Term::Tuple(vec![Term::Bool(true)]),
    ] {
        let goal = F::Or(vec![F::True, eq(bad.clone(), bad)]);
        assert!(matches!(
            Context::default().prove(goal, Budget::default()),
            Outcome::IllFormed(_)
        ));
    }
}
#[test]
fn dependent_binders_do_not_get_flattened_into_global_equality_variables() {
    let signature = Term::Function {
        parameters: vec![Parameter {
            sort: Sort::Type,
            ty: ty(P::Type),
        }],
        result: Box::new(Term::Bound {
            depth: 0,
            index: 0,
            sort: Sort::Type,
        }),
    };
    let constant = Term::Function {
        parameters: vec![Parameter {
            sort: Sort::Type,
            ty: ty(P::Type),
        }],
        result: Box::new(ty(P::Bool)),
    };
    let mut context = Context::default();
    context.assume(eq(signature.clone(), constant));
    assert!(!matches!(
        context.prove(F::False, Budget::default()),
        Outcome::Entailed(_)
    ));
    proved(&Context::default(), eq(signature.clone(), signature));
}
#[test]
fn proof_support_and_sexpression_order_are_stable() {
    let mut context = Context::default();
    context.assume(eq(variable(1), ty(P::Bool)));
    let goal = eq(variable(1), ty(P::Bool));
    let first = context.prove(goal.clone(), Budget::default());
    assert_eq!(
        first.to_string(),
        context.prove(goal, Budget::default()).to_string()
    );
    let Outcome::Entailed(proof) = first else {
        panic!()
    };
    assert_eq!(proof.assumptions.len(), 1);
    assert!(
        Outcome::Entailed(proof)
            .to_string()
            .starts_with("(entailed (assumptions")
    );
}

#[test]
fn supported_boolean_theory_results_agree_with_independent_ground_evaluation() {
    use std::collections::BTreeMap;
    fn replace(term: &Term, env: &BTreeMap<DeclarationId, Term>) -> Term {
        match term {
            Term::Rigid { declaration, .. } => env[declaration].clone(),
            Term::Pointer(inner) => Term::Pointer(Box::new(replace(inner, env))),
            Term::Tuple(items) => Term::Tuple(items.iter().map(|t| replace(t, env)).collect()),
            term => term.clone(),
        }
    }
    fn eval(formula: &F, env: &BTreeMap<DeclarationId, Term>) -> bool {
        match formula {
            F::Kind(_) => panic!("no kind atoms in this generator"),
            F::True => true,
            F::False => false,
            F::Equal(e) => replace(&e.0, env) == replace(&e.1, env),
            F::IntLe(_) => panic!("no integer atoms in this generator"),
            F::Not(f) => !eval(f, env),
            F::And(fs) => fs.iter().all(|f| eval(f, env)),
            F::Or(fs) => fs.iter().any(|f| eval(f, env)),
            F::Trait(_) => panic!("finite type-only oracle has no implementation world"),
        }
    }
    let (t, u) = (variable(1), variable(2));
    let atoms = [
        eq(t.clone(), ty(P::Bool)),
        eq(u.clone(), ty(P::Char)),
        eq(t.clone(), u.clone()),
        eq(
            Term::Pointer(Box::new(t.clone())),
            Term::Pointer(Box::new(u.clone())),
        ),
        eq(
            Term::Tuple(vec![t.clone(), u.clone()]),
            Term::Tuple(vec![u, t]),
        ),
    ];
    for a in &atoms {
        for b in &atoms {
            for goal in &atoms {
                let fact = F::Or(vec![a.clone(), b.clone().negate()]);
                let mut context = Context::default();
                context.assume(fact.clone());
                match context.prove(goal.clone(), Budget::default()) {
                    Outcome::Entailed(_) => {
                        for x in [P::Bool, P::Char, P::U32] {
                            for y in [P::Bool, P::Char, P::U32] {
                                let env = BTreeMap::from([
                                    (
                                        DeclarationId {
                                            package: 0,
                                            local: 1,
                                        },
                                        ty(x),
                                    ),
                                    (
                                        DeclarationId {
                                            package: 0,
                                            local: 2,
                                        },
                                        ty(y),
                                    ),
                                ]);
                                assert!(
                                    !eval(&fact, &env) || eval(goal, &env),
                                    "unsound proof: {fact} => {goal}"
                                );
                            }
                        }
                    }
                    Outcome::NotEntailed(witness) => {
                        assert!(eval(&fact, &witness.types));
                        assert!(!eval(goal, &witness.types));
                    }
                    Outcome::Unknown { .. } => {}
                    Outcome::IllFormed(error) => panic!("valid formula rejected: {error}"),
                }
            }
        }
    }
}

#[test]
fn type_structure_depth_is_limited_before_recursive_processing() {
    let mut deep = ty(P::Bool);
    for _ in 0..32 {
        deep = Term::Pointer(Box::new(deep));
    }
    assert!(matches!(
        Context::default().prove(
            eq(deep.clone(), deep),
            Budget {
                steps: 100_000,
                max_depth: 8
            }
        ),
        Outcome::Unknown {
            reason: UnknownReason::BudgetExhausted,
            ..
        }
    ));
}

#[test]
fn declared_nominal_constructor_evidence_is_required_and_replayed() {
    let declaration = DeclarationId {
        package: 9,
        local: 1,
    };
    let nominal = |value| Term::Nominal {
        declaration,
        arguments: vec![value],
    };
    let mut base = Context::default();
    base.assume(eq(nominal(variable(1)), nominal(ty(P::Bool))));
    assert!(!matches!(
        base.prove(eq(variable(1), ty(P::Bool)), Budget::default()),
        Outcome::Entailed(_)
    ));
    let mut admitted = base.fork();
    admitted.admit_nominal_constructor(declaration);
    let Outcome::Entailed(proof) = admitted.prove(eq(variable(1), ty(P::Bool)), Budget::default())
    else {
        panic!("registered nominal constructor must determine its argument")
    };
    assert!(admitted.verify_proof(&proof, Budget::default()).is_ok());
    assert!(base.verify_proof(&proof, Budget::default()).is_err());
    let mut forged = proof.clone();
    forged.nominal_constructors.clear();
    assert!(base.verify_proof(&forged, Budget::default()).is_err());
    proved(
        &admitted,
        eq(nominal(ty(P::Bool)), nominal(ty(P::Char))).negate(),
    );
    let mut opaque = Context::default();
    opaque.admit_nominal_constructor(declaration);
    opaque.admit_congruence(declaration);
    let nf = |value| Term::NormalForm {
        head: declaration,
        sort: Sort::Type,
        arguments: vec![value],
    };
    opaque.assume(eq(nf(variable(1)), nf(ty(P::Bool))));
    assert!(!matches!(
        opaque.prove(eq(variable(1), ty(P::Bool)), Budget::default()),
        Outcome::Entailed(_)
    ));
}

#[test]
fn nominal_countermodels_are_concrete_scoped_and_independently_verified() {
    let declaration = DeclarationId {
        package: 8,
        local: 2,
    };
    let nominal = |ty| Term::Nominal {
        declaration,
        arguments: vec![ty],
    };
    let mut context = Context::default();
    context.admit_nominal_constructor(declaration);
    context.assume(eq(variable(1), nominal(ty(P::Bool))));
    let Outcome::NotEntailed(witness) =
        context.prove(eq(variable(1), nominal(ty(P::Char))), Budget::default())
    else {
        panic!("a declared nominal instance should have a concrete countermodel")
    };
    assert_eq!(context.verify_witness(&witness, Budget::default()), Ok(()));
    let mut changed = witness.clone();
    changed.types.insert(
        DeclarationId {
            package: 0,
            local: 1,
        },
        nominal(ty(P::Char)),
    );
    assert_eq!(
        context.verify_witness(&changed, Budget::default()),
        Err(elaboration::ptc::ProofError::Invalid)
    );
    let mut other = Context::default();
    other.assume(witness.assumptions[0].clone());
    assert!(other.verify_witness(&witness, Budget::default()).is_err());
    assert_eq!(
        context.verify_witness(
            &witness,
            Budget {
                steps: 0,
                max_depth: 128
            }
        ),
        Err(elaboration::ptc::ProofError::BudgetExhausted)
    );
    assert!(
        Outcome::NotEntailed(witness)
            .to_string()
            .contains("(nominal-constructors")
    );
}

#[test]
fn unregistered_nominal_and_normal_form_countermodels_remain_unknown() {
    let declaration = DeclarationId {
        package: 8,
        local: 3,
    };
    let nominal = |ty| Term::Nominal {
        declaration,
        arguments: vec![ty],
    };
    assert!(matches!(
        Context::default().prove(
            eq(nominal(ty(P::Bool)), nominal(ty(P::Char))),
            Budget::default()
        ),
        Outcome::Unknown { .. }
    ));
    let mut context = Context::default();
    context.admit_nominal_constructor(declaration);
    context.admit_congruence(declaration);
    let opaque = |ty| Term::NormalForm {
        head: declaration,
        sort: Sort::Type,
        arguments: vec![ty],
    };
    assert!(matches!(
        context.prove(
            eq(opaque(ty(P::Bool)), opaque(ty(P::Char))),
            Budget::default()
        ),
        Outcome::Unknown { .. }
    ));
    context.assume(F::Or(vec![F::True, eq(opaque(ty(P::Bool)), ty(P::Bool))]));
    assert!(matches!(
        context.prove(F::False, Budget::default()),
        Outcome::Unknown { .. }
    ));
}

#[test]
fn structural_view_metering_accounts_for_success_failure_and_recovery() {
    let subject = variable(992);
    let record = Term::Record(vec![("flag".into(), ty(P::Bool))]);
    let mut context = Context::default();
    context.assume(eq(subject.clone(), record.clone()));
    for steps in [0, 1, 4] {
        let (result, used) = context.structural_view_metered(
            &subject,
            Budget {
                steps,
                max_depth: 128,
            },
        );
        assert!(result.is_err());
        assert!(used <= steps);
        if steps > 0 {
            assert!(used > 0);
        }
    }
    let (result, used) = context.structural_view_metered(&subject, Budget::default());
    let (view, proof) = result.unwrap().unwrap();
    assert_eq!(view, record);
    assert!(used > 0 && used <= Budget::default().steps);
    assert_eq!(context.verify_proof(&proof, Budget::default()), Ok(()));
    assert_eq!(
        context
            .structural_view(&subject, Budget::default())
            .unwrap()
            .unwrap()
            .0,
        view
    );
}

#[test]
fn structural_views_carry_replayable_equality_and_preserve_opaque_heads() {
    let opaque = head(700, vec![ty(P::Bool)]);
    let shape = Term::Tuple(vec![ty(P::Bool), ty(P::Char)]);
    let mut context = Context::default();
    assert!(context.structural_view(&opaque, Budget::default()).is_err());
    context.assume(F::equal(opaque.clone(), shape.clone()));
    let (found, proof) = context
        .structural_view(&opaque, Budget::default())
        .unwrap()
        .unwrap();
    assert_eq!(found, shape);
    context.verify_proof(&proof, Budget::default()).unwrap();
    assert_eq!(proof.goal, F::equal(opaque.clone(), found));
    assert!(
        Context::default()
            .verify_proof(&proof, Budget::default())
            .is_err()
    );
    assert_eq!(opaque, head(700, vec![ty(P::Bool)]));
    assert!(
        context
            .structural_view(
                &opaque,
                Budget {
                    steps: 0,
                    max_depth: 128
                }
            )
            .is_err()
    );
}

#[test]
fn structural_views_follow_proved_transitivity_not_disjunctive_choices() {
    let a = variable(701);
    let b = variable(702);
    let shape = Term::Nominal {
        declaration: DeclarationId {
            package: 0,
            local: 703,
        },
        arguments: vec![ty(P::Bool)],
    };
    let mut context = Context::default();
    context.admit_nominal_constructor(DeclarationId {
        package: 0,
        local: 703,
    });
    context.assume(F::equal(a.clone(), b.clone()));
    context.assume(F::equal(b, shape.clone()));
    let (found, proof) = context
        .structural_view(&a, Budget::default())
        .unwrap()
        .unwrap();
    assert_eq!(found, shape);
    context.verify_proof(&proof, Budget::default()).unwrap();
    let mut alternatives = Context::default();
    alternatives.assume(F::Or(vec![
        F::equal(a.clone(), ty(P::Bool)),
        F::equal(a.clone(), ty(P::Char)),
    ]));
    assert!(
        alternatives
            .structural_view(&a, Budget::default())
            .unwrap()
            .is_none()
    );
}

#[test]
fn structural_views_of_designated_heads_require_equality_without_injectivity() {
    let identity = DeclarationId {
        package: 1,
        local: 990,
    };
    let accepts = |id| id == identity;
    let subject = variable(990);
    let tuple = head(990, vec![Term::TypeListValue(vec![ty(P::Bool), ty(P::U8)])]);
    let mut context = Context::default();
    context.assume(eq(subject.clone(), tuple.clone()));
    let (found, used) =
        context.structural_view_with_heads_metered(&subject, Budget::default(), accepts);
    let (found, proof) = found.unwrap().unwrap();
    assert_eq!(found, tuple);
    assert!(used > 0);
    context.verify_proof(&proof, Budget::default()).unwrap();
    assert_eq!(proof.goal, eq(subject.clone(), tuple.clone()));
    assert!(!matches!(
        context.structural_view(&subject, Budget::default()),
        Ok(Some(_))
    ));
    assert!(
        Context::default()
            .verify_proof(&proof, Budget::default())
            .is_err()
    );
    let mut alternative = Context::default();
    alternative.assume(F::Or(vec![
        eq(subject.clone(), tuple.clone()),
        eq(subject.clone(), ty(P::Bool)),
    ]));
    assert!(!matches!(
        alternative
            .structural_view_with_heads_metered(&subject, Budget::default(), accepts)
            .0,
        Ok(Some(_))
    ));
    assert!(
        context
            .structural_view_with_heads_metered(
                &subject,
                Budget {
                    steps: 0,
                    max_depth: 128
                },
                accepts
            )
            .0
            .is_err()
    );
    assert!(
        context
            .structural_view_with_heads_metered(&subject, Budget::default(), accepts)
            .0
            .unwrap()
            .is_some()
    );

    let left = variable(991);
    let right = variable(992);
    let mut equality = Context::default();
    equality.admit_congruence(identity);
    equality.assume(eq(
        head(990, vec![Term::TypeListValue(vec![left.clone()])]),
        head(990, vec![Term::TypeListValue(vec![right.clone()])]),
    ));
    assert!(!matches!(
        equality.prove(eq(left, right), Budget::default()),
        Outcome::Entailed(_)
    ));
}

#[test]
fn structural_view_search_does_not_export_local_binder_types() {
    let mut context = Context::default();
    context.assume(F::equal(
        variable(710),
        Term::Function {
            parameters: vec![Parameter {
                sort: Sort::Type,
                ty: ty(P::Type),
            }],
            result: Box::new(Term::Bound {
                depth: 0,
                index: 0,
                sort: Sort::Type,
            }),
        },
    ));
    assert!(
        context
            .structural_view(&variable(710), Budget::default())
            .unwrap()
            .is_none()
    );
}

#[test]
fn type_equalities_lift_through_nested_tuple_values_without_inverting_heads() {
    let (a, b) = (variable(1), variable(2));
    let wrap = |value| {
        head(
            7,
            vec![Term::TupleValue(vec![
                Term::Unit,
                Term::TupleValue(vec![value, Term::Bool(true)]),
            ])],
        )
    };
    let mut context = Context::default();
    context.admit_congruence(DeclarationId {
        package: 1,
        local: 7,
    });
    context.assume(eq(a.clone(), b.clone()));
    proved(&context, eq(wrap(a.clone()), wrap(b.clone())));
    let mut reverse = Context::default();
    reverse.admit_congruence(DeclarationId {
        package: 1,
        local: 7,
    });
    reverse.assume(eq(wrap(a.clone()), wrap(b.clone())));
    assert!(!matches!(
        reverse.prove(eq(a, b), Budget::default()),
        Outcome::Entailed(_)
    ));
    assert!(matches!(
        context.prove(
            eq(
                Term::TupleValue(vec![ty(P::U8)]),
                Term::TupleValue(vec![ty(P::U8)])
            ),
            Budget::default()
        ),
        Outcome::IllFormed(_)
    ));
}

#[test]
fn tuple_value_nesting_is_bounded_before_ptc_hashing_and_cloning() {
    let mut nested = ty(P::U8);
    for _ in 0..64 {
        nested = Term::TupleValue(vec![nested]);
    }
    let term = head(7, vec![nested]);
    assert!(matches!(
        Context::default().prove(
            eq(term.clone(), term),
            Budget {
                steps: 10_000,
                max_depth: 8
            }
        ),
        Outcome::Unknown {
            reason: UnknownReason::BudgetExhausted,
            ..
        }
    ));
}

#[test]
fn owned_package_context_restores_only_valid_declaration_theory() {
    use nir::normal_forms::{Head, Kind, Parameter};
    use nir::program::{LocalId, Package};
    let declaration = DeclarationId {
        package: 1,
        local: 7,
    };
    let mut package = Package::default();
    package.normal_forms.push(Head {
        role: nir::normal_forms::Role::Opaque,
        declaration,
        parameters: vec![Parameter {
            declaration: DeclarationId {
                package: 1,
                local: 8,
            },
            binding: LocalId { owner: 7, index: 0 },
            kind: Kind::Type,
            ty: ty(P::Type),
        }],
        requirements: vec![],
    });
    let mut context = Context::from_package(&package).unwrap();
    context.assume(eq(variable(1), variable(2)));
    proved(
        &context,
        eq(head(7, vec![variable(1)]), head(7, vec![variable(2)])),
    );
    let fresh = Context::from_package(&package).unwrap();
    assert!(!matches!(
        fresh.prove(eq(variable(1), variable(2)), Budget::default()),
        Outcome::Entailed(_)
    ));
    assert!(!matches!(
        fresh.prove(eq(head(7, vec![ty(P::U8)]), ty(P::U8)), Budget::default()),
        Outcome::Entailed(_)
    ));
    package.normal_forms[0].parameters[0].ty = ty(P::Bool);
    assert!(Context::from_package(&package).is_err());
    assert!(
        elaboration::materialization::dispatch_instances(&package, Budget::default())
            .unwrap_err()
            .contains("dispatch type context")
    );
}

#[test]
fn comptime_callable_equality_preserves_phase_and_component_evidence() {
    let signature = |result| Term::Function {
        parameters: vec![],
        result: Box::new(result),
    };
    let a = signature(variable(1)).with_comptime_phase(true);
    let b = signature(variable(2)).with_comptime_phase(true);
    let mut context = Context::default();
    context.assume(eq(variable(1), variable(2)));
    proved(&context, eq(a.clone(), b.clone()));
    proved(
        &Context::default(),
        F::Not(Box::new(eq(a.clone(), signature(variable(1))))),
    );
    let mut reverse = Context::default();
    reverse.assume(eq(a, b));
    proved(&reverse, eq(variable(1), variable(2)));
}

#[test]
fn record_fields_obey_type_input_budgets_and_queries_recover() {
    let mut deep = ty(P::Bool);
    for _ in 0..32 {
        deep = Term::Record(vec![("nested".into(), deep)]);
    }
    let context = Context::default();
    assert!(matches!(
        context.prove(
            eq(deep.clone(), deep),
            Budget {
                steps: 100_000,
                max_depth: 8
            }
        ),
        Outcome::Unknown {
            reason: UnknownReason::BudgetExhausted,
            ..
        }
    ));
    let record = Term::Record(vec![("flag".into(), ty(P::Bool))]);
    proved(&context, eq(record.clone(), record));
}

#[test]
fn records_are_distinct_from_other_type_families_without_deciding_permutations() {
    let record = Term::Record(vec![
        ("flag".into(), ty(P::Bool)),
        ("count".into(), ty(P::U8)),
    ]);
    let context = Context::default();
    for other in [
        ty(P::Bool),
        Term::Tuple(vec![ty(P::Bool), ty(P::U8)]),
        Term::Pointer(Box::new(ty(P::Bool))),
        Term::Nominal {
            declaration: DeclarationId {
                package: 0,
                local: 2,
            },
            arguments: vec![],
        },
    ] {
        proved(&context, F::Not(Box::new(eq(record.clone(), other))));
    }
    let permuted = Term::Record(vec![
        ("count".into(), ty(P::U8)),
        ("flag".into(), ty(P::Bool)),
    ]);
    assert!(matches!(
        context.prove(eq(record.clone(), permuted.clone()), Budget::default()),
        Outcome::Unknown { .. }
    ));
    assert!(matches!(
        context.prove(F::Not(Box::new(eq(record, permuted))), Budget::default()),
        Outcome::Unknown { .. }
    ));
}

#[test]
fn record_type_congruence_uses_field_evidence_without_inventing_inverse_rules() {
    let field = variable(910);
    let record = |value| Term::Record(vec![("field".into(), value)]);
    let left = record(record(field.clone()));
    let right = record(record(ty(P::Bool)));
    let goal = eq(left.clone(), right.clone());
    assert!(!matches!(
        Context::default().prove(goal.clone(), Budget::default()),
        Outcome::Entailed(_)
    ));
    let mut context = Context::default();
    context.assume(eq(field.clone(), ty(P::Bool)));
    assert!(matches!(
        context.prove(
            goal.clone(),
            Budget {
                steps: 1,
                max_depth: 64
            }
        ),
        Outcome::Unknown { .. }
    ));
    let Outcome::Entailed(proof) = context.prove(goal, Budget::default()) else {
        panic!("record congruence")
    };
    context.verify_proof(&proof, Budget::default()).unwrap();
    assert!(
        Context::default()
            .verify_proof(&proof, Budget::default())
            .is_err()
    );
    let changed_label = Term::Record(vec![("other".into(), record(ty(P::Bool)))]);
    assert!(!matches!(
        context.prove(eq(left.clone(), changed_label), Budget::default()),
        Outcome::Entailed(_)
    ));
    let mut inverse = Context::default();
    inverse.assume(eq(left, right));
    assert!(!matches!(
        inverse.prove(eq(field, ty(P::Bool)), Budget::default()),
        Outcome::Entailed(_)
    ));
}

#[test]
fn record_structural_views_require_scoped_equality_evidence() {
    let subject = variable(901);
    let opaque = head(902, vec![]);
    let shape = Term::Record(vec![("width".into(), ty(P::U8))]);
    assert!(
        Context::default()
            .structural_view(&subject, Budget::default())
            .unwrap()
            .is_none()
    );
    let mut context = Context::default();
    context.assume(eq(subject.clone(), opaque.clone()));
    context.assume(eq(opaque, shape.clone()));
    let (found, proof) = context
        .structural_view(&subject, Budget::default())
        .unwrap()
        .unwrap();
    assert_eq!(found, shape);
    context.verify_proof(&proof, Budget::default()).unwrap();
    assert!(
        Context::default()
            .verify_proof(&proof, Budget::default())
            .is_err()
    );
    assert!(
        context
            .structural_view(
                &subject,
                Budget {
                    steps: 0,
                    max_depth: 128
                }
            )
            .is_err()
    );
    // A disjunction supplies candidates, not permission to choose one.
    let mut alternatives = Context::default();
    alternatives.assume(F::Or(vec![
        eq(subject.clone(), shape),
        eq(
            subject.clone(),
            Term::Record(vec![("width".into(), ty(P::U16))]),
        ),
    ]));
    assert!(!matches!(
        alternatives.structural_view(&subject, Budget::default()),
        Ok(Some(_))
    ));
}

fn trait_atom(interface: u32, subject: Term, arguments: Vec<Term>) -> F {
    F::Trait(elaboration::ptc::TraitQuery {
        interface: DeclarationId {
            package: 42,
            local: interface,
        },
        subject,
        arguments,
    })
}

#[test]
fn trait_logic_handles_negation_disjunction_and_proof_replay() {
    let a = trait_atom(1, variable(1), vec![]);
    let b = trait_atom(2, variable(1), vec![]);
    let mut context = Context::default();
    context.assume(F::Or(vec![a.clone(), b.clone()]));
    context.assume(a.clone().negate());
    proved(&context, b.clone());
    proved(&Context::default(), F::Or(vec![a.clone(), a.negate()]));
    let Outcome::Entailed(proof) = context.prove(b, Budget::default()) else {
        panic!("missing proof")
    };
    assert!(
        Context::default()
            .verify_proof(&proof, Budget::default())
            .is_err()
    );
}

#[test]
fn trait_logic_uses_input_equality_without_inverting_truth_values() {
    let mut context = Context::default();
    context.assume(eq(variable(1), ty(P::Bool)));
    context.assume(eq(variable(2), ty(P::Char)));
    context.assume(trait_atom(1, variable(1), vec![variable(2)]));
    proved(&context, trait_atom(1, ty(P::Bool), vec![ty(P::Char)]));
    for unrelated in [
        trait_atom(2, ty(P::Bool), vec![ty(P::Char)]),
        trait_atom(1, ty(P::Bool), vec![ty(P::Bool)]),
        trait_atom(1, ty(P::Bool), vec![]),
    ] {
        assert!(!matches!(
            context.prove(unrelated, Budget::default()),
            Outcome::Entailed(_)
        ));
    }
    let mut context = Context::default();
    context.assume(trait_atom(1, variable(1), vec![]));
    context.assume(trait_atom(1, variable(2), vec![]));
    assert!(!matches!(
        context.prove(eq(variable(1), variable(2)), Budget::default()),
        Outcome::Entailed(_)
    ));
}

#[test]
fn trait_logic_never_invents_world_evidence_from_a_sat_assignment() {
    let context = Context::default();
    let goal = trait_atom(1, ty(P::Bool), vec![]);
    for query in [goal.clone(), goal.negate()] {
        assert!(matches!(
            context.prove(query, Budget::default()),
            Outcome::Unknown { .. }
        ));
    }
}

#[test]
fn trait_logic_budget_failure_does_not_poison_later_proofs() {
    let goal = trait_atom(1, variable(1), vec![variable(2)]);
    let mut context = Context::default();
    context.assume(goal.clone());
    assert!(matches!(
        context.prove(
            goal.clone(),
            Budget {
                steps: 1,
                ..Budget::default()
            }
        ),
        Outcome::Unknown { .. }
    ));
    proved(&context, goal);
    let mut nested = ty(P::Bool);
    for _ in 0..40 {
        nested = Term::Pointer(Box::new(nested));
    }
    assert!(matches!(
        context.prove(
            trait_atom(1, nested, vec![]),
            Budget {
                max_depth: 8,
                ..Budget::default()
            }
        ),
        Outcome::Unknown { .. }
    ));
}

#[test]
fn trait_logic_keeps_opaque_heads_and_negative_scope_isolated() {
    let a = trait_atom(1, head(8, vec![ty(P::Bool)]), vec![]);
    let context = Context::default();
    let mut branch = context.fork();
    branch.assume(a.clone().negate());
    proved(&branch, a.clone().negate());
    assert!(!matches!(
        branch.prove(
            trait_atom(1, ty(P::Bool), vec![]).negate(),
            Budget::default()
        ),
        Outcome::Entailed(_)
    ));
    assert!(!matches!(
        context.prove(a.negate(), Budget::default()),
        Outcome::Entailed(_)
    ));
}

#[test]
fn trait_logic_agrees_with_finite_boolean_oracle() {
    fn atom(n: u32) -> F {
        trait_atom(n, ty(P::Bool), vec![])
    }
    fn evaluate(f: &F, bits: u8) -> bool {
        match f {
            F::Trait(q) => bits & (1 << q.interface.local) != 0,
            F::Not(f) => !evaluate(f, bits),
            F::And(fs) => fs.iter().all(|f| evaluate(f, bits)),
            F::Or(fs) => fs.iter().any(|f| evaluate(f, bits)),
            _ => panic!("trait-only finite oracle"),
        }
    }
    let mut forms = vec![atom(0), atom(1), atom(2)];
    forms.extend(forms.clone().into_iter().map(F::negate));
    for a in forms.clone() {
        for b in forms.clone().into_iter().take(6) {
            for goal in [
                F::And(vec![a.clone(), b.clone()]),
                F::Or(vec![a.clone(), b.clone()]),
                a.clone().implies(b),
            ] {
                let mut context = Context::default();
                let premise = F::Or(vec![atom(0), atom(1)]);
                context.assume(premise.clone());
                let expected =
                    (0..8).all(|bits| !evaluate(&premise, bits) || evaluate(&goal, bits));
                let actual = context.prove(goal, Budget::default());
                assert_eq!(matches!(actual, Outcome::Entailed(_)), expected, "{actual}");
                if let Outcome::Entailed(proof) = actual {
                    context.verify_proof(&proof, Budget::default()).unwrap();
                }
            }
        }
    }
}

#[test]
fn trait_value_indices_preserve_suffix_identity_and_type_congruence() {
    let px = Term::Symbol("px".into());
    let em = Term::Symbol("em".into());
    let mut context = Context::default();
    context.assume(eq(variable(1), ty(P::U32)));
    context.assume(trait_atom(
        20,
        ty(P::Integer),
        vec![px.clone(), variable(1)],
    ));
    let goal = trait_atom(20, ty(P::Integer), vec![px.clone(), ty(P::U32)]);
    proved(&context, goal.clone());
    for other in [
        trait_atom(20, ty(P::Integer), vec![em, ty(P::U32)]),
        trait_atom(20, ty(P::Integer), vec![px.clone(), ty(P::U64)]),
        trait_atom(20, ty(P::Integer), vec![ty(P::U32), px.clone()]),
        trait_atom(21, ty(P::Integer), vec![px.clone(), ty(P::U32)]),
    ] {
        assert!(!matches!(
            context.prove(other, Budget::default()),
            Outcome::Entailed(_)
        ));
    }
    assert!(matches!(
        Context::default().prove(goal.clone(), Budget::default()),
        Outcome::Unknown { .. }
    ));
    assert!(matches!(
        context.prove(eq(px.clone(), px), Budget::default()),
        Outcome::IllFormed(_)
    ));
    assert!(matches!(
        context.prove(
            goal.clone(),
            Budget {
                steps: 1,
                ..Budget::default()
            }
        ),
        Outcome::Unknown { .. }
    ));
    proved(&context, goal);
}

#[test]
fn trait_value_indices_validate_scope_and_do_not_admit_runtime_propositions() {
    let index = Term::Bound {
        depth: 0,
        index: 0,
        sort: Sort::ComptimeValue,
    };
    let goal = trait_atom(20, ty(P::Integer), vec![index]);
    let mut context = Context::with_scope(vec![vec![Sort::ComptimeValue]]);
    context.assume(goal.clone());
    proved(&context, goal.clone());
    for other in [
        Context::default(),
        Context::with_scope(vec![vec![Sort::Type]]),
    ] {
        assert!(matches!(
            other.prove(goal.clone(), Budget::default()),
            Outcome::IllFormed(_)
        ));
    }
    let runtime = Term::Rigid {
        declaration: DeclarationId {
            package: 1,
            local: 99,
        },
        sort: Sort::Runtime,
    };
    assert!(matches!(
        Context::default().prove(
            trait_atom(20, ty(P::Integer), vec![runtime]),
            Budget::default()
        ),
        Outcome::IllFormed(_)
    ));
    assert!(matches!(
        Context::default().prove(
            trait_atom(20, Term::Symbol("px".into()), vec![]),
            Budget::default()
        ),
        Outcome::IllFormed(_)
    ));
}

#[test]
fn trait_logic_rejects_non_type_inputs_and_preserves_argument_order() {
    let invalid = trait_atom(1, Term::Bool(true), vec![]);
    assert!(matches!(
        Context::default().prove(invalid, Budget::default()),
        Outcome::IllFormed(_)
    ));
    let mut context = Context::default();
    context.assume(trait_atom(1, ty(P::Bool), vec![ty(P::Bool), ty(P::Char)]));
    assert!(!matches!(
        context.prove(
            trait_atom(1, ty(P::Bool), vec![ty(P::Char), ty(P::Bool)]),
            Budget::default()
        ),
        Outcome::Entailed(_)
    ));
}

#[test]
fn trait_logic_dump_preserves_logical_and_interface_structure() {
    let formula = F::And(vec![
        trait_atom(1, variable(7), vec![head(8, vec![ty(P::Bool)])]),
        F::Or(vec![trait_atom(2, variable(7), vec![]).negate(), F::True]),
    ]);
    let dump = formula.to_string();
    assert!(dump.starts_with("(and (trait-bound (interface "));
    assert!(dump.contains("(arguments (normal-form"));
    assert!(dump.contains("(or (not (trait-bound"));
    println!("TRAIT_LOGIC_DUMP={dump}");
}

#[test]
fn computation_type_congruence_keeps_operation_layout_and_result() {
    let operation = |local, argument| nir::effects::Operation {
        declaration: nir::DeclarationId { package: 8, local },
        arguments: vec![argument],
    };
    let left = Term::computation(
        vec![operation(1, variable(1)), operation(1, ty(P::Char))],
        variable(2),
    );
    let right = Term::computation(
        vec![operation(1, ty(P::Bool)), operation(1, ty(P::Char))],
        ty(P::Bool),
    );
    let mut context = Context::default();
    context.assume(eq(variable(1), ty(P::Bool)));
    context.assume(eq(variable(2), ty(P::Bool)));
    proved(&context, eq(left, right.clone()));
    let missing_layer = Term::computation(vec![operation(1, ty(P::Bool))], ty(P::Bool));
    assert!(!matches!(
        context.prove(eq(right, missing_layer), Budget::default()),
        Outcome::Entailed(_)
    ));
    // Distinct occurrence layouts require row reasoning. Generic constructor
    // disjointness must not invent a proof that commuting rows are different.
    let a = Term::computation(
        vec![operation(1, ty(P::Bool)), operation(2, ty(P::Bool))],
        ty(P::Bool),
    );
    let b = Term::computation(
        vec![operation(2, ty(P::Bool)), operation(1, ty(P::Bool))],
        ty(P::Bool),
    );
    assert!(!matches!(
        context.prove(eq(a, b).negate(), Budget::default()),
        Outcome::Entailed(_)
    ));
}

#[test]
fn computation_type_proofs_bound_occurrences_and_nested_arguments() {
    let operation = nir::effects::Operation {
        declaration: nir::DeclarationId {
            package: 8,
            local: 1,
        },
        arguments: vec![],
    };
    let wide = Term::computation(vec![operation; 1000], ty(P::Bool));
    let context = Context::default();
    assert!(matches!(
        context.prove(
            eq(wide.clone(), wide),
            Budget {
                steps: 10,
                max_depth: 64
            }
        ),
        Outcome::Unknown {
            reason: UnknownReason::BudgetExhausted,
            ..
        }
    ));
}

#[test]
fn scoped_type_propositions_preserve_bound_identity_and_opaque_congruence() {
    let a = Term::Bound {
        depth: 0,
        index: 0,
        sort: Sort::Type,
    };
    let b = Term::Bound {
        depth: 0,
        index: 1,
        sort: Sort::Type,
    };
    let mut context = Context::with_scope(vec![vec![Sort::Type, Sort::Type]]);
    context.admit_congruence(DeclarationId {
        package: 1,
        local: 7,
    });
    context.assume(eq(a.clone(), ty(P::Bool)));
    proved(
        &context,
        eq(head(7, vec![a.clone()]), head(7, vec![ty(P::Bool)])),
    );
    assert!(!matches!(
        context.prove(eq(b.clone(), ty(P::Bool)), Budget::default()),
        Outcome::Entailed(_)
    ));
    assert!(!matches!(
        context.prove(eq(head(7, vec![a.clone()]), a.clone()), Budget::default()),
        Outcome::Entailed(_)
    ));
    let nested = |outer| Term::Function {
        parameters: vec![Parameter {
            sort: Sort::Type,
            ty: ty(P::Type),
        }],
        result: Box::new(Term::Tuple(vec![
            outer,
            Term::Bound {
                depth: 0,
                index: 0,
                sort: Sort::Type,
            },
        ])),
    };
    let nested_input = nested(Term::Bound {
        depth: 1,
        index: 0,
        sort: Sort::Type,
    });
    proved(&context, eq(nested_input.clone(), nested_input));
    assert!(matches!(
        Context::default().prove(eq(a.clone(), a), Budget::default()),
        Outcome::IllFormed(_)
    ));
    assert!(context.to_string().contains("(scope (frame type type))"));
}

#[test]
fn scoped_trait_boolean_assumptions_do_not_strengthen_disjunctions() {
    let input = Term::Bound {
        depth: 0,
        index: 0,
        sort: Sort::Type,
    };
    let left = F::from_predicate(&nir::program::TypePredicate::Trait(
        nir::program::TraitRequirement {
            interface: DeclarationId {
                package: 3,
                local: 1,
            },
            subject: input.clone(),
            interface_arguments: vec![],
        },
    ));
    let right = F::from_predicate(&nir::program::TypePredicate::Trait(
        nir::program::TraitRequirement {
            interface: DeclarationId {
                package: 3,
                local: 2,
            },
            subject: input,
            interface_arguments: vec![],
        },
    ));
    let mut context = Context::with_scope(vec![vec![Sort::Type]]);
    context.assume(F::Or(vec![left.clone(), right.clone()]));
    proved(&context, F::Or(vec![right.clone(), left.clone()]));
    assert!(!matches!(
        context.prove(left.clone(), Budget::default()),
        Outcome::Entailed(_)
    ));
    context.assume(left.clone().negate());
    proved(&context, right);
    proved(&context, left.negate());
}

#[test]
fn scoped_proofs_and_counterexamples_are_bound_to_the_exact_frame() {
    use elaboration::ptc::ProofError;
    let input = Term::Bound {
        depth: 0,
        index: 0,
        sort: Sort::Type,
    };
    let context = Context::with_scope(vec![vec![Sort::Type]]);
    let Outcome::Entailed(proof) = context.prove(eq(input.clone(), input), Budget::default())
    else {
        panic!("reflexivity")
    };
    assert_eq!(context.verify_proof(&proof, Budget::default()), Ok(()));
    for wrong in [
        Context::default(),
        Context::with_scope(vec![vec![Sort::Runtime]]),
        Context::with_scope(vec![vec![Sort::Type, Sort::Type]]),
    ] {
        assert_eq!(
            wrong.verify_proof(&proof, Budget::default()),
            Err(ProofError::Invalid)
        );
    }
    assert!(proof.to_string().contains("(scope (frame type))"));
    let Outcome::NotEntailed(witness) = context.prove(F::False, Budget::default()) else {
        panic!("constant false")
    };
    assert_eq!(context.verify_witness(&witness, Budget::default()), Ok(()));
    assert_eq!(
        Context::default().verify_witness(&witness, Budget::default()),
        Err(ProofError::Invalid)
    );
    assert!(context.theory_context().scope().is_empty());
    for invalid in [
        Term::Bound {
            depth: 1,
            index: 0,
            sort: Sort::Type,
        },
        Term::Bound {
            depth: 0,
            index: 1,
            sort: Sort::Type,
        },
        Term::Bound {
            depth: 0,
            index: 0,
            sort: Sort::Runtime,
        },
    ] {
        assert!(matches!(
            context.prove(eq(invalid.clone(), invalid), Budget::default()),
            Outcome::IllFormed(_)
        ));
    }
}

#[test]
fn scoped_structural_views_keep_outer_and_inner_frames_distinct() {
    let outer = Term::Bound {
        depth: 1,
        index: 0,
        sort: Sort::Type,
    };
    let inner = Term::Bound {
        depth: 0,
        index: 0,
        sort: Sort::Type,
    };
    let shape = Term::Tuple(vec![ty(P::Bool), inner.clone()]);
    let mut context = Context::with_scope(vec![vec![Sort::Type], vec![Sort::Type]]);
    context.assume(eq(outer.clone(), shape.clone()));
    let (view, proof) = context
        .structural_view(&outer, Budget::default())
        .unwrap()
        .expect("outer structural evidence");
    assert_eq!(view, shape);
    assert_eq!(context.verify_proof(&proof, Budget::default()), Ok(()));
    assert!(!matches!(
        context.prove(eq(inner, shape), Budget::default()),
        Outcome::Entailed(_)
    ));
    let mut wrong = Context::with_scope(vec![vec![Sort::Type, Sort::Type]]);
    for assumption in &proof.assumptions {
        wrong.assume(assumption.clone());
    }
    assert!(wrong.verify_proof(&proof, Budget::default()).is_err());
}

#[test]
fn record_value_congruence_retains_labels_and_type_only_boundary() {
    let (a, b) = (variable(1), variable(2));
    let wrap = |label: &str, value| head(7, vec![Term::RecordValue(vec![(label.into(), value)])]);
    let mut context = Context::default();
    context.admit_congruence(DeclarationId {
        package: 1,
        local: 7,
    });
    context.assume(eq(a.clone(), b.clone()));
    proved(
        &context,
        eq(wrap("item", a.clone()), wrap("item", b.clone())),
    );
    assert!(!matches!(
        context.prove(
            eq(wrap("left", a.clone()), wrap("right", b.clone())),
            Budget::default()
        ),
        Outcome::Entailed(_)
    ));
    assert!(matches!(
        context.prove(
            eq(Term::RecordValue(vec![]), Term::RecordValue(vec![])),
            Budget::default()
        ),
        Outcome::IllFormed(_)
    ));
    let mut reverse = Context::default();
    reverse.admit_congruence(DeclarationId {
        package: 1,
        local: 7,
    });
    reverse.assume(eq(wrap("item", a.clone()), wrap("item", b.clone())));
    assert!(!matches!(
        reverse.prove(eq(a, b), Budget::default()),
        Outcome::Entailed(_)
    ));
}

#[test]
fn nominal_value_congruence_preserves_variant_and_type_only_boundary() {
    let (a, b) = (variable(1), variable(2));
    let value = |variant, field| Term::NominalValue {
        ty: Box::new(Term::Nominal {
            declaration: DeclarationId {
                package: 1,
                local: 50,
            },
            arguments: vec![],
        }),
        variant,
        fields: vec![field],
    };
    let wrap = |variant, field| head(7, vec![value(variant, field)]);
    let mut context = Context::default();
    context.admit_congruence(DeclarationId {
        package: 1,
        local: 7,
    });
    context.assume(eq(a.clone(), b.clone()));
    proved(
        &context,
        eq(wrap(Some(0), a.clone()), wrap(Some(0), b.clone())),
    );
    for variant in [None, Some(1)] {
        assert!(!matches!(
            context.prove(
                eq(wrap(Some(0), a.clone()), wrap(variant, b.clone())),
                Budget::default()
            ),
            Outcome::Entailed(_)
        ));
    }
    assert!(matches!(
        context.prove(
            eq(value(Some(0), a.clone()), value(Some(0), a.clone())),
            Budget::default()
        ),
        Outcome::IllFormed(_)
    ));
    let mut reverse = Context::default();
    reverse.admit_congruence(DeclarationId {
        package: 1,
        local: 7,
    });
    reverse.assume(eq(wrap(Some(0), a.clone()), wrap(Some(0), b.clone())));
    assert!(!matches!(
        reverse.prove(eq(a, b), Budget::default()),
        Outcome::Entailed(_)
    ));
}

#[test]
fn record_labels_are_bounded_before_ptc_cloning_and_retry_is_clean() {
    for record in [
        Term::Record(vec![("x".repeat(512), ty(P::Bool))]),
        Term::RecordValue(vec![("x".repeat(512), Term::Bool(true))]),
    ] {
        let term = head(7, vec![record]);
        let formula = eq(term.clone(), term);
        let context = Context::default();
        assert!(matches!(
            context.prove(
                formula.clone(),
                Budget {
                    steps: 128,
                    max_depth: 128
                }
            ),
            Outcome::Unknown {
                reason: UnknownReason::BudgetExhausted,
                ..
            }
        ));
        proved(&context, formula);
    }
}

#[test]
fn metered_proofs_preserve_remaining_work_on_all_outcomes() {
    let context = Context::default();
    for goal in [
        eq(ty(P::Bool), ty(P::Bool)),
        eq(ty(P::Bool), ty(P::U8)),
        eq(
            Term::Bound {
                depth: 0,
                index: 0,
                sort: Sort::Type,
            },
            ty(P::Bool),
        ),
    ] {
        let expected = context.prove(goal.clone(), Budget::default());
        let mut budget = Budget::default();
        let outcome = context.prove_metered(goal.clone(), &mut budget);
        assert_eq!(
            std::mem::discriminant(&outcome),
            std::mem::discriminant(&expected)
        );
        let used = Budget::default().steps - budget.steps;
        assert!(used > 0 && used < 1000, "used {used}");
        let mut exact = Budget {
            steps: used,
            max_depth: 128,
        };
        let repeated = context.prove_metered(goal, &mut exact);
        assert_eq!(
            std::mem::discriminant(&repeated),
            std::mem::discriminant(&expected)
        );
        assert_eq!(exact.steps, 0);
    }
    let mut budget = Budget {
        steps: 64,
        max_depth: 128,
    };
    let mut completed = 0;
    loop {
        let before = budget.steps;
        let outcome = context.prove_metered(eq(ty(P::Bool), ty(P::Bool)), &mut budget);
        assert!(budget.steps <= before);
        if matches!(
            outcome,
            Outcome::Unknown {
                reason: UnknownReason::BudgetExhausted,
                ..
            }
        ) {
            break;
        }
        assert!(matches!(outcome, Outcome::Entailed(_)));
        assert!(budget.steps < before);
        completed += 1;
        assert!(completed <= 64);
    }
    assert!(completed > 1);
    assert_eq!(budget.steps, 0);
}

#[test]
fn type_list_inputs_lift_type_equalities_without_inverting_normal_forms() {
    let (a, b) = (variable(1), variable(2));
    let wrap = |value| head(7, vec![Term::TypeListValue(vec![value])]);
    let declaration = DeclarationId {
        package: 1,
        local: 7,
    };
    let mut context = Context::default();
    context.admit_congruence(declaration);
    context.assume(eq(a.clone(), b.clone()));
    proved(&context, eq(wrap(a.clone()), wrap(b.clone())));

    let mut reverse = Context::default();
    reverse.admit_congruence(declaration);
    reverse.assume(eq(wrap(a.clone()), wrap(b.clone())));
    assert!(!matches!(
        reverse.prove(eq(a.clone(), b), Budget::default()),
        Outcome::Entailed(_)
    ));
    assert!(!matches!(
        context.prove(eq(wrap(a.clone()), a), Budget::default()),
        Outcome::Entailed(_)
    ));
    assert!(matches!(
        context.prove(
            eq(Term::TypeListValue(vec![]), Term::TypeListValue(vec![])),
            Budget::default()
        ),
        Outcome::IllFormed(_)
    ));
    assert!(matches!(
        context.prove(
            eq(wrap(Term::Bool(true)), wrap(Term::Bool(true))),
            Budget::default()
        ),
        Outcome::IllFormed(_)
    ));
}

#[test]
fn type_list_input_budget_failure_does_not_poison_later_proofs() {
    let mut context = Context::default();
    context.admit_congruence(DeclarationId {
        package: 1,
        local: 7,
    });
    let application = head(7, vec![Term::TypeListValue(vec![ty(P::U8); 64])]);
    let proposition = eq(application.clone(), application);
    assert!(matches!(
        context.prove(
            proposition.clone(),
            Budget {
                steps: 8,
                max_depth: 128
            }
        ),
        Outcome::Unknown {
            reason: UnknownReason::BudgetExhausted,
            ..
        }
    ));
    proved(&context, proposition);
}

#[test]
fn proposition_validation_is_bounded_and_does_not_establish_the_goal() {
    use elaboration::ptc::ValidationError;
    let context = Context::default();
    let before = context.to_string();
    let goal = eq(ty(P::Bool), ty(P::U8));
    context
        .validate_formula(&goal, &mut Budget::default())
        .unwrap();
    context
        .validate_formula(&F::False, &mut Budget::default())
        .unwrap();
    assert!(matches!(
        context.prove(goal, Budget::default()),
        Outcome::NotEntailed(_)
    ));
    assert_eq!(context.to_string(), before);
    assert!(matches!(
        context.validate_formula(&eq(Term::Bool(true), ty(P::Bool)), &mut Budget::default()),
        Err(ValidationError::IllFormed(_))
    ));
    let scoped = eq(
        Term::Bound {
            depth: 0,
            index: 0,
            sort: Sort::Type,
        },
        ty(P::Bool),
    );
    assert!(matches!(
        context.validate_formula(&scoped, &mut Budget::default()),
        Err(ValidationError::IllFormed(_))
    ));
    Context::with_scope(vec![vec![Sort::Type]])
        .validate_formula(&scoped, &mut Budget::default())
        .unwrap();
    let mut deep = F::True;
    for _ in 0..140 {
        deep = deep.negate();
    }
    assert!(matches!(
        context.validate_formula(&deep, &mut Budget::default()),
        Err(ValidationError::BudgetExhausted)
    ));
    assert!(matches!(
        context.validate_formula(
            &F::And(vec![F::True; 100]),
            &mut Budget {
                steps: 4,
                max_depth: 128
            }
        ),
        Err(ValidationError::BudgetExhausted)
    ));
}

#[test]
fn closure_context_input_budget_failure_does_not_poison_later_proofs() {
    let context = Context::default();
    let fields = Term::ClosureContextValue(vec![ty(P::U8); 64]);
    let contextual = Term::contextual_closure(
        Term::NormalForm {
            head: DeclarationId {
                package: 1,
                local: 77,
            },
            sort: nir::Sort::ComptimeValue,
            arguments: vec![fields.clone()],
        },
        Term::Function {
            parameters: vec![],
            result: Box::new(ty(P::Bool)),
        },
    )
    .unwrap();
    for input in [head(7, vec![fields]), contextual] {
        let proposition = eq(input.clone(), input);
        assert!(matches!(
            context.prove(
                proposition.clone(),
                Budget {
                    steps: 8,
                    max_depth: 128
                }
            ),
            Outcome::Unknown {
                reason: UnknownReason::BudgetExhausted,
                ..
            }
        ));
        proved(&context, proposition);
    }
}

#[test]
fn unsafe_callable_constructor_keeps_identity_and_congruence() {
    let callable = |result| Term::Function {
        parameters: vec![],
        result: Box::new(result),
    };
    let unsafe_type = |result| Term::UnsafeFunction(Box::new(callable(result)));
    let mut context = Context::default();
    context.assume(eq(variable(701), ty(P::Bool)));
    proved(
        &context,
        eq(unsafe_type(variable(701)), unsafe_type(ty(P::Bool))),
    );
    for other in [
        callable(ty(P::Bool)),
        callable(ty(P::Bool)).with_comptime_phase(true),
    ] {
        proved(
            &context,
            F::Not(Box::new(eq(unsafe_type(ty(P::Bool)), other))),
        );
    }
    let mut injectivity = Context::default();
    injectivity.assume(eq(unsafe_type(variable(702)), unsafe_type(ty(P::Bool))));
    proved(&injectivity, eq(variable(702), ty(P::Bool)));
}
