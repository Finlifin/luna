use comptime_runtime::{Error as ArithmeticError, Integer};
use elaboration::verification::{
    Facts, Interval, Limits, Outcome, PlaceId, Polynomial, Unknown, ValueId,
};
fn interval(low: i128, high: i128) -> Interval {
    Interval::new(
        Integer::from_i128(low),
        Integer::from_i128(high),
        &mut Limits::default(),
    )
    .unwrap()
}
fn variable(
    facts: &mut Facts,
    local: u32,
    low: i128,
    high: i128,
    limits: &mut Limits,
) -> (ValueId, Polynomial) {
    let id = facts
        .assign(PlaceId { owner: 1, local }, interval(low, high))
        .unwrap();
    (id, Polynomial::variable(id, limits).unwrap())
}
fn constant(value: i128, limits: &mut Limits) -> Polynomial {
    Polynomial::constant(Integer::from_i128(value), limits).unwrap()
}

#[test]
fn three_dimensional_index_has_a_replayable_cubic_range_proof() {
    let mut limits = Limits::default();
    let mut facts = Facts::default();
    let (_, z) = variable(&mut facts, 0, 0, 2, &mut limits);
    let (_, height) = variable(&mut facts, 1, 3, 3, &mut limits);
    let (_, y) = variable(&mut facts, 2, 0, 2, &mut limits);
    let (_, width) = variable(&mut facts, 3, 4, 4, &mut limits);
    let (_, x) = variable(&mut facts, 4, 0, 3, &mut limits);
    let index = z
        .multiply(&height, &mut limits)
        .unwrap()
        .add(&y, &mut limits)
        .unwrap()
        .multiply(&width, &mut limits)
        .unwrap()
        .add(&x, &mut limits)
        .unwrap();
    let Outcome::Proven(proof) = index.prove_within(&interval(0, 35), &facts, &mut limits) else {
        panic!("cubic index range not established")
    };
    assert_eq!(proof.enclosure(), &interval(0, 35));
    proof.replay(&facts, &mut limits).unwrap();
    assert_eq!(
        index.prove_within(&interval(0, 34), &facts, &mut limits),
        Outcome::Unknown(Unknown::InsufficientBounds)
    );
    println!("{proof}");
}

#[test]
fn normalized_polynomials_preserve_correlations_and_deterministic_order() {
    let mut limits = Limits::default();
    let mut facts = Facts::default();
    let (_, x) = variable(&mut facts, 0, -4, 7, &mut limits);
    let (_, y) = variable(&mut facts, 1, -2, 5, &mut limits);
    let (_, z) = variable(&mut facts, 2, 0, 9, &mut limits);
    let xy = x.multiply(&y, &mut limits).unwrap();
    let yx = y.multiply(&x, &mut limits).unwrap();
    assert_eq!(xy, yx);
    assert_eq!(xy.to_string(), yx.to_string());
    let left = x
        .multiply(&y.add(&z, &mut limits).unwrap(), &mut limits)
        .unwrap();
    let right = xy
        .add(&x.multiply(&z, &mut limits).unwrap(), &mut limits)
        .unwrap();
    assert_eq!(left, right);
    let zero = left.subtract(&right, &mut limits).unwrap();
    assert_eq!(zero.to_string(), "(polynomial (terms))");
    assert!(matches!(
        zero.prove_within(&interval(0, 0), &facts, &mut limits),
        Outcome::Proven(_)
    ));
}

#[test]
fn sampled_signed_cubic_values_are_inside_the_proven_enclosure() {
    let mut limits = Limits::default();
    let mut facts = Facts::default();
    let (_, x) = variable(&mut facts, 0, -3, 4, &mut limits);
    let (_, y) = variable(&mut facts, 1, -2, 5, &mut limits);
    let cube = x
        .multiply(&x, &mut limits)
        .unwrap()
        .multiply(&x, &mut limits)
        .unwrap();
    let expression = cube
        .multiply(&constant(-3, &mut limits), &mut limits)
        .unwrap()
        .add(
            &x.multiply(&y, &mut limits)
                .unwrap()
                .multiply(&constant(2, &mut limits), &mut limits)
                .unwrap(),
            &mut limits,
        )
        .unwrap()
        .add(&constant(7, &mut limits), &mut limits)
        .unwrap();
    let Outcome::Proven(proof) =
        expression.prove_within(&interval(-10000, 10000), &facts, &mut limits)
    else {
        panic!("bounded polynomial")
    };
    for x in -3i128..=4 {
        for y in -2i128..=5 {
            let exact = Integer::from_i128(-3 * x * x * x + 2 * x * y + 7);
            assert!(&exact >= proof.enclosure().lower() && &exact <= proof.enclosure().upper());
        }
    }
}

#[test]
fn assignments_invalidate_only_the_used_versions_and_replay_rechecks_premises() {
    let mut limits = Limits::default();
    let mut facts = Facts::default();
    let (old, x) = variable(&mut facts, 0, 0, 10, &mut limits);
    let Outcome::Proven(proof) = x.prove_within(&interval(0, 10), &facts, &mut limits) else {
        panic!("bound")
    };
    variable(&mut facts, 1, -100, 100, &mut limits);
    proof.replay(&facts, &mut limits).unwrap();
    let new = facts.assign(old.place, interval(0, 10)).unwrap();
    assert_ne!(old, new);
    assert_eq!(
        proof.replay(&facts, &mut limits),
        Err(Unknown::StaleValue(old))
    );
    assert_eq!(
        x.prove_within(&interval(0, 10), &facts, &mut limits),
        Outcome::Unknown(Unknown::StaleValue(old))
    );
    let unavailable = facts.invalidate(old.place).unwrap();
    let new_expression = Polynomial::variable(unavailable, &mut limits).unwrap();
    assert_eq!(
        new_expression.prove_within(&interval(0, 10), &facts, &mut limits),
        Outcome::Unknown(Unknown::MissingBound(unavailable))
    );
    let current = facts.assign(old.place, interval(1, 2)).unwrap();
    assert!(matches!(
        Polynomial::variable(current, &mut limits)
            .unwrap()
            .prove_within(&interval(0, 10), &facts, &mut limits),
        Outcome::Proven(_)
    ));
}

#[test]
fn degree_expansion_size_and_work_limits_produce_no_partial_proof() {
    let mut limits = Limits::default();
    let mut facts = Facts::default();
    let (_, x) = variable(&mut facts, 0, 0, 2, &mut limits);
    let (_, y) = variable(&mut facts, 1, 0, 2, &mut limits);
    let square = x.multiply(&x, &mut limits).unwrap();
    let before = square.clone();
    assert_eq!(
        square.multiply(&square, &mut limits),
        Err(Unknown::DegreeLimit)
    );
    assert_eq!(square, before);
    let sum = x.add(&y, &mut limits).unwrap();
    let mut small = Limits {
        max_terms: 2,
        ..Limits::default()
    };
    assert_eq!(sum.multiply(&sum, &mut small), Err(Unknown::TermLimit));
    let mut exhausted = Limits::default();
    exhausted.arithmetic.steps = 0;
    assert_eq!(
        Polynomial::default().add(&Polynomial::default(), &mut exhausted),
        Err(Unknown::Arithmetic(ArithmeticError::BudgetExhausted))
    );

    assert_eq!(
        x.prove_within(&interval(0, 2), &facts, &mut exhausted),
        Outcome::Unknown(Unknown::Arithmetic(ArithmeticError::BudgetExhausted))
    );
    let Outcome::Proven(proof) = x.prove_within(&interval(0, 2), &facts, &mut limits) else {
        panic!("recovery")
    };
    assert_eq!(
        proof.replay(&facts, &mut exhausted),
        Err(Unknown::Arithmetic(ArithmeticError::BudgetExhausted))
    );
    proof.replay(&facts, &mut limits).unwrap();
    let mut small = Limits::default();
    small.arithmetic.max_limbs = 1;
    assert_eq!(
        Polynomial::constant(Integer::from_u128(u128::MAX), &mut small),
        Err(Unknown::Arithmetic(ArithmeticError::SizeLimit))
    );
    assert_eq!(
        Interval::new(Integer::from_u128(2), Integer::from_u128(1), &mut limits),
        Err(Unknown::InvalidInterval)
    );
}

#[test]
fn unknown_bounds_do_not_publish_an_arithmetic_or_type_fact() {
    let mut limits = Limits::default();
    let mut facts = Facts::default();
    let (_, x) = variable(&mut facts, 0, -2, 2, &mut limits);
    let square = x.multiply(&x, &mut limits).unwrap();
    assert!(matches!(
        square.prove_within(&interval(0, 4), &facts, &mut limits),
        Outcome::Proven(_)
    ));
    // Finite integer reasoning now establishes this correlation. A stronger
    // false bound must still remain unknown, without publishing a new fact.
    let correlated = square.add(&x, &mut limits).unwrap();
    assert!(matches!(
        correlated.prove_within(&interval(0, 6), &facts, &mut limits),
        Outcome::Proven(_)
    ));
    assert_eq!(
        correlated.prove_within(&interval(1, 6), &facts, &mut limits),
        Outcome::Unknown(Unknown::InsufficientBounds)
    );
    let other = ValueId {
        place: PlaceId { owner: 2, local: 0 },
        version: 0,
    };
    let difference = x
        .subtract(
            &Polynomial::variable(other, &mut limits).unwrap(),
            &mut limits,
        )
        .unwrap();
    assert_eq!(
        difference.prove_within(&interval(0, 0), &facts, &mut limits),
        Outcome::Unknown(Unknown::MissingBound(other))
    );
}

#[test]
fn single_variable_powers_cover_each_small_integer_interval_exactly() {
    let mut limits = Limits::default();
    for low in -4i128..=4 {
        for high in low..=4 {
            let mut facts = Facts::default();
            let (_, x) = variable(&mut facts, 0, low, high, &mut limits);
            let mut power = x.clone();
            for exponent in 1..=3 {
                let Outcome::Proven(proof) =
                    power.prove_within(&interval(-1000, 1000), &facts, &mut limits)
                else {
                    panic!("power bounds")
                };
                let values: Vec<_> = (low..=high).map(|x| x.pow(exponent)).collect();
                assert_eq!(
                    proof.enclosure(),
                    &interval(*values.iter().min().unwrap(), *values.iter().max().unwrap())
                );
                if exponent < 3 {
                    power = power.multiply(&x, &mut limits).unwrap();
                }
            }
        }
    }
}

#[test]
fn machine_range_obligations_use_mathematical_bounds_before_admitting_operations() {
    let mut limits = Limits::default();
    let mut facts = Facts::default();
    let byte = Interval::machine(8, false, &mut limits).unwrap();
    assert_eq!(byte, interval(0, 255));
    assert_eq!(
        Interval::machine(8, true, &mut limits).unwrap(),
        interval(-128, 127)
    );
    let full = Interval::machine(128, false, &mut limits).unwrap();
    assert_eq!(full.upper(), &Integer::from_u128(u128::MAX));
    assert_eq!(
        Interval::machine(128, true, &mut limits).unwrap().lower(),
        &Integer::from_i128(i128::MIN)
    );
    assert_eq!(
        Interval::machine(0, false, &mut limits),
        Err(Unknown::InvalidMachineWidth)
    );
    let (_, x) = variable(&mut facts, 0, 0, 200, &mut limits);
    let (_, y) = variable(&mut facts, 1, 0, 55, &mut limits);
    let sum = x.add(&y, &mut limits).unwrap();
    assert!(matches!(
        sum.prove_within(&byte, &facts, &mut limits),
        Outcome::Proven(_)
    ));
    let too_large = sum.add(&constant(1, &mut limits), &mut limits).unwrap();
    assert_eq!(
        too_large.prove_within(&byte, &facts, &mut limits),
        Outcome::Unknown(Unknown::InsufficientBounds)
    );
}

#[test]
fn comparisons_refine_both_truth_paths_against_exhaustive_small_integer_domains() {
    use elaboration::verification::Comparison as C;
    for relation in [
        C::Equal,
        C::NotEqual,
        C::Less,
        C::LessEqual,
        C::Greater,
        C::GreaterEqual,
    ] {
        for truth in [false, true] {
            for low in -3..=3 {
                for high in low..=3 {
                    for constant in -4..=4 {
                        let mut limits = Limits::default();
                        let mut facts = Facts::default();
                        let (id, _) = variable(&mut facts, 0, low, high, &mut limits);
                        facts
                            .assume_comparison(
                                id,
                                relation,
                                &Integer::from_i128(constant),
                                truth,
                                &mut limits,
                            )
                            .unwrap();
                        let values: Vec<_> = (low..=high)
                            .filter(|x| {
                                (match relation {
                                    C::Equal => *x == constant,
                                    C::NotEqual => *x != constant,
                                    C::Less => *x < constant,
                                    C::LessEqual => *x <= constant,
                                    C::Greater => *x > constant,
                                    C::GreaterEqual => *x >= constant,
                                }) == truth
                            })
                            .collect();
                        assert_eq!(
                            facts.is_reachable(),
                            !values.is_empty(),
                            "{low}..{high} {relation} {constant} = {truth}"
                        );
                        if !values.is_empty() {
                            assert_eq!(facts.value(id.place), Some(id));
                            assert_eq!(
                                facts.bound(id, &mut limits).unwrap(),
                                &interval(values[0], *values.last().unwrap())
                            );
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn sibling_writes_and_merge_values_never_reuse_each_others_identity() {
    let mut limits = Limits::default();
    let mut root = Facts::default();
    let (old, _) = variable(&mut root, 0, 0, 255, &mut limits);
    let mut left = root.fork(&mut limits).unwrap();
    let mut right = root.fork(&mut limits).unwrap();
    let a = left.assign(old.place, interval(1, 2)).unwrap();
    let b = right.assign(old.place, interval(1, 2)).unwrap();
    assert_ne!(a, b);
    let x = Polynomial::variable(a, &mut limits).unwrap();
    let Outcome::Proven(proof) = x.prove_within(&interval(0, 10), &left, &mut limits) else {
        panic!("bound")
    };
    assert_eq!(
        proof.replay(&right, &mut limits),
        Err(Unknown::StaleValue(a))
    );
    let joined = left.join(&right, &mut limits).unwrap();
    let merged = joined.value(old.place).unwrap();
    assert_ne!(merged, a);
    assert_ne!(merged, b);
    assert_eq!(joined.bound(merged, &mut limits).unwrap(), &interval(1, 2));
    assert_eq!(
        proof.replay(&joined, &mut limits),
        Err(Unknown::StaleValue(a))
    );
    assert_eq!(root.bound(old, &mut limits).unwrap(), &interval(0, 255));
}

#[test]
fn merge_widens_branch_bounds_and_discards_unknown_or_missing_facts() {
    use elaboration::verification::Comparison as C;
    let mut limits = Limits::default();
    let mut root = Facts::default();
    let (id, x) = variable(&mut root, 0, 0, 255, &mut limits);
    let mut left = root.fork(&mut limits).unwrap();
    let mut right = root.fork(&mut limits).unwrap();
    left.assume_comparison(id, C::Less, &Integer::from_u128(100), true, &mut limits)
        .unwrap();
    right
        .assume_comparison(id, C::Less, &Integer::from_u128(100), false, &mut limits)
        .unwrap();
    let Outcome::Proven(proof) = x.prove_within(&interval(0, 99), &left, &mut limits) else {
        panic!("narrowed")
    };
    let joined = left.join(&right, &mut limits).unwrap();
    assert_eq!(joined.value(id.place), Some(id));
    assert_eq!(joined.bound(id, &mut limits).unwrap(), &interval(0, 255));
    assert_eq!(
        proof.replay(&joined, &mut limits),
        Err(Unknown::StaleValue(id))
    );
    right.invalidate(id.place).unwrap();
    let only_left = left
        .assign(PlaceId { owner: 1, local: 1 }, interval(3, 3))
        .unwrap();
    let joined = left.join(&right, &mut limits).unwrap();
    for place in [id.place, only_left.place] {
        let merged = joined.value(place).unwrap();
        assert_eq!(
            joined.bound(merged, &mut limits),
            Err(Unknown::MissingBound(merged))
        );
    }
}

#[test]
fn contradictory_paths_cannot_publish_proofs_and_do_not_poison_live_merges() {
    let mut limits = Limits::default();
    let mut root = Facts::default();
    let (id, x) = variable(&mut root, 0, 0, 10, &mut limits);
    let mut dead = root.fork(&mut limits).unwrap();
    dead.refine(id, Some(&Integer::from_i128(11)), None, &mut limits)
        .unwrap();
    assert!(!dead.is_reachable());
    // Even a constant theorem does not turn a dead path into an executable proof.
    assert_eq!(
        constant(0, &mut limits).prove_within(&interval(0, 0), &dead, &mut limits),
        Outcome::Unknown(Unknown::UnreachablePath)
    );
    assert_eq!(
        dead.assign(id.place, interval(0, 0)),
        Err(Unknown::UnreachablePath)
    );
    for joined in [
        dead.join(&root, &mut limits).unwrap(),
        root.join(&dead, &mut limits).unwrap(),
    ] {
        assert!(joined.is_reachable());
        assert_eq!(joined.value(id.place), Some(id));
        assert!(matches!(
            x.prove_within(&interval(0, 10), &joined, &mut limits),
            Outcome::Proven(_)
        ));
    }
    assert!(!dead.join(&dead, &mut limits).unwrap().is_reachable());
}

#[test]
fn refinement_and_join_budget_failures_preserve_inputs_and_allow_retry() {
    let mut limits = Limits::default();
    let mut root = Facts::default();
    let (id, _) = variable(&mut root, 0, 0, 255, &mut limits);
    let before = root.to_string();
    for steps in 0..12 {
        let mut exhausted = Limits::default();
        exhausted.arithmetic.steps = steps;
        let result = root.refine(
            id,
            Some(&Integer::from_u128(100)),
            Some(&Integer::from_u128(200)),
            &mut exhausted,
        );
        if result.is_err() {
            assert_eq!(root.to_string(), before);
        } else {
            break;
        }
    }
    let mut other = root.fork(&mut limits).unwrap();
    other.assign(id.place, interval(0, 99)).unwrap();
    let before_root = root.to_string();
    let before_other = other.to_string();
    let mut exhausted = Limits::default();
    exhausted.arithmetic.steps = 0;
    assert!(root.fork(&mut exhausted).is_err());
    assert!(root.join(&other, &mut exhausted).is_err());
    assert_eq!(root.to_string(), before_root);
    assert_eq!(other.to_string(), before_other);
    let joined = root.join(&other, &mut limits).unwrap();
    assert!(joined.is_reachable());
}

#[test]
fn unrelated_analysis_cannot_merge_or_replay_proofs_even_with_identical_ids() {
    let mut limits = Limits::default();
    let mut first = Facts::default();
    let mut second = Facts::default();
    let (a, x) = variable(&mut first, 0, 0, 10, &mut limits);
    let (b, _) = variable(&mut second, 0, 0, 10, &mut limits);
    assert_eq!(a, b);
    assert!(matches!(
        first.join(&second, &mut limits),
        Err(Unknown::UnrelatedFacts)
    ));
    let Outcome::Proven(proof) = x.prove_within(&interval(0, 10), &first, &mut limits) else {
        panic!("proof")
    };
    assert_eq!(
        proof.replay(&second, &mut limits),
        Err(Unknown::UnrelatedFacts)
    );
    proof
        .replay(&first.fork(&mut limits).unwrap(), &mut limits)
        .unwrap();
}

#[test]
fn boolean_condition_paths_enclose_every_concrete_solution_without_cross_branch_leaks() {
    use elaboration::verification::{Comparison as C, Condition as Q};
    fn compare(value: ValueId, relation: C, constant: i128) -> Q {
        Q::Compare {
            value,
            relation,
            constant: Integer::from_i128(constant),
        }
    }
    fn evaluate(q: &Q, x: i128, y: i128) -> bool {
        match q {
            Q::Bool(b) => *b,
            Q::PolynomialCompare { .. } => unreachable!("scalar-only fixture"),
            Q::Not(q) => !evaluate(q, x, y),
            Q::And(a, b) => evaluate(a, x, y) && evaluate(b, x, y),
            Q::Or(a, b) => evaluate(a, x, y) || evaluate(b, x, y),
            Q::Compare {
                value,
                relation,
                constant,
            } => {
                let n = Integer::from_i128(if value.place.local == 0 { x } else { y });
                match relation {
                    C::Equal => n == *constant,
                    C::NotEqual => n != *constant,
                    C::Less => n < *constant,
                    C::LessEqual => n <= *constant,
                    C::Greater => n > *constant,
                    C::GreaterEqual => n >= *constant,
                }
            }
        }
    }
    let mut limits = Limits::default();
    let mut root = Facts::default();
    let (x, _) = variable(&mut root, 0, -4, 4, &mut limits);
    let (y, _) = variable(&mut root, 1, -4, 4, &mut limits);
    let a = compare(x, C::Less, 0);
    let b = compare(y, C::GreaterEqual, 2);
    let conditions = [
        Q::And(Box::new(a.clone()), Box::new(b.clone())),
        Q::Or(Box::new(a.clone()), Box::new(b.clone())),
        Q::Not(Box::new(Q::And(Box::new(a.clone()), Box::new(b.clone())))),
        Q::Or(
            Box::new(Q::And(Box::new(a.clone()), Box::new(b.clone()))),
            Box::new(compare(x, C::Greater, 2)),
        ),
        Q::And(Box::new(a.clone()), Box::new(Q::Not(Box::new(a.clone())))),
        Q::Or(Box::new(a.clone()), Box::new(Q::Not(Box::new(a)))),
    ];
    for condition in conditions {
        for truth in [false, true] {
            let mut branch = root.fork(&mut limits).unwrap();
            branch.assume(&condition, truth, &mut limits).unwrap();
            for xv in -4..=4 {
                for yv in -4..=4 {
                    if evaluate(&condition, xv, yv) != truth {
                        continue;
                    }
                    assert!(branch.is_reachable(), "{condition} = {truth}");
                    for (id, value) in [(x, xv), (y, yv)] {
                        let bound = branch.bound(id, &mut limits).unwrap();
                        let value = Integer::from_i128(value);
                        assert!(
                            &value >= bound.lower() && &value <= bound.upper(),
                            "{condition} = {truth}: {branch}"
                        );
                    }
                }
            }
        }
    }
    let mut disjunction = root.fork(&mut limits).unwrap();
    disjunction
        .assume(
            &Q::Or(
                Box::new(compare(x, C::Less, 0)),
                Box::new(compare(y, C::GreaterEqual, 2)),
            ),
            true,
            &mut limits,
        )
        .unwrap();
    assert_eq!(disjunction.bound(x, &mut limits).unwrap(), &interval(-4, 4));
    assert_eq!(disjunction.bound(y, &mut limits).unwrap(), &interval(-4, 4));
    println!("{disjunction}");
}

#[test]
fn compound_condition_failure_is_transactional_and_stale_values_cannot_refine() {
    use elaboration::verification::{Comparison as C, Condition as Q};
    let mut limits = Limits::default();
    let mut facts = Facts::default();
    let (old, _) = variable(&mut facts, 0, 0, 255, &mut limits);
    let current = facts.assign(old.place, interval(0, 255)).unwrap();
    let condition = Q::And(
        Box::new(Q::Compare {
            value: current,
            relation: C::Less,
            constant: Integer::from_u128(100),
        }),
        Box::new(Q::Compare {
            value: old,
            relation: C::Greater,
            constant: Integer::zero(),
        }),
    );
    let before = facts.to_string();
    assert_eq!(
        facts.assume(&condition, true, &mut limits),
        Err(Unknown::StaleValue(old))
    );
    assert_eq!(facts.to_string(), before);
    let mut deep = Q::Bool(true);
    for _ in 0..130 {
        deep = Q::Not(Box::new(deep));
    }
    assert_eq!(
        facts.assume(&deep, true, &mut limits),
        Err(Unknown::ConditionDepthLimit)
    );
    assert_eq!(facts.to_string(), before);
    let valid = Q::Compare {
        value: current,
        relation: C::Less,
        constant: Integer::from_u128(100),
    };
    for steps in 0..30 {
        let mut branch = facts.fork(&mut limits).unwrap();
        let mut small = Limits::default();
        small.arithmetic.steps = steps;
        if branch.assume(&valid, true, &mut small).is_err() {
            assert_eq!(branch.to_string(), before);
        }
    }
    let mut small = Limits::default();
    small.arithmetic.max_limbs = 1;
    assert_eq!(
        facts.refine(
            current,
            Some(&Integer::from_u128(u128::MAX)),
            None,
            &mut small
        ),
        Err(Unknown::Arithmetic(ArithmeticError::SizeLimit))
    );
    assert_eq!(facts.to_string(), before);
    facts.assume(&valid, true, &mut limits).unwrap();
    assert_eq!(facts.bound(current, &mut limits).unwrap(), &interval(0, 99));
}

#[test]
fn checked_relations_propagate_bounds_and_prove_correlated_differences() {
    use elaboration::verification::{Comparison as C, Condition as Q};
    let mut limits = Limits::default();
    let mut facts = Facts::default();
    let (xi, x) = variable(&mut facts, 0, 0, 255, &mut limits);
    let (yi, y) = variable(&mut facts, 1, 0, 255, &mut limits);
    let condition = Q::PolynomialCompare {
        left: x.clone(),
        relation: C::Less,
        right: y.clone(),
    };
    facts.assume(&condition, true, &mut limits).unwrap();
    assert_eq!(facts.bound(xi, &mut limits).unwrap(), &interval(0, 254));
    assert_eq!(facts.bound(yi, &mut limits).unwrap(), &interval(1, 255));
    let difference = y.subtract(&x, &mut limits).unwrap();
    let Outcome::Proven(proof) = difference.prove_within(&interval(1, 255), &facts, &mut limits)
    else {
        panic!("relational difference")
    };
    proof.replay(&facts, &mut limits).unwrap();
    println!("{proof}");
    let mut changed = facts.fork(&mut limits).unwrap();
    changed.assign(xi.place, interval(0, 254)).unwrap();
    assert!(proof.replay(&changed, &mut limits).is_err());
    facts
        .assume(
            &Q::Compare {
                value: yi,
                relation: C::LessEqual,
                constant: Integer::from_i128(100),
            },
            true,
            &mut limits,
        )
        .unwrap();
    assert_eq!(facts.bound(xi, &mut limits).unwrap(), &interval(0, 99));
}

#[test]
fn checked_relations_enclose_all_small_concrete_solutions_on_both_truth_paths() {
    use elaboration::verification::{Comparison as C, Condition as Q};
    for relation in [
        C::Less,
        C::LessEqual,
        C::Greater,
        C::GreaterEqual,
        C::Equal,
        C::NotEqual,
    ] {
        for truth in [false, true] {
            let mut limits = Limits::default();
            let mut facts = Facts::default();
            let (xi, x) = variable(&mut facts, 0, -3, 4, &mut limits);
            let (yi, y) = variable(&mut facts, 1, -2, 5, &mut limits);
            facts
                .assume(
                    &Q::PolynomialCompare {
                        left: x.clone(),
                        relation,
                        right: y.clone(),
                    },
                    truth,
                    &mut limits,
                )
                .unwrap();
            let p = y.subtract(&x, &mut limits).unwrap();
            let Outcome::Proven(proof) = p.prove_within(&interval(-20, 20), &facts, &mut limits)
            else {
                panic!("enclosure")
            };
            for a in -3..=4 {
                for b in -2..=5 {
                    let holds = match relation {
                        C::Less => a < b,
                        C::LessEqual => a <= b,
                        C::Greater => a > b,
                        C::GreaterEqual => a >= b,
                        C::Equal => a == b,
                        C::NotEqual => a != b,
                    };
                    if holds != truth {
                        continue;
                    }
                    for (id, n) in [(xi, a), (yi, b)] {
                        let bound = facts.bound(id, &mut limits).unwrap();
                        assert!(
                            bound.lower() <= &Integer::from_i128(n)
                                && bound.upper() >= &Integer::from_i128(n)
                        );
                    }
                    let n = Integer::from_i128(b - a);
                    assert!(proof.enclosure().lower() <= &n && proof.enclosure().upper() >= &n);
                }
            }
        }
    }
}

#[test]
fn checked_polynomial_sum_and_cubic_rows_are_versioned_and_budgeted() {
    use elaboration::verification::{Comparison as C, Condition as Q};
    let mut limits = Limits::default();
    let mut facts = Facts::default();
    let (xi, x) = variable(&mut facts, 0, 0, 10, &mut limits);
    let (_, y) = variable(&mut facts, 1, 0, 10, &mut limits);
    let (_, z) = variable(&mut facts, 2, 0, 10, &mut limits);
    let sum = x.add(&y, &mut limits).unwrap();
    let constraint = Q::PolynomialCompare {
        left: sum.clone(),
        relation: C::LessEqual,
        right: constant(12, &mut limits),
    };
    let before = facts.to_string();
    let mut tiny = Limits::default();
    tiny.arithmetic.steps = 20;
    assert!(facts.assume(&constraint, true, &mut tiny).is_err());
    assert_eq!(before, facts.to_string());
    facts.assume(&constraint, true, &mut limits).unwrap();
    assert!(matches!(
        sum.prove_within(&interval(0, 12), &facts, &mut limits),
        Outcome::Proven(_)
    ));
    let cube = x
        .multiply(&y, &mut limits)
        .unwrap()
        .multiply(&z, &mut limits)
        .unwrap();
    facts
        .assume(
            &Q::PolynomialCompare {
                left: cube.clone(),
                relation: C::LessEqual,
                right: constant(200, &mut limits),
            },
            true,
            &mut limits,
        )
        .unwrap();
    let Outcome::Proven(proof) = cube.prove_within(&interval(0, 200), &facts, &mut limits) else {
        panic!("cubic row")
    };
    proof.replay(&facts, &mut limits).unwrap();
    facts.invalidate(xi.place).unwrap();
    assert!(proof.replay(&facts, &mut limits).is_err());
}

#[test]
fn relational_rows_do_not_cross_alternatives_or_resurrect_after_writes() {
    use elaboration::verification::{Comparison as C, Condition as Q};
    let mut limits = Limits::default();
    let mut root = Facts::default();
    let (xi, x) = variable(&mut root, 0, 0, 255, &mut limits);
    let (_, y) = variable(&mut root, 1, 0, 255, &mut limits);
    let condition = Q::PolynomialCompare {
        left: x.clone(),
        relation: C::LessEqual,
        right: y.clone(),
    };
    let mut yes = root.fork(&mut limits).unwrap();
    yes.assume(&condition, true, &mut limits).unwrap();
    let joined = yes.join(&root, &mut limits).unwrap();
    let p = y.subtract(&x, &mut limits).unwrap();
    let Outcome::Proven(original) = p.prove_within(&interval(0, 255), &yes, &mut limits) else {
        panic!("relation establishes difference bound");
    };
    assert_eq!(
        original.replay(&joined, &mut limits),
        Err(Unknown::InsufficientBounds)
    );
    assert!(matches!(
        p.prove_within(&interval(0, 255), &joined, &mut limits),
        Outcome::Unknown(_)
    ));
    let mut other = root.fork(&mut limits).unwrap();
    other.assume(&condition, true, &mut limits).unwrap();
    let joined = yes.join(&other, &mut limits).unwrap();
    assert!(matches!(
        p.prove_within(&interval(0, 255), &joined, &mut limits),
        Outcome::Proven(_)
    ));
    yes.assign(xi.place, interval(255, 255)).unwrap();
    let mut retry = Limits::default();
    assert!(matches!(
        p.prove_within(&interval(0, 255), &yes, &mut retry),
        Outcome::Unknown(_)
    ));
}

#[test]
fn relational_contraction_exhaustion_is_transactional_and_contradictions_are_not_proofs() {
    use elaboration::verification::{Comparison as C, Condition as Q};
    let mut limits = Limits::default();
    let mut facts = Facts::default();
    let (_, x) = variable(&mut facts, 0, 0, i128::MAX, &mut limits);
    let (_, y) = variable(&mut facts, 1, 0, i128::MAX, &mut limits);
    let condition = Q::And(
        Box::new(Q::PolynomialCompare {
            left: x.clone(),
            relation: C::Less,
            right: y.clone(),
        }),
        Box::new(Q::PolynomialCompare {
            left: y.clone(),
            relation: C::Less,
            right: x.clone(),
        }),
    );
    let before = facts.to_string();
    let mut tiny = Limits::default();
    tiny.arithmetic.steps = 2000;
    assert_eq!(
        facts.assume(&condition, true, &mut tiny),
        Err(Unknown::Arithmetic(ArithmeticError::BudgetExhausted))
    );
    assert_eq!(before, facts.to_string());
    // Small domains reach a contradiction within budget, rather than becoming
    // a successful proof authorizing code on the impossible path.
    let mut small = Facts::default();
    let (_, a) = variable(&mut small, 0, 0, 3, &mut limits);
    let (_, b) = variable(&mut small, 1, 0, 3, &mut limits);
    small
        .assume(
            &Q::And(
                Box::new(Q::PolynomialCompare {
                    left: a.clone(),
                    relation: C::Less,
                    right: b.clone(),
                }),
                Box::new(Q::PolynomialCompare {
                    left: b,
                    relation: C::Less,
                    right: a,
                }),
            ),
            true,
            &mut limits,
        )
        .unwrap();
    assert!(!small.is_reachable());
    assert_eq!(
        constant(0, &mut limits).prove_within(&interval(0, 0), &small, &mut limits),
        Outcome::Unknown(Unknown::UnreachablePath)
    );
}

#[test]
fn difference_constraints_prove_transitive_ranges_and_scaled_strict_bounds() {
    use elaboration::verification::{Comparison as C, Condition as Q};
    let mut limits = Limits::default();
    let mut facts = Facts::default();
    let (ai, a) = variable(&mut facts, 0, 0, 255, &mut limits);
    let (_, b) = variable(&mut facts, 1, 0, 255, &mut limits);
    let (_, c) = variable(&mut facts, 2, 0, 255, &mut limits);
    for (left, right) in [(a.clone(), b.clone()), (b.clone(), c.clone())] {
        facts
            .assume(
                &Q::PolynomialCompare {
                    left,
                    relation: C::Less,
                    right,
                },
                true,
                &mut limits,
            )
            .unwrap();
    }
    let gap = c.subtract(&a, &mut limits).unwrap();
    let Outcome::Proven(proof) = gap.prove_within(&interval(2, 255), &facts, &mut limits) else {
        panic!("transitive gap")
    };
    proof.replay(&facts, &mut limits).unwrap();
    println!("{proof}");
    assert_eq!(facts.bound(ai, &mut limits).unwrap(), &interval(0, 253));
    let scaled = gap
        .multiply(&constant(3, &mut limits), &mut limits)
        .unwrap();
    assert!(matches!(
        scaled.prove_within(&interval(6, 765), &facts, &mut limits),
        Outcome::Proven(_)
    ));
    // 2*a <= 2*c-1 implies a-c <= floor(-1/2) = -1.
    let mut root = Facts::default();
    let (_, x) = variable(&mut root, 0, -10, 10, &mut limits);
    let (_, y) = variable(&mut root, 1, -10, 10, &mut limits);
    let two = constant(2, &mut limits);
    root.assume(
        &Q::PolynomialCompare {
            left: x.multiply(&two, &mut limits).unwrap(),
            relation: C::LessEqual,
            right: y
                .multiply(&two, &mut limits)
                .unwrap()
                .subtract(&constant(1, &mut limits), &mut limits)
                .unwrap(),
        },
        true,
        &mut limits,
    )
    .unwrap();
    assert!(matches!(
        y.subtract(&x, &mut limits)
            .unwrap()
            .prove_within(&interval(1, 20), &root, &mut limits),
        Outcome::Proven(_)
    ));
}

#[test]
fn coefficient_isolation_rounds_outward_for_every_small_signed_boundary() {
    use elaboration::verification::{Comparison as C, Condition as Q};
    for coefficient in -5i128..=5 {
        if coefficient == 0 {
            continue;
        }
        for bound in -12i128..=12 {
            for relation in [
                C::Less,
                C::LessEqual,
                C::Greater,
                C::GreaterEqual,
                C::Equal,
                C::NotEqual,
            ] {
                let mut limits = Limits::default();
                let mut facts = Facts::default();
                let (id, x) = variable(&mut facts, 0, -10, 10, &mut limits);
                let left = x
                    .multiply(&constant(coefficient, &mut limits), &mut limits)
                    .unwrap();
                facts
                    .assume(
                        &Q::PolynomialCompare {
                            left,
                            relation,
                            right: constant(bound, &mut limits),
                        },
                        true,
                        &mut limits,
                    )
                    .unwrap();
                let solutions: Vec<_> = (-10i128..=10)
                    .filter(|x| match relation {
                        C::Less => coefficient * x < bound,
                        C::LessEqual => coefficient * x <= bound,
                        C::Greater => coefficient * x > bound,
                        C::GreaterEqual => coefficient * x >= bound,
                        C::Equal => coefficient * x == bound,
                        C::NotEqual => coefficient * x != bound,
                    })
                    .collect();
                if solutions.is_empty() {
                    assert!(!facts.is_reachable(), "{coefficient} {relation} {bound}");
                    continue;
                }
                let range = facts.bound(id, &mut limits).unwrap();
                for value in &solutions {
                    let value = Integer::from_i128(*value);
                    assert!(range.lower() <= &value && range.upper() >= &value);
                }
                if relation != C::NotEqual {
                    assert_eq!(
                        range,
                        &interval(*solutions.first().unwrap(), *solutions.last().unwrap()),
                        "{coefficient} {relation} {bound}"
                    );
                }
            }
        }
    }
}

#[test]
fn transitive_proofs_reject_lost_intermediate_relations_and_versions() {
    use elaboration::verification::{Comparison as C, Condition as Q};
    let mut limits = Limits::default();
    let mut root = Facts::default();
    let (_, a) = variable(&mut root, 0, 0, 255, &mut limits);
    let (bi, b) = variable(&mut root, 1, 0, 255, &mut limits);
    let (_, c) = variable(&mut root, 2, 0, 255, &mut limits);
    root.assume(
        &Q::PolynomialCompare {
            left: a.clone(),
            relation: C::LessEqual,
            right: b.clone(),
        },
        true,
        &mut limits,
    )
    .unwrap();
    let mut facts = root.fork(&mut limits).unwrap();
    facts
        .assume(
            &Q::PolynomialCompare {
                left: b.clone(),
                relation: C::LessEqual,
                right: c.clone(),
            },
            true,
            &mut limits,
        )
        .unwrap();
    let gap = c.subtract(&a, &mut limits).unwrap();
    let Outcome::Proven(proof) = gap.prove_within(&interval(0, 255), &facts, &mut limits) else {
        panic!("transitive proof")
    };
    assert!(proof.replay(&root, &mut limits).is_err());
    assert!(
        proof
            .replay(&root.join(&facts, &mut limits).unwrap(), &mut limits)
            .is_err()
    );
    facts.assign(bi.place, interval(0, 255)).unwrap();
    assert!(proof.replay(&facts, &mut limits).is_err());
    assert!(matches!(
        gap.prove_within(&interval(0, 255), &facts, &mut limits),
        Outcome::Unknown(_)
    ));
}

#[test]
fn coefficient_rounding_never_truncates_to_host_integer_width() {
    use elaboration::verification::{Comparison as C, Condition as Q};
    for negative in [false, true] {
        let mut limits = Limits::default();
        let mut facts = Facts::default();
        let (id, x) = variable(&mut facts, 0, -10, 10, &mut limits);
        let big = Integer::from_u128(u128::MAX)
            .multiply(&Integer::from_u128(u128::MAX), &mut limits.arithmetic)
            .unwrap();
        let bound = big
            .multiply(&Integer::from_u128(2), &mut limits.arithmetic)
            .unwrap()
            .add(&Integer::from_u128(1), &mut limits.arithmetic)
            .unwrap();
        let coefficient = if negative {
            big.negate(&mut limits.arithmetic).unwrap()
        } else {
            big
        };
        let left = x
            .multiply(
                &Polynomial::constant(coefficient, &mut limits).unwrap(),
                &mut limits,
            )
            .unwrap();
        facts
            .assume(
                &Q::PolynomialCompare {
                    left,
                    relation: C::LessEqual,
                    right: Polynomial::constant(bound, &mut limits).unwrap(),
                },
                true,
                &mut limits,
            )
            .unwrap();
        assert_eq!(
            facts.bound(id, &mut limits).unwrap(),
            &if negative {
                interval(-2, 10)
            } else {
                interval(-10, 2)
            }
        );
    }
}

#[test]
fn transitive_cycles_over_large_domains_are_detected_without_endpoint_iteration() {
    use elaboration::verification::{Comparison as C, Condition as Q};
    let mut limits = Limits::default();
    let mut facts = Facts::default();
    let (_, a) = variable(&mut facts, 0, 0, i128::MAX, &mut limits);
    let (_, b) = variable(&mut facts, 1, 0, i128::MAX, &mut limits);
    let (_, c) = variable(&mut facts, 2, 0, i128::MAX, &mut limits);
    for (left, right, relation) in [
        (a.clone(), b.clone(), C::LessEqual),
        (b.clone(), c.clone(), C::LessEqual),
        (c, a, C::Less),
    ] {
        facts
            .assume(
                &Q::PolynomialCompare {
                    left,
                    relation,
                    right,
                },
                true,
                &mut limits,
            )
            .unwrap();
    }
    assert!(!facts.is_reachable());
}

#[test]
fn joining_reassigned_values_preserves_common_differences_without_reusing_old_ids() {
    for right_end in [99, 101] {
        let mut limits = Limits::default();
        let mut root = Facts::default();
        let (old_x, _) = variable(&mut root, 0, 0, 255, &mut limits);
        let (old_y, _) = variable(&mut root, 1, 0, 255, &mut limits);
        let mut left = root.fork(&mut limits).unwrap();
        let mut right = root.fork(&mut limits).unwrap();
        let lx = left.assign(old_x.place, interval(1, 1)).unwrap();
        let ly = left.assign(old_y.place, interval(2, 2)).unwrap();
        right.assign(old_x.place, interval(100, 100)).unwrap();
        right
            .assign(old_y.place, interval(right_end, right_end))
            .unwrap();
        let left_dump = left.to_string();
        let right_dump = right.to_string();
        let mut tiny = Limits::default();
        tiny.arithmetic.steps = 20;
        assert!(left.join(&right, &mut tiny).is_err());
        assert_eq!(left_dump, left.to_string());
        assert_eq!(right_dump, right.to_string());
        let joined = left.join(&right, &mut limits).unwrap();
        let x = joined.value(old_x.place).unwrap();
        let y = joined.value(old_y.place).unwrap();
        assert_ne!(x, lx);
        assert_ne!(y, ly);
        let difference = Polynomial::variable(y, &mut limits)
            .unwrap()
            .subtract(&Polynomial::variable(x, &mut limits).unwrap(), &mut limits)
            .unwrap();
        let expected = interval(if right_end == 101 { 1 } else { -1 }, 1);
        let Outcome::Proven(proof) = difference.prove_within(&expected, &joined, &mut limits)
        else {
            panic!("common difference")
        };
        assert_eq!(proof.enclosure(), &expected);
        proof.replay(&joined, &mut limits).unwrap();
        if right_end == 99 {
            assert!(matches!(
                difference.prove_within(&interval(0, 255), &joined, &mut limits),
                Outcome::Unknown(_)
            ));
        }
        let before = Polynomial::variable(ly, &mut limits)
            .unwrap()
            .subtract(&Polynomial::variable(lx, &mut limits).unwrap(), &mut limits)
            .unwrap();
        let Outcome::Proven(old_proof) = before.prove_within(&interval(1, 1), &left, &mut limits)
        else {
            panic!("old point proof")
        };
        assert!(old_proof.replay(&joined, &mut limits).is_err());
    }
}

#[test]
fn composed_differences_prove_sums_shared_coefficients_and_nonlinear_remainders() {
    use elaboration::verification::{Comparison as C, Condition as Q};
    let mut limits = Limits::default();
    let mut facts = Facts::default();
    let (aid, a) = variable(&mut facts, 0, 0, 100, &mut limits);
    let (_, b) = variable(&mut facts, 1, 0, 100, &mut limits);
    let (_, c) = variable(&mut facts, 2, 0, 100, &mut limits);
    let (_, d) = variable(&mut facts, 3, 0, 100, &mut limits);
    let (_, z) = variable(&mut facts, 4, -2, 2, &mut limits);
    for (left, right) in [
        (c.clone(), a.clone()),
        (d.clone(), b.clone()),
        (d.clone(), a.clone()),
    ] {
        facts
            .assume(
                &Q::PolynomialCompare {
                    left,
                    relation: C::LessEqual,
                    right,
                },
                true,
                &mut limits,
            )
            .unwrap();
    }
    // Both relations are required: bounding only one pair leaves a negative lower bound.
    let independent = a
        .add(&b, &mut limits)
        .unwrap()
        .subtract(&c, &mut limits)
        .unwrap()
        .subtract(&d, &mut limits)
        .unwrap();
    // A single positive coefficient must be split across two distinct edges.
    let shared = a
        .multiply(&constant(2, &mut limits), &mut limits)
        .unwrap()
        .subtract(&c, &mut limits)
        .unwrap()
        .subtract(&d, &mut limits)
        .unwrap();
    let square = z.multiply(&z, &mut limits).unwrap();
    for p in [independent, shared] {
        let p = p
            .add(&square, &mut limits)
            .unwrap()
            .add(&constant(3, &mut limits), &mut limits)
            .unwrap();
        for (query, expected) in [
            (p.clone(), interval(3, 207)),
            (
                Polynomial::default().subtract(&p, &mut limits).unwrap(),
                interval(-207, -3),
            ),
        ] {
            let Outcome::Proven(proof) = query.prove_within(&expected, &facts, &mut limits) else {
                panic!("combined edges must prove {query}")
            };
            assert_eq!(proof.enclosure(), &expected);
            proof.replay(&facts, &mut limits).unwrap();
            let mut changed = facts.fork(&mut limits).unwrap();
            changed.assign(aid.place, interval(0, 100)).unwrap();
            assert!(proof.replay(&changed, &mut limits).is_err());
        }
    }
}

#[test]
fn composed_differences_enclosures_cover_exhaustive_small_weighted_models() {
    use elaboration::verification::{Comparison as C, Condition as Q};
    let mut limits = Limits::default();
    let mut facts = Facts::default();
    let values: Vec<_> = (0..4)
        .map(|i| variable(&mut facts, i, -2, 2, &mut limits).1)
        .collect();
    // Overlapping edges with nonzero offsets exercise both signs and choices
    // of pairing, rather than comparing against another implementation.
    for (a, b, offset) in [(0, 2, 1), (0, 3, 0), (1, 2, -1), (1, 3, 2)] {
        let rhs = values[b]
            .add(&constant(offset, &mut limits), &mut limits)
            .unwrap();
        facts
            .assume(
                &Q::PolynomialCompare {
                    left: values[a].clone(),
                    relation: C::LessEqual,
                    right: rhs,
                },
                true,
                &mut limits,
            )
            .unwrap();
    }
    let models: Vec<_> = (-2..=2)
        .flat_map(|a| {
            (-2..=2)
                .flat_map(move |b| (-2..=2).flat_map(move |c| (-2..=2).map(move |d| [a, b, c, d])))
        })
        .filter(|[a, b, c, d]| a - c <= 1 && a <= d && b - c <= -1 && b - d <= 2)
        .collect();
    assert!(!models.is_empty());
    for encoded in 0..625 {
        let mut limits = Limits::default();
        let mut code = encoded;
        let weights: Vec<i128> = (0..4)
            .map(|_| {
                let w = code % 5 - 2;
                code /= 5;
                w
            })
            .collect();
        let mut query = Polynomial::default();
        for (value, weight) in values.iter().zip(&weights) {
            let term = value
                .multiply(&constant(*weight, &mut limits), &mut limits)
                .unwrap();
            query = query.add(&term, &mut limits).unwrap();
        }
        let Outcome::Proven(proof) = query.prove_within(&interval(-16, 16), &facts, &mut limits)
        else {
            panic!("bounded linear query {query}")
        };
        for model in &models {
            let expected = Integer::from_i128(model.iter().zip(&weights).map(|(x, w)| x * w).sum());
            assert!(
                &expected >= proof.enclosure().lower() && &expected <= proof.enclosure().upper(),
                "{model:?} {weights:?}: {proof}"
            );
        }
        proof.replay(&facts, &mut limits).unwrap();
    }
}

#[test]
fn predicate_entailment_checks_both_truths_and_boolean_structure_over_small_domains() {
    use elaboration::verification::{Comparison as C, Condition as Q};
    for low in -2..=2 {
        for high in low..=2 {
            for threshold in -3..=3 {
                for relation in [
                    C::Equal,
                    C::NotEqual,
                    C::Less,
                    C::LessEqual,
                    C::Greater,
                    C::GreaterEqual,
                ] {
                    let mut limits = Limits::default();
                    let mut facts = Facts::default();
                    let (id, _) = variable(&mut facts, 0, low, high, &mut limits);
                    let q = Q::Compare {
                        value: id,
                        relation,
                        constant: Integer::from_i128(threshold),
                    };
                    let eval = |x| match relation {
                        C::Equal => x == threshold,
                        C::NotEqual => x != threshold,
                        C::Less => x < threshold,
                        C::LessEqual => x <= threshold,
                        C::Greater => x > threshold,
                        C::GreaterEqual => x >= threshold,
                    };
                    for truth in [false, true] {
                        let expected = (low..=high).all(|x| eval(x) == truth);
                        for query in [
                            q.clone(),
                            Q::And(Box::new(Q::Bool(true)), Box::new(q.clone())),
                            Q::Or(Box::new(Q::Bool(false)), Box::new(q.clone())),
                            Q::Not(Box::new(Q::Not(Box::new(q.clone())))),
                        ] {
                            let before = facts.to_string();
                            let result =
                                facts.prove_condition(&query, truth, &mut Limits::default());
                            assert_eq!(
                                result.is_ok(),
                                expected,
                                "{low}..{high} {query} truth={truth}: {result:?}"
                            );
                            assert_eq!(facts.to_string(), before);
                            if let Ok(proof) = result {
                                assert_eq!(proof.condition(), &query);
                                assert_eq!(proof.truth(), truth);
                                proof.replay(&facts, &mut Limits::default()).unwrap();
                            }
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn predicate_proofs_replay_relations_and_reject_lost_stale_or_unrelated_evidence() {
    use elaboration::verification::{Comparison as C, Condition as Q};
    let mut limits = Limits::default();
    let mut facts = Facts::default();
    let (aid, a) = variable(&mut facts, 0, 0, 255, &mut limits);
    let (_, b) = variable(&mut facts, 1, 0, 255, &mut limits);
    let (_, c) = variable(&mut facts, 2, 0, 255, &mut limits);
    let unrefined = facts.fork(&mut limits).unwrap();
    for (left, right) in [(a.clone(), b.clone()), (b, c.clone())] {
        facts
            .assume(
                &Q::PolynomialCompare {
                    left,
                    relation: C::LessEqual,
                    right,
                },
                true,
                &mut limits,
            )
            .unwrap();
    }
    let goal = Q::PolynomialCompare {
        left: a,
        relation: C::LessEqual,
        right: c,
    };
    let before = facts.to_string();
    let mut tiny = Limits::default();
    tiny.arithmetic.steps = 1;
    assert!(matches!(
        facts.prove_condition(&goal, true, &mut tiny),
        Err(Unknown::Arithmetic(_))
    ));
    assert_eq!(before, facts.to_string());
    let proof = facts.prove_condition(&goal, true, &mut limits).unwrap();
    assert_eq!(before, facts.to_string());
    proof.replay(&facts, &mut limits).unwrap();
    proof
        .replay(&facts.fork(&mut limits).unwrap(), &mut limits)
        .unwrap();
    assert!(proof.replay(&unrefined, &mut limits).is_err());
    let joined = facts.join(&unrefined, &mut limits).unwrap();
    assert!(proof.replay(&joined, &mut limits).is_err());
    assert!(joined.prove_condition(&goal, true, &mut limits).is_err());
    let mut changed = facts.fork(&mut limits).unwrap();
    changed.assign(aid.place, interval(0, 255)).unwrap();
    assert!(proof.replay(&changed, &mut limits).is_err());
    assert_eq!(
        proof.replay(&Facts::default(), &mut limits),
        Err(Unknown::UnrelatedFacts)
    );
    facts.assume(&Q::Bool(false), true, &mut limits).unwrap();
    assert_eq!(
        facts.prove_condition(&Q::Bool(true), true, &mut limits),
        Err(Unknown::UnreachablePath)
    );
    assert_eq!(
        proof.replay(&facts, &mut limits),
        Err(Unknown::UnreachablePath)
    );
    println!("{proof}");
}

#[test]
fn predicate_entailment_uses_composed_bounds_and_short_circuit_refutation() {
    use elaboration::verification::{Comparison as C, Condition as Q};
    let mut limits = Limits::default();
    let mut facts = Facts::default();
    let (_, a) = variable(&mut facts, 0, 0, 100, &mut limits);
    let (_, b) = variable(&mut facts, 1, 0, 100, &mut limits);
    let (_, c) = variable(&mut facts, 2, 0, 100, &mut limits);
    let (_, d) = variable(&mut facts, 3, 0, 100, &mut limits);
    for (left, right) in [(c.clone(), a.clone()), (d.clone(), b.clone())] {
        facts
            .assume(
                &Q::PolynomialCompare {
                    left,
                    relation: C::LessEqual,
                    right,
                },
                true,
                &mut limits,
            )
            .unwrap();
    }
    let left = a.add(&b, &mut limits).unwrap();
    let right = c.add(&d, &mut limits).unwrap();
    for (relation, truth) in [(C::GreaterEqual, true), (C::Less, false)] {
        let query = Q::PolynomialCompare {
            left: left.clone(),
            relation,
            right: right.clone(),
        };
        facts
            .prove_condition(&query, truth, &mut limits)
            .unwrap()
            .replay(&facts, &mut limits)
            .unwrap();
    }
    // Neither disjunct is individually true on the entire input domain.
    let lt = Q::PolynomialCompare {
        left: a.clone(),
        relation: C::Less,
        right: b.clone(),
    };
    let ge = Q::PolynomialCompare {
        left: a,
        relation: C::GreaterEqual,
        right: b,
    };
    assert!(facts.prove_condition(&lt, true, &mut limits).is_err());
    assert!(facts.prove_condition(&ge, true, &mut limits).is_err());
    let total = Q::Or(Box::new(lt), Box::new(ge));
    facts
        .prove_condition(&total, true, &mut limits)
        .unwrap()
        .replay(&facts, &mut limits)
        .unwrap();
}

#[test]
fn retained_disjunctions_require_current_values_and_both_join_paths() {
    use elaboration::verification::{Comparison as C, Condition as Q};
    let mut limits = Limits::default();
    let mut root = Facts::default();
    let (a, _) = variable(&mut root, 0, 0, 1, &mut limits);
    let (b, _) = variable(&mut root, 1, 0, 1, &mut limits);
    let bit = |value| Q::Compare {
        value,
        relation: C::Equal,
        constant: Integer::from_u128(1),
    };
    let condition = Q::Or(Box::new(bit(a)), Box::new(bit(b)));
    let mut known = root.fork(&mut limits).unwrap();
    known.assume(&condition, true, &mut limits).unwrap();
    let proof = known
        .prove_condition(&condition, true, &mut limits)
        .unwrap();
    proof.replay(&known, &mut limits).unwrap();
    assert!(known.prove_condition(&bit(a), true, &mut limits).is_err());
    assert!(known.prove_condition(&bit(b), true, &mut limits).is_err());
    assert!(proof.replay(&root, &mut limits).is_err());
    let common = known
        .join(&known.fork(&mut limits).unwrap(), &mut limits)
        .unwrap();
    common
        .prove_condition(&condition, true, &mut limits)
        .unwrap();
    let partial = known.join(&root, &mut limits).unwrap();
    assert!(
        partial
            .prove_condition(&condition, true, &mut limits)
            .is_err()
    );
    let mut contradicted = known.fork(&mut limits).unwrap();
    contradicted.assume(&condition, false, &mut limits).unwrap();
    assert!(!contradicted.is_reachable());
    known.assign(a.place, interval(0, 0)).unwrap();
    assert!(proof.replay(&known, &mut limits).is_err());
    assert!(
        known
            .prove_condition(&condition, true, &mut limits)
            .is_err()
    );
    let replaced = Q::Or(
        Box::new(bit(known.value(a.place).unwrap())),
        Box::new(bit(b)),
    );
    assert!(known.prove_condition(&replaced, true, &mut limits).is_err());
    let mut exhausted = Limits::default();
    exhausted.arithmetic.steps = 0;
    let before = root.to_string();
    assert!(root.assume(&condition, true, &mut exhausted).is_err());
    assert_eq!(root.to_string(), before);
    root.assume(&condition, true, &mut limits).unwrap();
    assert!(root.to_string().contains("(conditions (established"));
    root.prove_condition(&condition, true, &mut limits).unwrap();
}

#[test]
fn retained_disjunctions_follow_fresh_merge_versions_and_operand_permutations() {
    use elaboration::verification::{Comparison as C, Condition as Q};
    let mut limits = Limits::default();
    let mut root = Facts::default();
    let (a, _) = variable(&mut root, 0, 0, 1, &mut limits);
    let (b, _) = variable(&mut root, 1, 0, 1, &mut limits);
    let bit = |value| Q::Compare {
        value,
        relation: C::Equal,
        constant: Integer::from_u128(1),
    };
    let mut left = root.fork(&mut limits).unwrap();
    let mut right = root.fork(&mut limits).unwrap();
    let la = left.assign(a.place, interval(0, 1)).unwrap();
    let lb = left.assign(b.place, interval(0, 1)).unwrap();
    let ra = right.assign(a.place, interval(0, 1)).unwrap();
    let rb = right.assign(b.place, interval(0, 1)).unwrap();
    left.assume(
        &Q::Or(Box::new(bit(la)), Box::new(bit(lb))),
        true,
        &mut limits,
    )
    .unwrap();
    right
        .assume(
            &Q::Or(Box::new(bit(rb)), Box::new(bit(ra))),
            true,
            &mut limits,
        )
        .unwrap();
    let merged = left.join(&right, &mut limits).unwrap();
    let condition = Q::Or(
        Box::new(bit(merged.value(a.place).unwrap())),
        Box::new(bit(merged.value(b.place).unwrap())),
    );
    let proof = merged
        .prove_condition(&condition, true, &mut limits)
        .unwrap();
    proof.replay(&merged, &mut limits).unwrap();
    assert!(proof.replay(&left, &mut limits).is_err());
    assert!(
        merged
            .prove_condition(
                &Q::And(
                    Box::new(bit(merged.value(a.place).unwrap())),
                    Box::new(bit(merged.value(b.place).unwrap()))
                ),
                true,
                &mut limits
            )
            .is_err()
    );
}

#[test]
fn retained_boolean_matching_is_associative_bounded_and_operator_sensitive() {
    use elaboration::verification::{Comparison as C, Condition as Q};
    let mut limits = Limits::default();
    let mut facts = Facts::default();
    let (a, _) = variable(&mut facts, 0, 0, 1, &mut limits);
    let (b, _) = variable(&mut facts, 1, 0, 1, &mut limits);
    let (c, _) = variable(&mut facts, 2, 0, 1, &mut limits);
    let bit = |value| Q::Compare {
        value,
        relation: C::Equal,
        constant: Integer::from_u128(1),
    };
    let either = |a, b| Q::Or(Box::new(a), Box::new(b));
    let premise = either(bit(a), either(bit(b), bit(c)));
    facts.assume(&premise, true, &mut limits).unwrap();
    let regrouped = either(either(bit(c), bit(a)), bit(b));
    let proof = facts
        .prove_condition(&regrouped, true, &mut limits)
        .unwrap();
    proof.replay(&facts, &mut limits).unwrap();
    let stronger = either(bit(a), Q::And(Box::new(bit(b)), Box::new(bit(c))));
    assert!(facts.prove_condition(&stronger, true, &mut limits).is_err());
    assert!(
        facts
            .prove_condition(&either(bit(a), bit(b)), true, &mut limits)
            .is_err()
    );
    let mut tiny = Limits::default();
    tiny.arithmetic.steps = 2;
    assert!(facts.prove_condition(&regrouped, true, &mut tiny).is_err());
    facts
        .prove_condition(&regrouped, true, &mut limits)
        .unwrap();
    let both = |a, b| Q::And(Box::new(a), Box::new(b));
    facts
        .assume(&both(bit(a), both(bit(b), bit(c))), false, &mut limits)
        .unwrap();
    facts
        .prove_condition(&both(both(bit(c), bit(a)), bit(b)), false, &mut limits)
        .unwrap();
}

#[test]
fn unbounded_integer_comparisons_agree_with_integer_order() {
    use elaboration::verification::{Comparison as C, Condition};
    fn accepts(c: C, x: i128, y: i128) -> bool {
        match c {
            C::Equal => x == y,
            C::NotEqual => x != y,
            C::Less => x < y,
            C::LessEqual => x <= y,
            C::Greater => x > y,
            C::GreaterEqual => x >= y,
        }
    }
    let relations = [
        C::Equal,
        C::NotEqual,
        C::Less,
        C::LessEqual,
        C::Greater,
        C::GreaterEqual,
    ];
    for a in -2..=2 {
        for b in -2..=2 {
            for premise in relations {
                for query in relations {
                    let mut limits = Limits::default();
                    let mut facts = Facts::default();
                    let id = facts
                        .invalidate(PlaceId {
                            owner: 77,
                            local: 0,
                        })
                        .unwrap();
                    let condition = |relation, c| Condition::Compare {
                        value: id,
                        relation,
                        constant: Integer::from_i128(c),
                    };
                    facts
                        .assume(&condition(premise, a), true, &mut limits)
                        .unwrap();
                    // All transition points lie inside this witness set; its endpoints
                    // also represent the two unbounded tails for these comparisons.
                    let expected =
                        (-4..=4).all(|x| !accepts(premise, x, a) || accepts(query, x, b));
                    assert_eq!(
                        facts
                            .prove_condition(&condition(query, b), true, &mut limits)
                            .is_ok(),
                        expected,
                        "premise {premise:?} {a}, query {query:?} {b}"
                    );
                }
            }
        }
    }
}

#[test]
fn unbounded_integer_proof_replay_rejects_assignment_and_unrelated_lineage() {
    use elaboration::verification::{Comparison as C, Condition};
    let mut limits = Limits::default();
    let mut facts = Facts::default();
    let place = PlaceId {
        owner: 78,
        local: 0,
    };
    let id = facts.invalidate(place).unwrap();
    let condition = |c| Condition::Compare {
        value: id,
        relation: C::Greater,
        constant: Integer::from_i128(c),
    };
    facts.assume(&condition(10), true, &mut limits).unwrap();
    let proof = facts
        .prove_condition(&condition(0), true, &mut limits)
        .unwrap();
    proof.replay(&facts, &mut limits).unwrap();
    proof
        .replay(&facts.fork(&mut limits).unwrap(), &mut limits)
        .unwrap();
    assert!(proof.replay(&Facts::default(), &mut limits).is_err());
    facts.invalidate(place).unwrap();
    assert!(proof.replay(&facts, &mut limits).is_err());
    assert!(
        facts
            .prove_condition(&condition(0), true, &mut limits)
            .is_err()
    );
}

#[test]
fn unbounded_integer_join_does_not_assume_both_alternatives() {
    use elaboration::verification::{Comparison as C, Condition};
    let mut limits = Limits::default();
    let mut root = Facts::default();
    let id = root
        .invalidate(PlaceId {
            owner: 79,
            local: 0,
        })
        .unwrap();
    let condition = |c| Condition::Compare {
        value: id,
        relation: C::Greater,
        constant: Integer::from_i128(c),
    };
    let mut left = root.fork(&mut limits).unwrap();
    let mut right = root.fork(&mut limits).unwrap();
    left.assume(&condition(0), true, &mut limits).unwrap();
    right.assume(&condition(10), true, &mut limits).unwrap();
    let merged = left.join(&right, &mut limits).unwrap();
    merged
        .prove_condition(&condition(0), true, &mut limits)
        .unwrap();
    assert!(
        merged
            .prove_condition(&condition(10), true, &mut limits)
            .is_err()
    );
    let reversed = right.join(&left, &mut limits).unwrap();
    reversed
        .prove_condition(&condition(0), true, &mut limits)
        .unwrap();
    assert!(
        reversed
            .prove_condition(&condition(10), true, &mut limits)
            .is_err()
    );
    let merged = left.join(&root, &mut limits).unwrap();
    assert!(
        merged
            .prove_condition(&condition(0), true, &mut limits)
            .is_err()
    );
}

#[test]
fn unbounded_integer_two_sided_premises_enable_polynomial_enclosures() {
    use elaboration::verification::{Comparison as C, Condition};
    let mut limits = Limits::default();
    let mut facts = Facts::default();
    let id = facts
        .invalidate(PlaceId {
            owner: 80,
            local: 0,
        })
        .unwrap();
    let condition = |relation, c| Condition::Compare {
        value: id,
        relation,
        constant: Integer::from_i128(c),
    };
    facts
        .assume(&condition(C::Greater, 10), true, &mut limits)
        .unwrap();
    assert!(facts.bound(id, &mut limits).is_err());
    facts
        .assume(&condition(C::Less, 20), true, &mut limits)
        .unwrap();
    assert_eq!(facts.bound(id, &mut limits).unwrap(), &interval(11, 19));
    let x = Polynomial::variable(id, &mut limits).unwrap();
    let square = x.multiply(&x, &mut limits).unwrap();
    assert!(matches!(
        square.prove_within(&interval(121, 361), &facts, &mut limits),
        Outcome::Proven(_)
    ));
}

#[test]
fn unbounded_integer_excluded_singleton_is_unreachable() {
    use elaboration::verification::{Comparison as C, Condition};
    let mut limits = Limits::default();
    let mut facts = Facts::default();
    let id = facts
        .invalidate(PlaceId {
            owner: 81,
            local: 0,
        })
        .unwrap();
    for (relation, c) in [(C::NotEqual, 1), (C::Greater, 0), (C::Less, 2)] {
        facts
            .assume(
                &Condition::Compare {
                    value: id,
                    relation,
                    constant: Integer::from_i128(c),
                },
                true,
                &mut limits,
            )
            .unwrap();
    }
    assert!(!facts.is_reachable());
}

#[test]
fn unbounded_symbolic_equality_keeps_versions_without_inventing_bounds() {
    use elaboration::verification::{Comparison as C, Condition};
    let mut limits = Limits::default();
    let mut facts = Facts::default();
    let x = facts
        .invalidate(PlaceId {
            owner: 82,
            local: 0,
        })
        .unwrap();
    let y = facts
        .invalidate(PlaceId {
            owner: 82,
            local: 1,
        })
        .unwrap();
    let equal = Condition::PolynomialCompare {
        left: Polynomial::variable(x, &mut limits).unwrap(),
        relation: C::Equal,
        right: Polynomial::variable(y, &mut limits).unwrap(),
    };
    assert!(facts.prove_condition(&equal, true, &mut limits).is_err());
    facts.assume(&equal, true, &mut limits).unwrap();
    let reversed = Condition::PolynomialCompare {
        left: Polynomial::variable(y, &mut limits).unwrap(),
        relation: C::Equal,
        right: Polynomial::variable(x, &mut limits).unwrap(),
    };
    let proof = facts.prove_condition(&reversed, true, &mut limits).unwrap();
    proof.replay(&facts, &mut limits).unwrap();
    assert!(matches!(
        facts.bound(x, &mut limits),
        Err(Unknown::MissingBound(_))
    ));
    facts.invalidate(y.place).unwrap();
    assert!(proof.replay(&facts, &mut limits).is_err());
    assert!(facts.assume(&equal, true, &mut limits).is_err());
}

#[test]
fn unbounded_polynomial_cancellation_proves_order_but_keeps_version_dependencies() {
    use elaboration::verification::{Comparison as C, Condition};
    let mut limits = Limits::default();
    let mut facts = Facts::default();
    let id = facts
        .invalidate(PlaceId {
            owner: 83,
            local: 0,
        })
        .unwrap();
    let x = Polynomial::variable(id, &mut limits).unwrap();
    let square = x.multiply(&x, &mut limits).unwrap();
    let cubic = square.multiply(&x, &mut limits).unwrap();
    let one = constant(1, &mut limits);
    let greater = Condition::PolynomialCompare {
        left: cubic.add(&one, &mut limits).unwrap(),
        relation: C::Greater,
        right: cubic,
    };
    let proof = facts.prove_condition(&greater, true, &mut limits).unwrap();
    proof.replay(&facts, &mut limits).unwrap();
    assert!(matches!(
        facts.bound(id, &mut limits),
        Err(Unknown::MissingBound(_))
    ));
    facts.invalidate(id.place).unwrap();
    assert!(proof.replay(&facts, &mut limits).is_err());
    assert!(facts.prove_condition(&greater, true, &mut limits).is_err());
}

#[test]
fn unbounded_polynomial_cancellation_detects_impossible_premises() {
    use elaboration::verification::{Comparison as C, Condition};
    for relation in [C::Less, C::Greater, C::NotEqual] {
        let mut limits = Limits::default();
        let mut facts = Facts::default();
        let id = facts
            .invalidate(PlaceId {
                owner: 84,
                local: 0,
            })
            .unwrap();
        let x = Polynomial::variable(id, &mut limits).unwrap();
        let condition = Condition::PolynomialCompare {
            left: x.clone(),
            relation,
            right: x,
        };
        facts.assume(&condition, true, &mut limits).unwrap();
        assert!(!facts.is_reachable());
    }
}

#[test]
fn captured_integer_facts_check_domains_budgets_and_symbolic_inputs() {
    use elaboration::verification::flow;
    use nir::program::{
        Expr, ExprKind as K, Function, FunctionPhase, IntegerOp, Literal, SourceRange,
    };
    use nir::{DeclarationId, Primitive, Sort, Term};
    let function = |primitive, value| {
        let ty = Term::Primitive(primitive);
        let expression = |kind| Expr {
            ty: ty.clone(),
            source: SourceRange { start: 0, end: 1 },
            kind,
        };
        Function {
            declaration: DeclarationId {
                package: 1,
                local: 1,
            },
            name: "increment".into(),
            phase: FunctionPhase::Runtime,
            inline: false,
            pure: false,
            is_unsafe: false,
            signature: Term::Function {
                parameters: vec![],
                result: Box::new(ty.clone()),
            },
            parameters: vec![],
            type_parameters: vec![],
            defaults: vec![],
            preconditions: vec![],
            postconditions: vec![],
            result_binding: None,
            entry_snapshots: vec![],
            capabilities: vec![],
            requirements: vec![],
            body: expression(K::IntegerBinary {
                op: IntegerOp::Add,
                left: Box::new(expression(K::ComptimeValue(value))),
                right: Box::new(expression(K::Literal(match primitive {
                    Primitive::I8 => Literal::SignedInteger(1),
                    Primitive::Integer => Literal::Integer("1".into()),
                    _ => Literal::UnsignedInteger(1),
                }))),
            }),
        }
    };
    for (primitive, value, accepted) in [
        (Primitive::U8, 254, true),
        (Primitive::U8, 255, false),
        (Primitive::U8, 256, false),
        (Primitive::U8, -1, false),
        (Primitive::I8, -128, true),
        (Primitive::I8, -129, false),
        (Primitive::I8, 127, false),
    ] {
        let input = function(primitive, Term::Integer(Integer::from_i128(value)));
        assert_eq!(
            flow::analyze(&input, &mut Limits::default()).is_ok(),
            accepted,
            "{primitive:?}/{value}"
        );
    }
    let symbolic = function(
        Primitive::U8,
        Term::Rigid {
            declaration: DeclarationId {
                package: 1,
                local: 2,
            },
            sort: Sort::ComptimeValue,
        },
    );
    assert!(flow::analyze(&symbolic, &mut Limits::default()).is_err());
    let huge = Integer::parse_literal(&"9".repeat(200), &mut Default::default()).unwrap();
    let input = function(Primitive::Integer, Term::Integer(huge));
    let mut limited = Limits::default();
    limited.arithmetic.max_limbs = 1;
    assert!(flow::analyze(&input, &mut limited).is_err());
    assert!(flow::analyze(&input, &mut Limits::default()).is_ok());
    let mut exhausted = Limits::default();
    exhausted.arithmetic.steps = 2;
    assert!(flow::analyze(&input, &mut exhausted).is_err());
    assert!(flow::analyze(&input, &mut Limits::default()).is_ok());

    // A checked associated value contributes its domain, never its initializer
    // or a fabricated constant. Reading u8 is valid; incrementing an unknown
    // u8 still needs an overflow proof.
    let mut associated = function(Primitive::U8, Term::Integer(Integer::from_u128(0)));
    let K::IntegerBinary { left, .. } = &mut associated.body.kind else {
        unreachable!()
    };
    left.kind = K::AssociatedValue {
        interface: nir::program::InterfaceInstance {
            declaration: DeclarationId {
                package: 1,
                local: 10,
            },
            arguments: vec![],
        },
        member: DeclarationId {
            package: 1,
            local: 11,
        },
        subject: Term::Primitive(Primitive::Bool),
    };
    let read = Function {
        body: *left.clone(),
        ..associated.clone()
    };
    assert!(flow::analyze(&read, &mut Limits::default()).is_ok());
    assert_eq!(
        flow::analyze(&associated, &mut Limits::default())
            .err()
            .unwrap()
            .reason,
        flow::Reason::MissingMachineProof,
    );
}

#[test]
fn retained_implications_activate_after_branch_selection_and_replay_safely() {
    use elaboration::verification::{Comparison as C, Condition as Q};
    let mut limits = Limits::default();
    let mut facts = Facts::default();
    let (tag, _) = variable(&mut facts, 0, 0, 1, &mut limits);
    let (middle, _) = variable(&mut facts, 1, 0, 1, &mut limits);
    let (payload, _) = variable(&mut facts, 2, 0, 255, &mut limits);
    let selected = Q::Compare {
        value: tag,
        relation: C::Equal,
        constant: Integer::from_u128(1),
    };
    let ready = Q::Compare {
        value: middle,
        relation: C::Equal,
        constant: Integer::from_u128(1),
    };
    let bounded = Q::Compare {
        value: payload,
        relation: C::LessEqual,
        constant: Integer::from_u128(19),
    };
    let implication = |a: Q, b: Q| Q::Or(Box::new(Q::Not(Box::new(a))), Box::new(b));
    // Deliberately reverse the dependency order to require repeated propagation.
    facts
        .assume(
            &implication(ready.clone(), bounded.clone()),
            true,
            &mut limits,
        )
        .unwrap();
    facts
        .assume(&implication(selected.clone(), ready), true, &mut limits)
        .unwrap();
    let before = facts.fork(&mut limits).unwrap();
    assert!(before.prove_condition(&bounded, true, &mut limits).is_err());
    facts.assume(&selected, true, &mut limits).unwrap();
    let proof = facts.prove_condition(&bounded, true, &mut limits).unwrap();
    proof.replay(&facts, &mut limits).unwrap();
    assert!(proof.replay(&before, &mut limits).is_err());
    let mut changed = facts.fork(&mut limits).unwrap();
    changed.assign(payload.place, interval(0, 255)).unwrap();
    assert!(proof.replay(&changed, &mut limits).is_err());
    let new_query = Q::Compare {
        value: changed.value(payload.place).unwrap(),
        relation: C::LessEqual,
        constant: Integer::from_u128(19),
    };
    assert!(
        changed
            .prove_condition(&new_query, true, &mut limits)
            .is_err()
    );
    let mut retagged = facts.fork(&mut limits).unwrap();
    retagged.assign(tag.place, interval(0, 1)).unwrap();
    assert!(proof.replay(&retagged, &mut limits).is_err());
    // Refinement already established a bound on the unchanged payload. A
    // later tag write does not undo that historical fact about its value.
    retagged
        .prove_condition(&bounded, true, &mut limits)
        .unwrap();
    let mut unselected = before.fork(&mut limits).unwrap();
    let new_tag = unselected.assign(tag.place, interval(0, 1)).unwrap();
    let new_selected = Q::Compare {
        value: new_tag,
        relation: C::Equal,
        constant: Integer::from_u128(1),
    };
    unselected.assume(&new_selected, true, &mut limits).unwrap();
    assert!(
        unselected
            .prove_condition(&bounded, true, &mut limits)
            .is_err()
    );
    let saved = facts.to_string();
    let mut exhausted = Limits::default();
    exhausted.arithmetic.steps = 0;
    assert!(
        facts
            .prove_condition(&bounded, true, &mut exhausted)
            .is_err()
    );
    assert_eq!(saved, facts.to_string());
}

#[test]
fn conditional_proofs_agree_with_exhaustive_boolean_models() {
    use elaboration::verification::{Comparison as C, Condition as Q};
    for premises in 0u8..8 {
        let mut limits = Limits::default();
        let mut facts = Facts::default();
        let ids = (0..3)
            .map(|local| variable(&mut facts, local, 0, 1, &mut limits).0)
            .collect::<Vec<_>>();
        let bit = |index: usize| Q::Compare {
            value: ids[index],
            relation: C::Equal,
            constant: Integer::from_u128(1),
        };
        let clauses = [
            Q::Or(Box::new(Q::Not(Box::new(bit(0)))), Box::new(bit(1))),
            Q::Or(Box::new(Q::Not(Box::new(bit(1)))), Box::new(bit(2))),
            Q::Or(Box::new(bit(0)), Box::new(bit(2))),
        ];
        for (index, clause) in clauses.iter().enumerate() {
            if premises & (1 << index) != 0 {
                facts.assume(clause, true, &mut limits).unwrap();
            }
        }
        for selected in [false, true] {
            let mut branch = facts.fork(&mut limits).unwrap();
            branch.assume(&bit(0), selected, &mut limits).unwrap();
            let models = (0u8..8)
                .filter(|model| {
                    let a = model & 1 != 0;
                    let b = model & 2 != 0;
                    let c = model & 4 != 0;
                    a == selected
                        && (premises & 1 == 0 || !a || b)
                        && (premises & 2 == 0 || !b || c)
                        && (premises & 4 == 0 || a || c)
                })
                .collect::<Vec<_>>();
            for index in 0..3 {
                for truth in [false, true] {
                    let proof = branch.prove_condition(&bit(index), truth, &mut Limits::default());
                    if let Ok(proof) = proof {
                        assert!(
                            models
                                .iter()
                                .all(|model| (model & (1 << index) != 0) == truth),
                            "premises={premises} branch={selected} query={index}/{truth}"
                        );
                        proof.replay(&branch, &mut Limits::default()).unwrap();
                    }
                }
            }
        }
    }
}

#[test]
fn checked_nonzero_division_preserves_both_signs_and_machine_obligations() {
    use elaboration::verification::flow;
    use nir::program::{
        Expr, ExprKind as K, Function, FunctionPhase, IntegerOp, Literal, LocalId, Place,
        SourceRange,
    };
    use nir::{DeclarationId, Parameter, Primitive, Sort, Term};
    let ty = Term::Primitive(Primitive::I8);
    let divisor = LocalId { owner: 1, index: 0 };
    let expression = |kind, ty| Expr {
        kind,
        ty,
        source: SourceRange { start: 0, end: 1 },
    };
    let literal = |value| expression(K::Literal(Literal::SignedInteger(value)), ty.clone());
    let function = |op, numerator, checked, overwrite| {
        let mut statements = vec![];
        if checked {
            let condition = expression(
                K::IntegerBinary {
                    op: IntegerOp::NotEqual,
                    left: Box::new(expression(K::Local(divisor), ty.clone())),
                    right: Box::new(literal(0)),
                },
                Term::Primitive(Primitive::Bool),
            );
            statements.push(expression(
                K::CheckPredicate(Box::new(condition)),
                Term::Primitive(Primitive::Void),
            ));
        }
        if overwrite {
            statements.push(expression(
                K::Assign {
                    place: Place {
                        root: divisor,
                        fields: vec![],
                    },
                    value: Box::new(literal(0)),
                },
                Term::Primitive(Primitive::Void),
            ));
        }
        Function {
            declaration: DeclarationId {
                package: 1,
                local: 1,
            },
            name: "checked_division".into(),
            phase: FunctionPhase::Runtime,
            inline: false,
            pure: false,
            is_unsafe: false,
            signature: Term::Function {
                parameters: vec![Parameter {
                    sort: Sort::Runtime,
                    ty: ty.clone(),
                }],
                result: Box::new(ty.clone()),
            },
            parameters: vec![divisor],
            type_parameters: vec![],
            defaults: vec![],
            preconditions: vec![],
            postconditions: vec![],
            result_binding: None,
            entry_snapshots: vec![],
            capabilities: vec![],
            requirements: vec![],
            body: expression(
                K::Block {
                    statements,
                    result: Some(Box::new(expression(
                        K::IntegerBinary {
                            op,
                            left: Box::new(literal(numerator)),
                            right: Box::new(expression(K::Local(divisor), ty.clone())),
                        },
                        ty.clone(),
                    ))),
                },
                ty.clone(),
            ),
        }
    };
    for op in [IntegerOp::DivideTrunc, IntegerOp::RemainderTrunc] {
        for numerator in [-127, -7, 0, 7, 127] {
            let input = function(op, numerator, true, false);
            let report = flow::analyze(&input, &mut Limits::default())
                .unwrap_or_else(|error| panic!("{op:?}/{numerator}: {error}"));
            let range = report
                .points
                .iter()
                .rev()
                .find(|point| point.kind == flow::PointKind::Arithmetic(op))
                .and_then(|point| point.range.as_ref())
                .unwrap();
            // Exhaust all machine divisors, independently of the abstract transfer.
            for denominator in -128i128..=127 {
                if denominator == 0 {
                    continue;
                }
                let value = Integer::from_i128(if op == IntegerOp::DivideTrunc {
                    numerator / denominator
                } else {
                    numerator % denominator
                });
                assert!(range.lower() <= &value && &value <= range.upper());
            }
            let mut exhausted = Limits::default();
            exhausted.arithmetic.steps = 0;
            assert!(flow::analyze(&input, &mut exhausted).is_err());
            assert!(flow::analyze(&input, &mut Limits::default()).is_ok());
        }
        for (checked, overwrite) in [(false, false), (true, true)] {
            assert_eq!(
                flow::analyze(&function(op, 7, checked, overwrite), &mut Limits::default())
                    .err()
                    .unwrap()
                    .reason,
                flow::Reason::MissingDivisionProof
            );
        }
        // MIN % -1 also needs the quotient to fit the machine kernel.
        assert_eq!(
            flow::analyze(&function(op, -128, true, false), &mut Limits::default())
                .err()
                .unwrap()
                .reason,
            flow::Reason::MissingMachineProof
        );
    }
}

#[test]
fn monotone_cubic_ranges_preserve_correlations_and_replay_dependencies() {
    let mut limits = Limits::default();
    let mut facts = Facts::default();
    let (id, x) = variable(&mut facts, 0, 1, 10, &mut limits);
    let cube = x
        .multiply(&x, &mut limits)
        .unwrap()
        .multiply(&x, &mut limits)
        .unwrap();
    let expression = cube.subtract(&x, &mut limits).unwrap();
    let Outcome::Proven(proof) = expression.prove_within(&interval(0, 990), &facts, &mut limits)
    else {
        panic!("monotone cubic")
    };
    assert_eq!(proof.enclosure(), &interval(0, 990));
    proof.replay(&facts, &mut Limits::default()).unwrap();
    let negative = constant(-1, &mut limits)
        .multiply(&expression, &mut limits)
        .unwrap();
    assert!(matches!(
        negative.prove_within(&interval(-990, 0), &facts, &mut limits),
        Outcome::Proven(_)
    ));
    facts.assign(id.place, interval(0, 10)).unwrap();
    assert!(proof.replay(&facts, &mut Limits::default()).is_err());
}

#[test]
fn monotonic_range_refinement_does_not_ignore_interior_extrema() {
    let mut limits = Limits::default();
    let mut facts = Facts::default();
    let (_, x) = variable(&mut facts, 0, 0, 2, &mut limits);
    let square = x.multiply(&x, &mut limits).unwrap();
    let linear = constant(2, &mut limits).multiply(&x, &mut limits).unwrap();
    let expression = square.subtract(&linear, &mut limits).unwrap();
    assert!(matches!(
        expression.prove_within(&interval(0, 0), &facts, &mut limits),
        Outcome::Unknown(_)
    ));
    assert!(matches!(
        expression.prove_within(&interval(-4, 4), &facts, &mut limits),
        Outcome::Proven(_)
    ));
}

#[test]
fn monotone_multivariate_cubics_use_each_coordinate_and_recover_after_budget_failure() {
    let mut limits = Limits::default();
    let mut facts = Facts::default();
    let (_, x) = variable(&mut facts, 0, 1, 4, &mut limits);
    let (_, y) = variable(&mut facts, 1, 1, 3, &mut limits);
    let mut expression = Polynomial::default();
    for variable in [x, y] {
        let cube = variable
            .multiply(&variable, &mut limits)
            .unwrap()
            .multiply(&variable, &mut limits)
            .unwrap();
        expression = expression
            .add(&cube.subtract(&variable, &mut limits).unwrap(), &mut limits)
            .unwrap();
    }
    let target = interval(0, 84);
    let mut exhausted = Limits::default();
    exhausted.arithmetic.steps = 1;
    assert!(matches!(
        expression.prove_within(&target, &facts, &mut exhausted),
        Outcome::Unknown(Unknown::Arithmetic(ArithmeticError::BudgetExhausted))
    ));
    let Outcome::Proven(proof) = expression.prove_within(&target, &facts, &mut limits) else {
        panic!("multivariate cubic")
    };
    proof.replay(&facts, &mut Limits::default()).unwrap();
    for x in 1..=4 {
        for y in 1..=3 {
            let exact = Integer::from_i128(x * x * x - x + y * y * y - y);
            assert!(&exact >= proof.enclosure().lower() && &exact <= proof.enclosure().upper());
        }
    }
}

#[test]
fn monotone_polynomial_predicates_preserve_truth_boundaries_and_versions() {
    use elaboration::verification::{Comparison, Condition};
    let mut limits = Limits::default();
    let mut facts = Facts::default();
    let (id, x) = variable(&mut facts, 0, 1, 10, &mut limits);
    let cube = x
        .multiply(&x, &mut limits)
        .unwrap()
        .multiply(&x, &mut limits)
        .unwrap();
    let condition = |relation| Condition::PolynomialCompare {
        left: cube.clone(),
        relation,
        right: x.clone(),
    };
    for (relation, truth) in [(Comparison::GreaterEqual, true), (Comparison::Less, false)] {
        let proof = facts
            .prove_condition(&condition(relation), truth, &mut limits)
            .unwrap();
        proof.replay(&facts, &mut limits).unwrap();
    }
    for relation in [Comparison::Greater, Comparison::Equal, Comparison::NotEqual] {
        for truth in [false, true] {
            assert!(
                facts
                    .prove_condition(&condition(relation), truth, &mut limits)
                    .is_err()
            );
        }
    }
    let proof = facts
        .prove_condition(&condition(Comparison::GreaterEqual), true, &mut limits)
        .unwrap();
    facts.assign(id.place, interval(-2, 2)).unwrap();
    assert!(proof.replay(&facts, &mut Limits::default()).is_err());
}

#[test]
fn integer_grid_monotonicity_proves_ranges_missed_by_real_derivatives() {
    let mut limits = Limits::default();
    let mut facts = Facts::default();
    let (_, x) = variable(&mut facts, 0, 0, 10, &mut limits);
    let square = x.multiply(&x, &mut limits).unwrap();
    let cube = square.multiply(&x, &mut limits).unwrap();
    for (power, upper) in [(square, 90), (cube, 990)] {
        let expression = power.subtract(&x, &mut limits).unwrap();
        let Outcome::Proven(proof) =
            expression.prove_within(&interval(0, upper), &facts, &mut limits)
        else {
            panic!("integer monotonicity")
        };
        proof.replay(&facts, &mut limits).unwrap();
        assert_eq!(proof.enclosure(), &interval(0, upper));
        let negative = constant(-1, &mut limits)
            .multiply(&expression, &mut limits)
            .unwrap();
        assert!(matches!(
            negative.prove_within(&interval(-upper, 0), &facts, &mut limits),
            Outcome::Proven(_)
        ));
    }
}

#[test]
fn discrete_monotonicity_checks_only_steps_inside_the_original_interval() {
    let mut limits = Limits::default();
    let mut facts = Facts::default();
    let (id, x) = variable(&mut facts, 0, 0, 2, &mut limits);
    let square = x.multiply(&x, &mut limits).unwrap();
    let linear = constant(3, &mut limits).multiply(&x, &mut limits).unwrap();
    let expression = linear.subtract(&square, &mut limits).unwrap();
    let Outcome::Proven(proof) = expression.prove_within(&interval(0, 2), &facts, &mut limits)
    else {
        panic!("finite step domain")
    };
    assert_eq!(facts.bound(id, &mut limits).unwrap(), &interval(0, 2));
    proof.replay(&facts, &mut limits).unwrap();
    assert!(matches!(
        expression.prove_within(&interval(0, 1), &facts, &mut limits),
        Outcome::Unknown(_)
    ));
}

#[test]
fn small_integer_polynomial_range_proofs_agree_with_exhaustive_values() {
    for (low, high) in [(-2, 0), (-1, 1), (0, 2), (1, 3)] {
        for a in -2..=2 {
            for b in -2..=2 {
                for c in -1..=1 {
                    let mut limits = Limits::default();
                    let mut facts = Facts::default();
                    let (_, x) = variable(&mut facts, 0, low, high, &mut limits);
                    let square = x.multiply(&x, &mut limits).unwrap();
                    let cube = square.multiply(&x, &mut limits).unwrap();
                    let expression = constant(a, &mut limits)
                        .multiply(&cube, &mut limits)
                        .unwrap()
                        .add(
                            &constant(b, &mut limits)
                                .multiply(&square, &mut limits)
                                .unwrap(),
                            &mut limits,
                        )
                        .unwrap()
                        .add(
                            &constant(c, &mut limits).multiply(&x, &mut limits).unwrap(),
                            &mut limits,
                        )
                        .unwrap();
                    let samples = (low..=high)
                        .map(|x| a * x * x * x + b * x * x + c * x)
                        .collect::<Vec<_>>();
                    let min = *samples.iter().min().unwrap();
                    let max = *samples.iter().max().unwrap();
                    let Outcome::Proven(proof) =
                        expression.prove_within(&interval(min, max), &facts, &mut limits)
                    else {
                        panic!("finite domain {a} {b} {c} over [{low},{high}]");
                    };
                    {
                        proof.replay(&facts, &mut Limits::default()).unwrap();
                        for value in &samples {
                            let value = Integer::from_i128(*value);
                            assert!(
                                &value >= proof.enclosure().lower()
                                    && &value <= proof.enclosure().upper()
                            );
                        }
                    }
                    for wrong in [interval(min - 1, max - 1), interval(min + 1, max + 1)] {
                        assert!(
                            matches!(
                                expression.prove_within(&wrong, &facts, &mut Limits::default()),
                                Outcome::Unknown(_)
                            ),
                            "unsound {a} {b} {c} over [{low},{high}]"
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn finite_boxes_prove_interior_extrema_and_retain_input_versions() {
    let mut limits = Limits::default();
    let mut facts = Facts::default();
    let (id, x) = variable(&mut facts, 0, 0, 4, &mut limits);
    let (_, y) = variable(&mut facts, 1, -2, 2, &mut limits);
    let complement = constant(4, &mut limits).subtract(&x, &mut limits).unwrap();
    let expression = x
        .multiply(&complement, &mut limits)
        .unwrap()
        .multiply(&y, &mut limits)
        .unwrap();
    let Outcome::Proven(proof) = expression.prove_within(&interval(-8, 8), &facts, &mut limits)
    else {
        panic!("interior extremum of multivariate cubic");
    };
    assert_eq!(proof.enclosure(), &interval(-8, 8));
    proof.replay(&facts, &mut Limits::default()).unwrap();
    assert!(matches!(
        expression.prove_within(&interval(-7, 7), &facts, &mut Limits::default()),
        Outcome::Unknown(_)
    ));
    let mut exhausted = Limits::default();
    exhausted.arithmetic.steps = 1;
    assert!(matches!(
        expression.prove_within(&interval(-8, 8), &facts, &mut exhausted),
        Outcome::Unknown(_)
    ));
    proof.replay(&facts, &mut Limits::default()).unwrap();
    facts.assign(id.place, interval(0, 4)).unwrap();
    assert!(proof.replay(&facts, &mut Limits::default()).is_err());
}

#[test]
fn finite_box_volume_limit_never_accepts_a_partial_enumeration() {
    let mut limits = Limits::default();
    let mut facts = Facts::default();
    let (_, x) = variable(&mut facts, 0, 0, 256, &mut limits);
    let complement = constant(256, &mut limits)
        .subtract(&x, &mut limits)
        .unwrap();
    let (_, y) = variable(&mut facts, 1, -1, 1, &mut limits);
    let expression = x
        .multiply(&x, &mut limits)
        .unwrap()
        .multiply(&complement, &mut limits)
        .unwrap()
        .add(
            &y.multiply(&y, &mut limits)
                .unwrap()
                .multiply(&x, &mut limits)
                .unwrap(),
            &mut limits,
        )
        .unwrap();
    let maximum = (0i128..=256).map(|x| x * x * (256 - x)).max().unwrap();
    // The extremum is inside a box larger than the enumeration cap. A
    // prefix or just the endpoints would incorrectly certify a smaller range.
    assert!(matches!(
        expression.prove_within(&interval(0, maximum + 256), &facts, &mut limits),
        Outcome::Unknown(_)
    ));
    assert!(matches!(
        expression.prove_within(&interval(0, 0), &facts, &mut Limits::default()),
        Outcome::Unknown(_)
    ));
}

#[test]
fn finite_box_cap_counts_the_cartesian_product_including_endpoints() {
    for (x_upper, expected) in [(15, true), (16, false)] {
        let mut limits = Limits::default();
        let mut facts = Facts::default();
        let (_, x) = variable(&mut facts, 0, 0, x_upper, &mut limits);
        let (_, y) = variable(&mut facts, 1, 0, 15, &mut limits);
        let complement = constant(15, &mut limits).subtract(&x, &mut limits).unwrap();
        let expression = x
            .multiply(&complement, &mut limits)
            .unwrap()
            .multiply(&y, &mut limits)
            .unwrap();
        let outcome = expression.prove_within(&interval(0, 840), &facts, &mut limits);
        assert_eq!(matches!(outcome, Outcome::Proven(_)), expected);
        if let Outcome::Proven(proof) = outcome {
            assert_eq!(proof.enclosure(), &interval(0, 840));
            proof.replay(&facts, &mut Limits::default()).unwrap();
        }
    }
}

#[test]
fn quadratic_vertices_are_exact_over_large_signed_integer_domains() {
    for (a, b, c) in [(1, 1, 0), (1, -1, 3), (-1, 1, 5), (-2, -3, -4), (3, 5, 7)] {
        let mut limits = Limits::default();
        let mut facts = Facts::default();
        let (id, x) = variable(&mut facts, 0, -1000, 1000, &mut limits);
        let square = x.multiply(&x, &mut limits).unwrap();
        let expression = constant(a, &mut limits)
            .multiply(&square, &mut limits)
            .unwrap()
            .add(
                &constant(b, &mut limits).multiply(&x, &mut limits).unwrap(),
                &mut limits,
            )
            .unwrap()
            .add(&constant(c, &mut limits), &mut limits)
            .unwrap();
        let values = (-1000..=1000)
            .map(|x| a * x * x + b * x + c)
            .collect::<Vec<_>>();
        let low = *values.iter().min().unwrap();
        let high = *values.iter().max().unwrap();
        let Outcome::Proven(proof) =
            expression.prove_within(&interval(low, high), &facts, &mut limits)
        else {
            panic!("quadratic {a} {b} {c}");
        };
        assert_eq!(proof.enclosure(), &interval(low, high));
        proof.replay(&facts, &mut Limits::default()).unwrap();
        for wrong in [interval(low + 1, high), interval(low, high - 1)] {
            assert!(matches!(
                expression.prove_within(&wrong, &facts, &mut Limits::default()),
                Outcome::Unknown(_)
            ));
        }
        facts.assign(id.place, interval(-1000, 1000)).unwrap();
        assert!(proof.replay(&facts, &mut Limits::default()).is_err());
    }
}

#[test]
fn separable_quadratics_bound_large_boxes_and_keep_each_dependency() {
    let mut limits = Limits::default();
    let mut facts = Facts::default();
    let (x_id, x) = variable(&mut facts, 0, 0, 500, &mut limits);
    let (y_id, y) = variable(&mut facts, 1, 0, 501, &mut limits);
    let (_, z) = variable(&mut facts, 2, -3, 7, &mut limits);
    let first = x
        .multiply(
            &constant(500, &mut limits)
                .subtract(&x, &mut limits)
                .unwrap(),
            &mut limits,
        )
        .unwrap();
    let second = y
        .multiply(
            &constant(501, &mut limits)
                .subtract(&y, &mut limits)
                .unwrap(),
            &mut limits,
        )
        .unwrap();
    let expression = first
        .subtract(&second, &mut limits)
        .unwrap()
        .add(&z, &mut limits)
        .unwrap()
        .add(&constant(11, &mut limits), &mut limits)
        .unwrap();
    let Outcome::Proven(proof) =
        expression.prove_within(&interval(-62742, 62518), &facts, &mut limits)
    else {
        panic!("independent quadratic components");
    };
    assert_eq!(proof.enclosure(), &interval(-62742, 62518));
    proof.replay(&facts, &mut Limits::default()).unwrap();
    for wrong in [interval(-62741, 62518), interval(-62742, 62517)] {
        assert!(matches!(
            expression.prove_within(&wrong, &facts, &mut Limits::default()),
            Outcome::Unknown(_)
        ));
    }
    for id in [x_id, y_id] {
        let mut changed = facts.fork(&mut Limits::default()).unwrap();
        changed.assign(id.place, interval(0, 500)).unwrap();
        assert!(proof.replay(&changed, &mut Limits::default()).is_err());
    }
}

#[test]
fn separable_quadratic_ranges_match_independent_cartesian_enumeration() {
    for a in [-2, 1, 3] {
        for b in [-3, 0, 2] {
            let mut limits = Limits::default();
            let mut facts = Facts::default();
            let (_, x) = variable(&mut facts, 0, -12, 13, &mut limits);
            let (_, y) = variable(&mut facts, 1, -14, 11, &mut limits);
            let square_x = x.multiply(&x, &mut limits).unwrap();
            let square_y = y.multiply(&y, &mut limits).unwrap();
            let expression = constant(a, &mut limits)
                .multiply(&square_x, &mut limits)
                .unwrap()
                .add(
                    &constant(b, &mut limits).multiply(&x, &mut limits).unwrap(),
                    &mut limits,
                )
                .unwrap()
                .subtract(&square_y, &mut limits)
                .unwrap()
                .add(&y, &mut limits)
                .unwrap();
            let values = (-12i128..=13)
                .flat_map(|x| (-14i128..=11).map(move |y| a * x * x + b * x - y * y + y))
                .collect::<Vec<_>>();
            let low = *values.iter().min().unwrap();
            let high = *values.iter().max().unwrap();
            let Outcome::Proven(proof) =
                expression.prove_within(&interval(low, high), &facts, &mut limits)
            else {
                panic!("separable oracle {a} {b}");
            };
            assert_eq!(proof.enclosure(), &interval(low, high));
            proof.replay(&facts, &mut Limits::default()).unwrap();
            assert!(matches!(
                expression.prove_within(&interval(low + 1, high), &facts, &mut Limits::default()),
                Outcome::Unknown(_)
            ));
            assert!(matches!(
                expression.prove_within(&interval(low, high - 1), &facts, &mut Limits::default()),
                Outcome::Unknown(_)
            ));
        }
    }
}

#[test]
fn cubic_coordinate_elimination_keeps_quadratic_interior_extrema() {
    for (y_low, y_high, low, high) in [
        (0, 1000, 0, 62_500_000),
        (-1000, 0, -62_500_000, 0),
        (-1000, 1000, -62_500_000, 62_500_000),
    ] {
        let mut limits = Limits::default();
        let mut facts = Facts::default();
        let (_, x) = variable(&mut facts, 0, 0, 500, &mut limits);
        let (y_id, y) = variable(&mut facts, 1, y_low, y_high, &mut limits);
        let arch = x
            .multiply(
                &constant(500, &mut limits)
                    .subtract(&x, &mut limits)
                    .unwrap(),
                &mut limits,
            )
            .unwrap();
        let expression = arch.multiply(&y, &mut limits).unwrap();
        let Outcome::Proven(proof) =
            expression.prove_within(&interval(low, high), &facts, &mut limits)
        else {
            panic!("mixed monotone and quadratic coordinates");
        };
        assert_eq!(proof.enclosure(), &interval(low, high));
        proof.replay(&facts, &mut Limits::default()).unwrap();
        for wrong in [interval(low + 1, high), interval(low, high - 1)] {
            assert!(matches!(
                expression.prove_within(&wrong, &facts, &mut Limits::default()),
                Outcome::Unknown(_)
            ));
        }
        facts.assign(y_id.place, interval(y_low, y_high)).unwrap();
        assert!(proof.replay(&facts, &mut Limits::default()).is_err());
    }
}

#[test]
fn cubic_monotonicity_uses_quadratic_finite_difference_without_sampling() {
    let mut limits = Limits::default();
    let mut facts = Facts::default();
    let (_, x) = variable(&mut facts, 0, -1000, 1000, &mut limits);
    // p(x+1)-p(x)=3*x*x-3*x+1 is positive at every integer.
    let square = x.multiply(&x, &mut limits).unwrap();
    let expression = square
        .multiply(&x, &mut limits)
        .unwrap()
        .subtract(
            &constant(3, &mut limits)
                .multiply(&square, &mut limits)
                .unwrap(),
            &mut limits,
        )
        .unwrap()
        .add(
            &constant(3, &mut limits).multiply(&x, &mut limits).unwrap(),
            &mut limits,
        )
        .unwrap();
    let Outcome::Proven(proof) =
        expression.prove_within(&interval(-1_003_003_000, 997_003_000), &facts, &mut limits)
    else {
        panic!("nonnegative quadratic finite difference");
    };
    proof.replay(&facts, &mut Limits::default()).unwrap();
}

#[test]
fn integer_derivative_samples_do_not_certify_real_monotonicity() {
    let mut limits = Limits::default();
    let mut facts = Facts::default();
    let (_, x) = variable(&mut facts, 0, 0, 1, &mut limits);
    let square = x.multiply(&x, &mut limits).unwrap();
    let cube = square.multiply(&x, &mut limits).unwrap();
    // p'(0)=p'(1)=1, yet p(1)-p(0)=-1. Applying integer vertex
    // bounds to the derivative would incorrectly infer increasing behavior.
    let expression = constant(4, &mut limits)
        .multiply(&cube, &mut limits)
        .unwrap()
        .subtract(
            &constant(6, &mut limits)
                .multiply(&square, &mut limits)
                .unwrap(),
            &mut limits,
        )
        .unwrap()
        .add(&x, &mut limits)
        .unwrap();
    assert!(matches!(
        expression.prove_within(&interval(0, 1), &facts, &mut limits),
        Outcome::Unknown(_)
    ));
    assert!(matches!(
        expression.prove_within(&interval(-1, 0), &facts, &mut Limits::default()),
        Outcome::Proven(_)
    ));
}

#[test]
fn quadratic_square_sums_preserve_cross_term_correlations_over_large_boxes() {
    let mut limits = Limits::default();
    let mut facts = Facts::default();
    let (_, x) = variable(&mut facts, 0, -1000, 1000, &mut limits);
    let (_, y) = variable(&mut facts, 1, -1000, 1000, &mut limits);
    let (z_id, z) = variable(&mut facts, 2, -1000, 1000, &mut limits);
    let xy = x.subtract(&y, &mut limits).unwrap();
    let yz = y.add(&z, &mut limits).unwrap();
    let expression = xy
        .multiply(&xy, &mut limits)
        .unwrap()
        .add(&yz.multiply(&yz, &mut limits).unwrap(), &mut limits)
        .unwrap()
        .add(&constant(7, &mut limits), &mut limits)
        .unwrap();
    let Outcome::Proven(proof) =
        expression.prove_within(&interval(7, 8_000_007), &facts, &mut limits)
    else {
        panic!("sum of correlated squares");
    };
    proof.replay(&facts, &mut Limits::default()).unwrap();
    let negative = constant(-1, &mut limits)
        .multiply(&expression, &mut limits)
        .unwrap();
    assert!(matches!(
        negative.prove_within(&interval(-8_000_007, -7), &facts, &mut limits),
        Outcome::Proven(_)
    ));
    assert!(matches!(
        expression.prove_within(&interval(8, 8_000_007), &facts, &mut limits),
        Outcome::Unknown(_)
    ));
    facts.assign(z_id.place, interval(-1000, 1000)).unwrap();
    assert!(proof.replay(&facts, &mut Limits::default()).is_err());
}

#[test]
fn quadratic_cross_term_certificates_agree_with_concrete_counterexamples() {
    for (a, b, c) in [
        (1, 2, 1),
        (1, -2, 1),
        (1, 3, 1),
        (-1, 2, -1),
        (0, 2, 1),
        (1, 2, 0),
        (1, -3, 1),
    ] {
        let mut limits = Limits::default();
        let mut facts = Facts::default();
        let (_, x) = variable(&mut facts, 0, -1000, 1000, &mut limits);
        let (_, y) = variable(&mut facts, 1, -1000, 1000, &mut limits);
        let xx = x.multiply(&x, &mut limits).unwrap();
        let xy = x.multiply(&y, &mut limits).unwrap();
        let yy = y.multiply(&y, &mut limits).unwrap();
        let expression = constant(a, &mut limits)
            .multiply(&xx, &mut limits)
            .unwrap()
            .add(
                &constant(b, &mut limits).multiply(&xy, &mut limits).unwrap(),
                &mut limits,
            )
            .unwrap()
            .add(
                &constant(c, &mut limits).multiply(&yy, &mut limits).unwrap(),
                &mut limits,
            )
            .unwrap();
        let values = (-2i128..=2)
            .flat_map(|x| (-2i128..=2).map(move |y| a * x * x + b * x * y + c * y * y))
            .collect::<Vec<_>>();
        let nonnegative = expression.prove_within(&interval(0, 10_000_000), &facts, &mut limits);
        if values.iter().any(|value| *value < 0) {
            assert!(
                matches!(nonnegative, Outcome::Unknown(_)),
                "unsound lower: {a} {b} {c}"
            );
        } else {
            assert!(
                matches!(nonnegative, Outcome::Proven(_)),
                "missing lower: {a} {b} {c}"
            );
        }
        let nonpositive =
            expression.prove_within(&interval(-10_000_000, 0), &facts, &mut Limits::default());
        if values.iter().any(|value| *value > 0) {
            assert!(
                matches!(nonpositive, Outcome::Unknown(_)),
                "unsound upper: {a} {b} {c}"
            );
        } else {
            assert!(
                matches!(nonpositive, Outcome::Proven(_)),
                "missing upper: {a} {b} {c}"
            );
        }
    }
}

#[test]
fn semidefinite_certificates_cover_weighted_affine_square_sums_and_singular_pivots() {
    for offset in [0, 1, -3] {
        let mut limits = Limits::default();
        let mut facts = Facts::default();
        let (x_id, x) = variable(&mut facts, 0, -1000, 1000, &mut limits);
        let (_, y) = variable(&mut facts, 1, -1000, 1000, &mut limits);
        let (_, z) = variable(&mut facts, 2, -1000, 1000, &mut limits);
        let first = constant(2, &mut limits)
            .multiply(&x, &mut limits)
            .unwrap()
            .subtract(&y, &mut limits)
            .unwrap()
            .add(&constant(offset, &mut limits), &mut limits)
            .unwrap();
        let second = constant(3, &mut limits)
            .multiply(&y, &mut limits)
            .unwrap()
            .add(&z, &mut limits)
            .unwrap()
            .subtract(&constant(2, &mut limits), &mut limits)
            .unwrap();
        let expression = first
            .multiply(&first, &mut limits)
            .unwrap()
            .add(&second.multiply(&second, &mut limits).unwrap(), &mut limits)
            .unwrap();
        let Outcome::Proven(proof) =
            expression.prove_within(&interval(0, 30_000_000), &facts, &mut limits)
        else {
            panic!("affine square sum offset {offset}");
        };
        proof.replay(&facts, &mut Limits::default()).unwrap();
        let negative = constant(-1, &mut limits)
            .multiply(&expression, &mut limits)
            .unwrap();
        assert!(matches!(
            negative.prove_within(&interval(-30_000_000, 0), &facts, &mut limits),
            Outcome::Proven(_)
        ));
        let mut tiny = Limits::default();
        tiny.arithmetic.steps = 10;
        assert!(matches!(
            expression.prove_within(&interval(0, 30_000_000), &facts, &mut tiny),
            Outcome::Unknown(_)
        ));
        proof.replay(&facts, &mut Limits::default()).unwrap();
        facts.assign(x_id.place, interval(-1000, 1000)).unwrap();
        assert!(proof.replay(&facts, &mut Limits::default()).is_err());
    }
}

#[test]
fn semidefinite_certificates_reject_linear_zero_pivots_and_indefinite_perturbations() {
    for (a, b, c) in [(2, -1, 0), (2, -1, 1), (3, 2, -2), (-1, 4, 3)] {
        let mut limits = Limits::default();
        let mut facts = Facts::default();
        let (_, x) = variable(&mut facts, 0, -1000, 1000, &mut limits);
        let (_, y) = variable(&mut facts, 1, -1000, 1000, &mut limits);
        let affine = constant(a, &mut limits)
            .multiply(&x, &mut limits)
            .unwrap()
            .add(
                &constant(b, &mut limits).multiply(&y, &mut limits).unwrap(),
                &mut limits,
            )
            .unwrap()
            .add(&constant(c, &mut limits), &mut limits)
            .unwrap();
        let square = affine.multiply(&affine, &mut limits).unwrap();
        for shift in [0, -1, -10] {
            let expression = square
                .add(&constant(shift, &mut limits), &mut limits)
                .unwrap();
            let outcome = expression.prove_within(&interval(0, 100_000_000), &facts, &mut limits);
            let counterexample = (-10i128..=10)
                .any(|x| (-10i128..=10).any(|y| (a * x + b * y + c).pow(2) + shift < 0));
            if counterexample {
                assert!(
                    matches!(outcome, Outcome::Unknown(_)),
                    "accepted negative value {a} {b} {c} {shift}"
                );
            } else if shift == 0 {
                assert!(
                    matches!(outcome, Outcome::Proven(_)),
                    "missing square certificate"
                );
            }
        }
        let xy = x.multiply(&y, &mut limits).unwrap();
        let zero_pivot = xy.add(&x, &mut limits).unwrap();
        assert!(matches!(
            zero_pivot.prove_within(&interval(0, 100_000_000), &facts, &mut limits),
            Outcome::Unknown(_)
        ));
    }
}

#[test]
fn semidefinite_range_targets_shift_lower_and_upper_certificates_independently() {
    let mut limits = Limits::default();
    let mut facts = Facts::default();
    let (_, x) = variable(&mut facts, 0, -1000, 1000, &mut limits);
    let (id, y) = variable(&mut facts, 1, -1000, 1000, &mut limits);
    let affine = constant(2, &mut limits)
        .multiply(&x, &mut limits)
        .unwrap()
        .subtract(&y, &mut limits)
        .unwrap()
        .add(&constant(1, &mut limits), &mut limits)
        .unwrap();
    let square = affine.multiply(&affine, &mut limits).unwrap();
    for (expression, low, high) in [
        (
            square.add(&constant(7, &mut limits), &mut limits).unwrap(),
            7,
            10_000_000,
        ),
        (
            constant(100, &mut limits)
                .subtract(&square, &mut limits)
                .unwrap(),
            -10_000_000,
            100,
        ),
    ] {
        let Outcome::Proven(proof) =
            expression.prove_within(&interval(low, high), &facts, &mut limits)
        else {
            panic!("shifted semidefinite range");
        };
        proof.replay(&facts, &mut Limits::default()).unwrap();
        assert!(matches!(
            expression.prove_within(&interval(8, 99), &facts, &mut Limits::default()),
            Outcome::Unknown(_)
        ));
        let mut changed = facts.fork(&mut Limits::default()).unwrap();
        changed.assign(id.place, interval(-1000, 1000)).unwrap();
        assert!(proof.replay(&changed, &mut Limits::default()).is_err());
    }
    // Both bounds need refinement. Proving the lower bound cannot let its
    // constant shift manufacture a successful upper bound for an impossible box.
    let shifted = square.add(&constant(7, &mut limits), &mut limits).unwrap();
    for target in [interval(7, 8), interval(8, 10_000_000)] {
        assert!(matches!(
            shifted.prove_within(&target, &facts, &mut Limits::default()),
            Outcome::Unknown(_)
        ));
    }
}

#[test]
fn cubic_extrema_match_exhaustive_integer_domains_beyond_the_enumeration_cap() {
    for (low, high) in [(-200, 200), (-300, -1), (0, 300), (5, 350)] {
        for a in [-3, -1, 1, 3] {
            for b in [-5, 0, 5] {
                for c in [-7, 0, 7] {
                    let mut limits = Limits::default();
                    let mut facts = Facts::default();
                    let (_, x) = variable(&mut facts, 0, low, high, &mut limits);
                    let square = x.multiply(&x, &mut limits).unwrap();
                    let cube = square.multiply(&x, &mut limits).unwrap();
                    let expression = constant(a, &mut limits)
                        .multiply(&cube, &mut limits)
                        .unwrap()
                        .add(
                            &constant(b, &mut limits)
                                .multiply(&square, &mut limits)
                                .unwrap(),
                            &mut limits,
                        )
                        .unwrap()
                        .add(
                            &constant(c, &mut limits).multiply(&x, &mut limits).unwrap(),
                            &mut limits,
                        )
                        .unwrap()
                        .add(&constant(11, &mut limits), &mut limits)
                        .unwrap();
                    let values = (low..=high)
                        .map(|x| a * x * x * x + b * x * x + c * x + 11)
                        .collect::<Vec<_>>();
                    let minimum = *values.iter().min().unwrap();
                    let maximum = *values.iter().max().unwrap();
                    let Outcome::Proven(proof) =
                        expression.prove_within(&interval(minimum, maximum), &facts, &mut limits)
                    else {
                        panic!("cubic {a} {b} {c}, [{low},{high}]");
                    };
                    assert_eq!(proof.enclosure(), &interval(minimum, maximum));
                    proof.replay(&facts, &mut Limits::default()).unwrap();
                    for wrong in [
                        interval(minimum + 1, maximum),
                        interval(minimum, maximum - 1),
                    ] {
                        assert!(matches!(
                            expression.prove_within(&wrong, &facts, &mut Limits::default()),
                            Outcome::Unknown(_)
                        ));
                    }
                }
            }
        }
    }
}

#[test]
fn cubic_interior_extrema_preserve_budget_and_input_versions() {
    let mut limits = Limits::default();
    let mut facts = Facts::default();
    let (id, x) = variable(&mut facts, 0, 0, 300, &mut limits);
    let complement = constant(300, &mut limits)
        .subtract(&x, &mut limits)
        .unwrap();
    let expression = x
        .multiply(&x, &mut limits)
        .unwrap()
        .multiply(&complement, &mut limits)
        .unwrap();
    let target = interval(0, 4_000_000);
    let Outcome::Proven(proof) = expression.prove_within(&target, &facts, &mut limits) else {
        panic!("interior maximum")
    };
    assert_eq!(proof.enclosure(), &target);
    let mut tiny = Limits::default();
    tiny.arithmetic.steps = 10;
    assert!(matches!(
        expression.prove_within(&target, &facts, &mut tiny),
        Outcome::Unknown(_)
    ));
    proof.replay(&facts, &mut Limits::default()).unwrap();
    facts.assign(id.place, interval(0, 300)).unwrap();
    assert!(proof.replay(&facts, &mut Limits::default()).is_err());
}

#[test]
fn independent_cubic_components_prove_large_cartesian_domains() {
    let mut limits = Limits::default();
    let mut facts = Facts::default();
    let (x_id, x) = variable(&mut facts, 0, 0, 300, &mut limits);
    let (y_id, y) = variable(&mut facts, 1, 0, 300, &mut limits);
    let (_, z) = variable(&mut facts, 2, -2, 3, &mut limits);
    let cx = constant(300, &mut limits)
        .subtract(&x, &mut limits)
        .unwrap();
    let cy = constant(300, &mut limits)
        .subtract(&y, &mut limits)
        .unwrap();
    let px = x
        .multiply(&x, &mut limits)
        .unwrap()
        .multiply(&cx, &mut limits)
        .unwrap();
    let py = y
        .multiply(&y, &mut limits)
        .unwrap()
        .multiply(&cy, &mut limits)
        .unwrap();
    let expression = px
        .subtract(&py, &mut limits)
        .unwrap()
        .add(&z, &mut limits)
        .unwrap()
        .add(&constant(7, &mut limits), &mut limits)
        .unwrap();
    let target = interval(-3_999_995, 4_000_010);
    let Outcome::Proven(proof) = expression.prove_within(&target, &facts, &mut limits) else {
        panic!("independent cubic extrema")
    };
    assert_eq!(proof.enclosure(), &target);
    proof.replay(&facts, &mut Limits::default()).unwrap();
    for wrong in [
        interval(-3_999_994, 4_000_010),
        interval(-3_999_995, 4_000_009),
    ] {
        assert!(matches!(
            expression.prove_within(&wrong, &facts, &mut Limits::default()),
            Outcome::Unknown(_)
        ));
    }
    for id in [x_id, y_id] {
        let mut changed = facts.fork(&mut Limits::default()).unwrap();
        changed.assign(id.place, interval(0, 300)).unwrap();
        assert!(proof.replay(&changed, &mut Limits::default()).is_err());
    }
}

#[test]
fn independent_cubic_components_match_concrete_cartesian_extrema() {
    for a in [-2, 1, 3] {
        for b in [-3, 0, 2] {
            let mut limits = Limits::default();
            let mut facts = Facts::default();
            let (_, x) = variable(&mut facts, 0, -12, 13, &mut limits);
            let (_, y) = variable(&mut facts, 1, -14, 11, &mut limits);
            let xx = x.multiply(&x, &mut limits).unwrap();
            let xxx = xx.multiply(&x, &mut limits).unwrap();
            let yy = y.multiply(&y, &mut limits).unwrap();
            let yyy = yy.multiply(&y, &mut limits).unwrap();
            let expression = constant(a, &mut limits)
                .multiply(&xxx, &mut limits)
                .unwrap()
                .add(
                    &constant(b, &mut limits).multiply(&xx, &mut limits).unwrap(),
                    &mut limits,
                )
                .unwrap()
                .subtract(&yyy, &mut limits)
                .unwrap()
                .add(&y, &mut limits)
                .unwrap();
            let values = (-12i128..=13)
                .flat_map(|x| {
                    (-14i128..=11).map(move |y| a * x * x * x + b * x * x - y * y * y + y)
                })
                .collect::<Vec<_>>();
            let low = *values.iter().min().unwrap();
            let high = *values.iter().max().unwrap();
            let Outcome::Proven(proof) =
                expression.prove_within(&interval(low, high), &facts, &mut limits)
            else {
                panic!("cartesian cubic oracle")
            };
            assert_eq!(proof.enclosure(), &interval(low, high));
            for wrong in [interval(low + 1, high), interval(low, high - 1)] {
                assert!(matches!(
                    expression.prove_within(&wrong, &facts, &mut Limits::default()),
                    Outcome::Unknown(_)
                ));
            }
        }
    }
}

#[test]
fn unbounded_quadratic_predicates_prove_global_signs_without_inventing_intervals() {
    use elaboration::verification::{Comparison as C, Condition};
    let mut limits = Limits::default();
    let mut facts = Facts::default();
    let x_id = facts.invalidate(PlaceId { owner: 1, local: 0 }).unwrap();
    let y_id = facts.invalidate(PlaceId { owner: 1, local: 1 }).unwrap();
    let x = Polynomial::variable(x_id, &mut limits).unwrap();
    let y = Polynomial::variable(y_id, &mut limits).unwrap();
    let affine = constant(2, &mut limits)
        .multiply(&x, &mut limits)
        .unwrap()
        .subtract(&y, &mut limits)
        .unwrap()
        .add(&constant(1, &mut limits), &mut limits)
        .unwrap();
    let square = affine.multiply(&affine, &mut limits).unwrap();
    let positive = square.add(&constant(1, &mut limits), &mut limits).unwrap();
    for (expression, relation, truth) in [
        (&square, C::GreaterEqual, true),
        (&square, C::Less, false),
        (&positive, C::Greater, true),
        (&positive, C::LessEqual, false),
        (&positive, C::Equal, false),
        (&positive, C::NotEqual, true),
    ] {
        let condition = Condition::PolynomialCompare {
            left: expression.clone(),
            relation,
            right: constant(0, &mut limits),
        };
        let proof = facts
            .prove_condition(&condition, truth, &mut Limits::default())
            .unwrap();
        proof.replay(&facts, &mut Limits::default()).unwrap();
        assert!(
            facts
                .prove_condition(&condition, !truth, &mut Limits::default())
                .is_err()
        );
        let mut changed = facts.fork(&mut Limits::default()).unwrap();
        changed.invalidate(y_id.place).unwrap();
        assert!(proof.replay(&changed, &mut Limits::default()).is_err());
    }
    for relation in [C::Greater, C::Equal, C::NotEqual] {
        let condition = Condition::PolynomialCompare {
            left: square.clone(),
            relation,
            right: constant(0, &mut limits),
        };
        for truth in [true, false] {
            assert!(
                facts
                    .prove_condition(&condition, truth, &mut Limits::default())
                    .is_err(),
                "{relation:?} {truth}"
            );
        }
    }
    assert!(matches!(
        facts.bound(x_id, &mut limits),
        Err(Unknown::MissingBound(_))
    ));
    assert!(matches!(
        facts.bound(y_id, &mut limits),
        Err(Unknown::MissingBound(_))
    ));
}

#[test]
fn unbounded_negative_quadratic_certificates_preserve_strictness_and_budget_recovery() {
    use elaboration::verification::{Comparison as C, Condition};
    let mut limits = Limits::default();
    let mut facts = Facts::default();
    let id = facts.invalidate(PlaceId { owner: 1, local: 0 }).unwrap();
    let x = Polynomial::variable(id, &mut limits).unwrap();
    let negative = constant(-1, &mut limits)
        .subtract(&x.multiply(&x, &mut limits).unwrap(), &mut limits)
        .unwrap();
    for (relation, truth) in [
        (C::Less, true),
        (C::GreaterEqual, false),
        (C::NotEqual, true),
        (C::Equal, false),
    ] {
        let condition = Condition::PolynomialCompare {
            left: negative.clone(),
            relation,
            right: constant(0, &mut limits),
        };
        let mut tiny = Limits::default();
        tiny.arithmetic.steps = 1;
        assert!(facts.prove_condition(&condition, truth, &mut tiny).is_err());
        facts
            .prove_condition(&condition, truth, &mut Limits::default())
            .unwrap()
            .replay(&facts, &mut Limits::default())
            .unwrap();
    }
}

#[test]
fn unbounded_integer_quadratic_signs_match_lattice_extrema() {
    use elaboration::verification::{Comparison as C, Condition};
    for a in [-3, -2, -1, 1, 2, 3] {
        for b in -5..=5 {
            for c in -2..=2 {
                let mut limits = Limits::default();
                let mut facts = Facts::default();
                let id = facts.invalidate(PlaceId { owner: 1, local: 0 }).unwrap();
                let x = Polynomial::variable(id, &mut limits).unwrap();
                let square = x.multiply(&x, &mut limits).unwrap();
                let expression = constant(a, &mut limits)
                    .multiply(&square, &mut limits)
                    .unwrap()
                    .add(
                        &constant(b, &mut limits).multiply(&x, &mut limits).unwrap(),
                        &mut limits,
                    )
                    .unwrap()
                    .add(&constant(c, &mut limits), &mut limits)
                    .unwrap();
                // Every vertex lies within [-3,3], and the outer samples expose
                // the unbounded tail's sign for all coefficients in this family.
                for relation in [
                    C::GreaterEqual,
                    C::Greater,
                    C::LessEqual,
                    C::Less,
                    C::Equal,
                    C::NotEqual,
                ] {
                    let condition = Condition::PolynomialCompare {
                        left: expression.clone(),
                        relation,
                        right: constant(0, &mut limits),
                    };
                    for truth in [true, false] {
                        let all = (-20i128..=20).all(|x| {
                            let value = a * x * x + b * x + c;
                            (match relation {
                                C::GreaterEqual => value >= 0,
                                C::Greater => value > 0,
                                C::LessEqual => value <= 0,
                                C::Less => value < 0,
                                C::Equal => value == 0,
                                C::NotEqual => value != 0,
                            }) == truth
                        });
                        let proof =
                            facts.prove_condition(&condition, truth, &mut Limits::default());
                        if !all {
                            assert!(proof.is_err(), "unsound {a} {b} {c} {relation:?} {truth}");
                        }
                        // A nonzero quadratic may have no integer root while changing
                        // sign, which the extremum-only inequality rule need not solve.
                        else if !matches!(relation, C::Equal | C::NotEqual) {
                            proof
                                .unwrap_or_else(|e| {
                                    panic!("missing {a} {b} {c} {relation:?}: {e:?}")
                                })
                                .replay(&facts, &mut Limits::default())
                                .unwrap();
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn facts_dump_distinguishes_current_and_stale_retained_conditions() {
    use elaboration::verification::{Comparison, Condition};
    let mut limits = Limits::default();
    let mut facts = Facts::default();
    let (id, _) = variable(&mut facts, 0, -2, 2, &mut limits);
    let condition = Condition::Compare {
        value: id,
        relation: Comparison::NotEqual,
        constant: Integer::zero(),
    };
    facts.assume(&condition, true, &mut limits).unwrap();
    let proof = facts
        .prove_condition(&condition, true, &mut limits)
        .unwrap();
    let before = facts.to_string();
    assert!(before.contains("(established (truth true)"), "{before}");
    assert!(!before.contains("stale-condition"));
    proof.replay(&facts, &mut limits).unwrap();
    let fresh = facts.assign(id.place, interval(-2, 2)).unwrap();
    let after = facts.to_string();
    assert!(after.contains("(stale-condition (truth true)"), "{after}");
    assert!(!after.contains("(established (truth true)"));
    assert_eq!(
        after,
        facts.to_string(),
        "dump order must stay deterministic"
    );
    assert!(
        after.contains(&id.to_string()),
        "retained old identity must remain visible"
    );
    assert!(
        after.contains(&fresh.to_string()),
        "current identity must remain visible"
    );
    assert!(proof.replay(&facts, &mut limits).is_err());
    let fresh_condition = Condition::Compare {
        value: fresh,
        relation: Comparison::NotEqual,
        constant: Integer::zero(),
    };
    assert!(
        facts
            .prove_condition(&fresh_condition, true, &mut limits)
            .is_err(),
        "formatting must not publish old facts"
    );
    let mut depth = 0;
    for ch in after.chars() {
        if ch == '(' {
            depth += 1;
        } else if ch == ')' {
            depth -= 1;
            assert!(depth >= 0);
        }
    }
    assert_eq!(depth, 0);
}

#[test]
fn integer_residue_certificates_preserve_versions_and_budget_recovery() {
    use elaboration::verification::{Comparison as C, Condition};
    for bounded in [false, true] {
        let mut facts = Facts::default();
        let mut limits = Limits::default();
        let place = PlaceId { owner: 1, local: 0 };
        let id = if bounded {
            facts.assign(place, interval(-1000, 1000)).unwrap()
        } else {
            facts.invalidate(place).unwrap()
        };
        let x = Polynomial::variable(id, &mut limits).unwrap();
        let square = x.multiply(&x, &mut limits).unwrap();
        // Mixed signs, unlike a sign-definite interval certificate.
        let p = square
            .multiply(&constant(6, &mut limits), &mut limits)
            .unwrap()
            .add(
                &x.multiply(&constant(-4, &mut limits), &mut limits).unwrap(),
                &mut limits,
            )
            .unwrap()
            .add(&constant(-1, &mut limits), &mut limits)
            .unwrap();
        for relation in [C::Equal, C::NotEqual] {
            let condition = Condition::PolynomialCompare {
                left: p.clone(),
                relation,
                right: constant(0, &mut limits),
            };
            let truth = relation == C::NotEqual;
            let mut exhausted = Limits::default();
            exhausted.arithmetic.steps = 0;
            assert!(
                facts
                    .prove_condition(&condition, truth, &mut exhausted)
                    .is_err()
            );
            let proof = facts
                .prove_condition(&condition, truth, &mut limits)
                .unwrap();
            proof.replay(&facts, &mut limits).unwrap();
            assert!(
                facts
                    .prove_condition(&condition, !truth, &mut limits)
                    .is_err()
            );
            let mut changed = facts.fork(&mut limits).unwrap();
            changed.invalidate(place).unwrap();
            assert!(proof.replay(&changed, &mut limits).is_err());
            assert!(
                changed
                    .prove_condition(&condition, truth, &mut limits)
                    .is_err()
            );
        }
    }
}

#[test]
fn multivariate_integer_residue_proofs_agree_with_concrete_witnesses() {
    use elaboration::verification::{Comparison as C, Condition};
    let mut limits = Limits::default();
    let mut facts = Facts::default();
    let x = Polynomial::variable(
        facts.invalidate(PlaceId { owner: 1, local: 0 }).unwrap(),
        &mut limits,
    )
    .unwrap();
    let y = Polynomial::variable(
        facts.invalidate(PlaceId { owner: 1, local: 1 }).unwrap(),
        &mut limits,
    )
    .unwrap();
    let xy = x.multiply(&y, &mut limits).unwrap();
    for a in [-6_i128, -2, 2, 6] {
        for b in [-4_i128, -1, 1, 4] {
            for c in -3_i128..=3 {
                let p = xy
                    .multiply(&constant(a, &mut limits), &mut limits)
                    .unwrap()
                    .add(
                        &y.multiply(&constant(b, &mut limits), &mut limits).unwrap(),
                        &mut limits,
                    )
                    .unwrap()
                    .add(&constant(c, &mut limits), &mut limits)
                    .unwrap();
                let condition = Condition::PolynomialCompare {
                    left: p,
                    relation: C::NotEqual,
                    right: constant(0, &mut limits),
                };
                let proof = facts.prove_condition(&condition, true, &mut Limits::default());
                let has_zero = (-10..=10).any(|x| (-10..=10).any(|y| a * x * y + b * y + c == 0));
                if has_zero {
                    assert!(proof.is_err(), "{a}xy + {b}y + {c}");
                }
                if a % 2 == 0 && b % 2 == 0 && c % 2 != 0 {
                    proof
                        .unwrap()
                        .replay(&facts, &mut Limits::default())
                        .unwrap();
                }
            }
        }
    }
}

#[test]
fn modular_nonzero_certificates_match_exhaustive_integer_witnesses() {
    use elaboration::verification::{Comparison as C, Condition};
    let mut facts = Facts::default();
    let mut limits = Limits::default();
    let id = facts.invalidate(PlaceId { owner: 1, local: 0 }).unwrap();
    let x = Polynomial::variable(id, &mut limits).unwrap();
    let square = x.multiply(&x, &mut limits).unwrap();
    let cube = square.multiply(&x, &mut limits).unwrap();
    for a in -2_i128..=2 {
        for b in -2_i128..=2 {
            for c in -2_i128..=2 {
                for d in -2_i128..=2 {
                    let p = cube
                        .multiply(&constant(a, &mut limits), &mut limits)
                        .unwrap()
                        .add(
                            &square
                                .multiply(&constant(b, &mut limits), &mut limits)
                                .unwrap(),
                            &mut limits,
                        )
                        .unwrap()
                        .add(
                            &x.multiply(&constant(c, &mut limits), &mut limits).unwrap(),
                            &mut limits,
                        )
                        .unwrap()
                        .add(&constant(d, &mut limits), &mut limits)
                        .unwrap();
                    let condition = Condition::PolynomialCompare {
                        left: p,
                        relation: C::NotEqual,
                        right: constant(0, &mut limits),
                    };
                    let proof = facts.prove_condition(&condition, true, &mut Limits::default());
                    if (-20..=20).any(|v| a * v * v * v + b * v * v + c * v + d == 0) {
                        assert!(proof.is_err(), "{a} {b} {c} {d}");
                    } else if [2_i128, 3, 5]
                        .iter()
                        .any(|m| (0..*m).all(|v| (a * v * v * v + b * v * v + c * v + d) % m != 0))
                    {
                        proof
                            .unwrap_or_else(|e| panic!("{a} {b} {c} {d}: {e:?}"))
                            .replay(&facts, &mut Limits::default())
                            .unwrap();
                    }
                }
            }
        }
    }
}

#[test]
fn modular_certificates_do_not_survive_budget_exhaustion_or_input_rewrite() {
    use elaboration::verification::{Comparison as C, Condition};
    let mut facts = Facts::default();
    let mut limits = Limits::default();
    let place = PlaceId { owner: 1, local: 0 };
    let id = facts.invalidate(place).unwrap();
    let x = Polynomial::variable(id, &mut limits).unwrap();
    let p = x
        .multiply(&x, &mut limits)
        .unwrap()
        .multiply(&x, &mut limits)
        .unwrap()
        .subtract(&x, &mut limits)
        .unwrap()
        .add(&constant(1, &mut limits), &mut limits)
        .unwrap();
    let condition = Condition::PolynomialCompare {
        left: p,
        relation: C::NotEqual,
        right: constant(0, &mut limits),
    };
    for steps in [0, 1, 10, 30] {
        let mut tiny = Limits::default();
        tiny.arithmetic.steps = steps;
        assert!(facts.prove_condition(&condition, true, &mut tiny).is_err());
    }
    let proof = facts
        .prove_condition(&condition, true, &mut limits)
        .unwrap();
    proof.replay(&facts, &mut limits).unwrap();
    facts.invalidate(place).unwrap();
    assert!(proof.replay(&facts, &mut limits).is_err());
    assert!(
        facts
            .prove_condition(&condition, true, &mut limits)
            .is_err()
    );
}

#[test]
fn prime_power_residue_certificates_and_search_cutoffs_are_sound() {
    use elaboration::verification::{Comparison as C, Condition};
    let mut limits = Limits::default();
    let mut facts = Facts::default();
    let x = Polynomial::variable(
        facts.invalidate(PlaceId { owner: 1, local: 0 }).unwrap(),
        &mut limits,
    )
    .unwrap();
    let square = x.multiply(&x, &mut limits).unwrap();
    // Each has roots modulo 2, 3 and 5. Their obstructions require 4, 8,
    // and 9 respectively; intervals straddle zero and cannot prove nonzero.
    for (offset, modulus) in [(6_i128, 4_i128), (21, 8), (24, 9)] {
        for m in [2_i128, 3, 5] {
            assert!((0..m).any(|v| (v * v - offset).rem_euclid(m) == 0));
        }
        assert!((0..modulus).all(|v| (v * v - offset).rem_euclid(modulus) != 0));
        let condition = Condition::PolynomialCompare {
            left: square
                .subtract(&constant(offset, &mut limits), &mut limits)
                .unwrap(),
            relation: C::NotEqual,
            right: constant(0, &mut limits),
        };
        facts
            .prove_condition(&condition, true, &mut Limits::default())
            .unwrap()
            .replay(&facts, &mut Limits::default())
            .unwrap();
    }
    for count in [8, 9] {
        let mut facts = Facts::default();
        let mut p = constant(1, &mut limits);
        for local in 0..count {
            let x = Polynomial::variable(
                facts.invalidate(PlaceId { owner: 2, local }).unwrap(),
                &mut limits,
            )
            .unwrap();
            let term = x
                .multiply(&x, &mut limits)
                .unwrap()
                .subtract(&x, &mut limits)
                .unwrap();
            p = p.add(&term, &mut limits).unwrap();
        }
        let condition = Condition::PolynomialCompare {
            left: p,
            relation: C::NotEqual,
            right: constant(0, &mut limits),
        };
        let result = facts.prove_condition(&condition, true, &mut Limits::default());
        if count == 8 {
            result
                .unwrap()
                .replay(&facts, &mut Limits::default())
                .unwrap();
        } else {
            assert!(
                result.is_err(),
                "unsearched assignments must not produce evidence"
            );
        }
    }
}

#[test]
fn bounded_residue_domains_prove_relative_nonzero_and_reject_stale_evidence() {
    use elaboration::verification::{Comparison as C, Condition};
    for (lo, hi) in [(1, 2), (-2, -1)] {
        let mut facts = Facts::default();
        let mut limits = Limits::default();
        let xid = facts.invalidate(PlaceId { owner: 1, local: 0 }).unwrap();
        let x = Polynomial::variable(xid, &mut limits).unwrap();
        let (yid, y) = variable(&mut facts, 1, lo, hi, &mut limits);
        let p = x
            .multiply(&x, &mut limits)
            .unwrap()
            .subtract(&y.multiply(&y, &mut limits).unwrap(), &mut limits)
            .unwrap()
            .subtract(&constant(1, &mut limits), &mut limits)
            .unwrap();
        let condition = Condition::PolynomialCompare {
            left: p,
            relation: C::NotEqual,
            right: constant(0, &mut limits),
        };
        let proof = facts
            .prove_condition(&condition, true, &mut limits)
            .unwrap();
        proof.replay(&facts, &mut limits).unwrap();
        facts.assign(yid.place, interval(-2, 2)).unwrap();
        assert!(proof.replay(&facts, &mut limits).is_err());
        assert!(
            facts
                .prove_condition(&condition, true, &mut limits)
                .is_err()
        );
    }
}

#[test]
fn nonzero_factor_evidence_is_required_on_every_joined_path() {
    use elaboration::verification::{Comparison as C, Condition};
    let mut limits = Limits::default();
    let mut facts = Facts::default();
    let (xid, x) = variable(&mut facts, 0, -5, 5, &mut limits);
    let (yid, y) = variable(&mut facts, 1, -5, 5, &mut limits);
    let product = x
        .multiply(&x, &mut limits)
        .unwrap()
        .multiply(&y, &mut limits)
        .unwrap();
    let condition = Condition::PolynomialCompare {
        left: product,
        relation: C::NotEqual,
        right: constant(0, &mut limits),
    };
    facts
        .assume(
            &Condition::Compare {
                value: xid,
                relation: C::NotEqual,
                constant: Integer::zero(),
            },
            true,
            &mut limits,
        )
        .unwrap();
    let only_x = facts.fork(&mut limits).unwrap();
    assert!(
        facts
            .prove_condition(&condition, true, &mut limits)
            .is_err()
    );
    facts
        .assume(
            &Condition::Compare {
                value: yid,
                relation: C::NotEqual,
                constant: Integer::zero(),
            },
            true,
            &mut limits,
        )
        .unwrap();
    let mut exhausted = Limits::default();
    exhausted.arithmetic.steps = 0;
    assert!(
        facts
            .prove_condition(&condition, true, &mut exhausted)
            .is_err()
    );
    let proof = facts
        .prove_condition(&condition, true, &mut limits)
        .unwrap();
    proof.replay(&facts, &mut limits).unwrap();
    let joined = facts.join(&only_x, &mut limits).unwrap();
    assert!(proof.replay(&joined, &mut limits).is_err());
    assert!(
        joined
            .prove_condition(&condition, true, &mut limits)
            .is_err()
    );
    facts.assign(yid.place, interval(-5, 5)).unwrap();
    assert!(proof.replay(&facts, &mut limits).is_err());
}

#[test]
fn common_factor_entailment_respects_truth_versions_and_joined_facts() {
    use elaboration::verification::{Comparison as C, Condition};
    for (relation, truth) in [
        (C::NotEqual, true),
        (C::Equal, false),
        (C::Greater, true),
        (C::Less, true),
    ] {
        let mut limits = Limits::default();
        let mut facts = Facts::default();
        let xid = facts.invalidate(PlaceId { owner: 1, local: 0 }).unwrap();
        let yid = facts.invalidate(PlaceId { owner: 1, local: 1 }).unwrap();
        let x = Polynomial::variable(xid, &mut limits).unwrap();
        let y = Polynomial::variable(yid, &mut limits).unwrap();
        let empty = facts.fork(&mut limits).unwrap();
        let known = Condition::PolynomialCompare {
            left: x.multiply(&y, &mut limits).unwrap(),
            relation,
            right: constant(0, &mut limits),
        };
        facts.assume(&known, truth, &mut limits).unwrap();
        for value in [xid, yid] {
            for (relation, truth) in [(C::NotEqual, true), (C::Equal, false)] {
                let query = Condition::Compare {
                    value,
                    relation,
                    constant: Integer::zero(),
                };
                let proof = facts.prove_condition(&query, truth, &mut limits).unwrap();
                proof.replay(&facts, &mut limits).unwrap();
                let joined = facts.join(&empty, &mut limits).unwrap();
                assert!(proof.replay(&joined, &mut limits).is_err());
                let mut changed = facts.fork(&mut limits).unwrap();
                // Rewriting the other factor also invalidates the premise.
                changed
                    .invalidate(if value == xid { yid.place } else { xid.place })
                    .unwrap();
                assert!(proof.replay(&changed, &mut limits).is_err());
            }
        }
    }
}

#[test]
fn monomial_sign_proofs_match_independent_signed_samples() {
    use elaboration::verification::{Comparison as C, Condition};
    let relations = [
        C::Equal,
        C::NotEqual,
        C::Less,
        C::LessEqual,
        C::Greater,
        C::GreaterEqual,
    ];
    for left_domain in [
        None,
        Some(C::Equal),
        Some(C::NotEqual),
        Some(C::Less),
        Some(C::LessEqual),
        Some(C::Greater),
        Some(C::GreaterEqual),
    ] {
        for right_domain in [
            None,
            Some(C::Equal),
            Some(C::NotEqual),
            Some(C::Less),
            Some(C::LessEqual),
            Some(C::Greater),
            Some(C::GreaterEqual),
        ] {
            for power in [1, 2] {
                for coefficient in [-1, 1] {
                    let mut facts = Facts::default();
                    let mut limits = Limits::default();
                    let xid = facts.invalidate(PlaceId { owner: 1, local: 0 }).unwrap();
                    let yid = facts.invalidate(PlaceId { owner: 1, local: 1 }).unwrap();
                    for (id, domain) in [(xid, left_domain), (yid, right_domain)] {
                        if let Some(relation) = domain {
                            facts
                                .assume(
                                    &Condition::Compare {
                                        value: id,
                                        relation,
                                        constant: Integer::zero(),
                                    },
                                    true,
                                    &mut limits,
                                )
                                .unwrap();
                        }
                    }
                    let x = Polynomial::variable(xid, &mut limits).unwrap();
                    let y = Polynomial::variable(yid, &mut limits).unwrap();
                    let mut p = x.clone();
                    if power == 2 {
                        p = p.multiply(&x, &mut limits).unwrap();
                    }
                    p = p
                        .multiply(&y, &mut limits)
                        .unwrap()
                        .multiply(&constant(coefficient, &mut limits), &mut limits)
                        .unwrap();
                    let holds = |value: i128, relation: C| match relation {
                        C::Equal => value == 0,
                        C::NotEqual => value != 0,
                        C::Less => value < 0,
                        C::LessEqual => value <= 0,
                        C::Greater => value > 0,
                        C::GreaterEqual => value >= 0,
                    };
                    for relation in relations {
                        let query = Condition::PolynomialCompare {
                            left: p.clone(),
                            relation,
                            right: constant(0, &mut limits),
                        };
                        let samples = (-2_i128..=2)
                            .filter(|v| left_domain.is_none_or(|r| holds(*v, r)))
                            .flat_map(|x| {
                                (-2_i128..=2)
                                    .filter(move |v| right_domain.is_none_or(|r| holds(*v, r)))
                                    .map(move |y| holds(coefficient * x.pow(power) * y, relation))
                            })
                            .collect::<Vec<_>>();
                        for truth in [false, true] {
                            let result =
                                facts.prove_condition(&query, truth, &mut Limits::default());
                            if samples.iter().all(|&actual| actual == truth) {
                                result.unwrap_or_else(|e|panic!("{left_domain:?} {right_domain:?} {power} {coefficient} {relation:?} {truth}: {e:?}"))
                                    .replay(&facts,&mut Limits::default()).unwrap();
                            } else {
                                assert!(result.is_err());
                            }
                        }
                    }
                }
            }
        }
    }
}
