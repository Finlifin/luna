use elaboration::{
    capabilities::World,
    implementation::{ImplementationHead, Index},
    ptc::{Budget, Context},
};
use nir::program::{
    AssociatedContract, Expr, ExprKind, Implementation, Trait, TraitRequirement, TypePredicate,
};
use nir::{AssociatedTypeOrigin, DeclarationId as Id, Primitive as P, Sort, Term};

fn id(local: u32) -> Id {
    Id { package: 1, local }
}
fn ty(p: P) -> Term {
    Term::Primitive(p)
}
fn rigid(n: u32) -> Term {
    Term::Rigid {
        declaration: id(n),
        sort: Sort::Type,
    }
}
fn holder(t: Term) -> Term {
    Term::Nominal {
        declaration: id(10),
        arguments: vec![t],
    }
}
fn value(t: Term) -> Expr {
    Expr {
        ty: ty(P::Type),
        source: nir::program::SourceRange { start: 0, end: 0 },
        kind: ExprKind::TypeValue(t),
    }
}
fn projection(t: Term, args: Vec<Term>) -> Term {
    Term::AssociatedType {
        origin: Box::new(AssociatedTypeOrigin {
            interface: id(1),
            member: id(2),
        }),
        subject: Box::new(t),
        arguments: args,
    }
}
fn contract() -> Trait {
    Trait {
        parameter_types: Vec::new(),
        declaration: id(1),
        self_parameter: id(3),
        name: "Source".into(),
        type_parameters: vec![],
        parents: vec![],
        methods: vec![],
        requirements: vec![],
        capabilities: vec![],
        associated: vec![AssociatedContract {
            phase: Default::default(),
            declaration: id(2),
            name: "Element".into(),
            ty: ty(P::Type),
            default: Some(value(ty(P::Char))),
        }],
    }
}
fn implementation(subject: Term, result: Term) -> Implementation {
    Implementation {
        phases: Default::default(),
        associated_initializers: vec![],
        parameter_types: vec![],
        declaration: id(4),
        subject,
        interface: Some(id(1)),
        interface_arguments: vec![],
        type_parameters: vec![],
        capabilities: vec![],
        requirements: vec![],
        methods: vec![],
        associated: vec![(id(2), value(result))],
    }
}
fn query(
    traits: &[Trait],
    implementations: &[Implementation],
    term: &Term,
    budget: Budget,
) -> Result<elaboration::associated::AssociatedTypeResolution, String> {
    with_world(traits, implementations, |world| {
        world.resolve_associated_type(term, budget)
    })
}
fn normalize(
    traits: &[Trait],
    implementations: &[Implementation],
    term: &Term,
    budget: Budget,
) -> Result<elaboration::associated::AssociatedTypeNormalization, String> {
    with_world(traits, implementations, |world| {
        world.normalize_associated_types(term, budget)
    })
}
fn with_world<R>(
    traits: &[Trait],
    implementations: &[Implementation],
    run: impl FnOnce(&World<'_>) -> R,
) -> R {
    let mut index = Index::default();
    for i in implementations {
        index
            .insert(
                ImplementationHead {
                    parameter_types: i.parameter_types.clone(),
                    declaration: i.declaration,
                    interface: i.interface,
                    interface_arguments: i.interface_arguments.clone(),
                    subject: i.subject.clone(),
                    type_parameters: i.type_parameters.clone(),
                    requirements: i.requirements.clone(),
                },
                Budget::default(),
            )
            .unwrap();
    }
    let mut context = Context::default();
    context.admit_nominal_constructor(id(10));
    run(&World {
        index: &index,
        implementations,
        traits,
        assumptions: &[],
        context: &context,
    })
}

#[test]
fn implementation_equalities_use_replayable_nested_associated_views() {
    use elaboration::implementation::Selection;
    let traits = [contract()];
    let provider = implementation(ty(P::Bool), ty(P::Bool));
    let mut wrapper = implementation(holder(rigid(9)), projection(rigid(9), vec![]));
    wrapper.declaration = id(5);
    wrapper.type_parameters = vec![id(9)];
    wrapper.capabilities = vec![TraitRequirement {
        subject: rigid(9),
        interface: id(1),
        interface_arguments: vec![],
    }];
    wrapper.requirements = vec![TypePredicate::Equal(
        projection(rigid(9), vec![]),
        ty(P::Bool),
    )];
    let implementations = [provider.clone(), wrapper.clone()];
    let subject = holder(holder(ty(P::Bool)));
    with_world(&traits, &implementations, |world| {
        let resolution = world.resolve(id(1), &subject, Budget::default()).unwrap();
        assert_eq!(
            resolution.types.selection,
            Selection::Selected(id(5)),
            "{resolution}"
        );
        assert!(resolution.to_string().contains("(projection (source"));
        world
            .verify_condition_support(&resolution, Budget::default())
            .unwrap();
        assert_eq!(
            world
                .normalize_associated_types(&projection(subject.clone(), vec![]), Budget::default())
                .unwrap()
                .value,
            ty(P::Bool)
        );
        let mut changed = implementations.clone();
        changed[0].associated[0].1 = value(ty(P::Char));
        let stale = World {
            implementations: &changed,
            ..*world
        };
        assert!(
            stale
                .verify_condition_support(&resolution, Budget::default())
                .is_err()
        );
        let tiny = Budget {
            steps: 4,
            ..Budget::default()
        };
        assert!(
            world
                .resolve(id(1), &subject, tiny)
                .map_or(true, |result| !matches!(
                    result.types.selection,
                    Selection::Selected(_)
                ))
        );
    });
    let mut wrong_provider = provider.clone();
    wrong_provider.associated[0].1 = value(ty(P::Char));
    with_world(&traits, &[wrong_provider, wrapper.clone()], |world| {
        assert!(!matches!(
            world
                .resolve(id(1), &subject, Budget::default())
                .unwrap()
                .types
                .selection,
            Selection::Selected(_)
        ));
    });
    // The selected candidate must not manufacture its own prerequisite value.
    wrapper.requirements = vec![TypePredicate::Equal(
        projection(holder(rigid(9)), vec![]),
        ty(P::Bool),
    )];
    with_world(&traits, &[provider, wrapper], |world| {
        assert!(!matches!(
            world
                .resolve(id(1), &subject, Budget::default())
                .unwrap()
                .types
                .selection,
            Selection::Selected(_)
        ));
    });
}

#[test]
fn selected_override_and_evidence_are_returned_together() {
    let subject = holder(ty(P::Bool));
    let result = query(
        &[contract()],
        &[implementation(subject.clone(), ty(P::Bool))],
        &projection(subject, vec![]),
        Budget::default(),
    )
    .unwrap();
    assert_eq!(result.value, ty(P::Bool));
    assert_eq!(result.evidence.implementation, id(4));
    assert!(
        result
            .to_string()
            .starts_with("(associated-type-resolution (value (primitive bool))")
    );
}

#[test]
fn generic_subject_and_interface_arguments_share_selected_substitution() {
    let mut trait_ = contract();
    trait_.type_parameters = vec![id(7)];
    let mut impl_ = implementation(holder(rigid(8)), Term::Tuple(vec![rigid(8), rigid(9)]));
    impl_.type_parameters = vec![id(8), id(9)];
    impl_.interface_arguments = vec![rigid(9)];
    for (a, b) in [(P::Bool, P::Char), (P::Char, P::Bool)] {
        let result = query(
            &[trait_.clone()],
            &[impl_.clone()],
            &projection(holder(ty(a)), vec![ty(b)]),
            Budget::default(),
        )
        .unwrap();
        assert_eq!(result.value, Term::Tuple(vec![ty(a), ty(b)]));
        assert_eq!(result.evidence.type_arguments, vec![ty(a), ty(b)]);
    }
}

#[test]
fn absent_or_incomplete_selection_never_falls_back_to_trait_default() {
    let term = projection(holder(ty(P::Bool)), vec![]);
    assert!(
        query(&[contract()], &[], &term, Budget::default())
            .unwrap_err()
            .contains("unresolved")
    );
    let mut impl_ = implementation(holder(ty(P::Bool)), ty(P::Bool));
    impl_.associated.clear();
    assert!(
        query(&[contract()], &[impl_], &term, Budget::default())
            .unwrap_err()
            .contains("no associated Type value")
    );
}

#[test]
fn member_origin_arity_and_type_contract_are_validated() {
    let subject = holder(ty(P::Bool));
    let impl_ = implementation(subject.clone(), ty(P::Bool));
    let mut foreign = projection(subject.clone(), vec![]);
    let Term::AssociatedType { origin, .. } = &mut foreign else {
        panic!()
    };
    origin.member = id(99);
    assert!(
        query(&[contract()], &[impl_.clone()], &foreign, Budget::default())
            .unwrap_err()
            .contains("does not belong")
    );
    assert!(
        query(
            &[contract()],
            &[impl_.clone()],
            &projection(subject.clone(), vec![ty(P::Bool)]),
            Budget::default()
        )
        .unwrap_err()
        .contains("argument count")
    );
    let mut trait_ = contract();
    trait_.associated[0].ty = ty(P::Bool);
    assert!(
        query(
            &[trait_],
            &[impl_],
            &projection(subject, vec![]),
            Budget::default()
        )
        .unwrap_err()
        .contains("not Type-valued")
    );
}

#[test]
fn interface_requirements_and_implementation_capabilities_are_not_erased() {
    let subject = holder(ty(P::Bool));
    let term = projection(subject.clone(), vec![]);
    let mut trait_ = contract();
    trait_.requirements.push(TypePredicate::Bool(false));
    let mut impl_ = implementation(subject.clone(), ty(P::Bool));
    assert!(
        query(&[trait_], &[impl_.clone()], &term, Budget::default())
            .unwrap_err()
            .contains("requirement")
    );
    impl_.capabilities.push(TraitRequirement {
        subject,
        interface: id(99),
        interface_arguments: vec![],
    });
    assert!(
        query(&[contract()], &[impl_], &term, Budget::default())
            .unwrap_err()
            .contains("unresolved")
    );
}

#[test]
fn normal_form_heads_and_nested_projection_results_are_not_unfolded() {
    let subject = holder(ty(P::Bool));
    let term = projection(subject.clone(), vec![]);
    for result in [
        Term::NormalForm {
            head: id(50),
            sort: Sort::Type,
            arguments: vec![ty(P::Char)],
        },
        term.clone(),
    ] {
        let resolved = query(
            &[contract()],
            &[implementation(subject.clone(), result.clone())],
            &term,
            Budget::default(),
        )
        .unwrap();
        assert_eq!(resolved.value, result);
    }
    assert!(
        query(
            &[contract()],
            &[implementation(subject, ty(P::Bool))],
            &term,
            Budget {
                steps: 0,
                max_depth: 128
            }
        )
        .is_err()
    );
}

#[test]
fn malformed_selected_values_do_not_pass_as_type_templates() {
    let subject = holder(ty(P::Bool));
    let term = projection(subject.clone(), vec![]);
    let mut impl_ = implementation(subject, ty(P::Bool));
    impl_.associated.push(impl_.associated[0].clone());
    assert!(
        query(&[contract()], &[impl_.clone()], &term, Budget::default())
            .unwrap_err()
            .contains("duplicate")
    );
    impl_.associated.pop();
    impl_.associated[0].1.kind = ExprKind::Unit;
    assert!(
        query(&[contract()], &[impl_], &term, Budget::default())
            .unwrap_err()
            .contains("not a completed Type template")
    );
}

#[test]
fn interface_capability_formation_is_checked_before_returning_a_value() {
    let subject = holder(ty(P::Bool));
    let term = projection(subject.clone(), vec![]);
    let mut trait_ = contract();
    trait_.capabilities.push(TraitRequirement {
        subject: rigid(3),
        interface: id(99),
        interface_arguments: vec![],
    });
    let impl_ = implementation(subject.clone(), ty(P::Bool));
    assert!(
        query(
            &[trait_.clone()],
            &[impl_.clone()],
            &term,
            Budget::default()
        )
        .unwrap_err()
        .contains("capability")
    );
    let mut marker = contract();
    marker.declaration = id(99);
    marker.associated.clear();
    let mut marker_impl = implementation(subject, ty(P::Bool));
    marker_impl.declaration = id(5);
    marker_impl.interface = Some(id(99));
    marker_impl.associated.clear();
    assert_eq!(
        query(
            &[trait_, marker],
            &[impl_, marker_impl],
            &term,
            Budget::default()
        )
        .unwrap()
        .value,
        ty(P::Bool)
    );
}

#[test]
fn deeply_nested_interface_requirements_fail_before_recursive_substitution() {
    let subject = holder(ty(P::Bool));
    let mut requirement = TypePredicate::Bool(true);
    for _ in 0..100 {
        requirement = TypePredicate::Not(Box::new(requirement));
    }
    let mut trait_ = contract();
    trait_.requirements.push(requirement);
    assert!(
        query(
            &[trait_],
            &[implementation(subject.clone(), ty(P::Bool))],
            &projection(subject, vec![]),
            Budget::default()
        )
        .unwrap_err()
        .contains("depth exhausted")
    );
}

#[test]
fn recursive_normalization_resolves_inputs_before_selection_and_shares_results() {
    let first = holder(ty(P::Bool));
    let second = holder(ty(P::Char));
    let inner = projection(first.clone(), vec![]);
    let outer = projection(inner.clone(), vec![]);
    let a = implementation(first, second.clone());
    let mut b = implementation(second, ty(P::Char));
    b.declaration = id(5);
    let input = Term::Tuple(vec![outer.clone(), outer, inner]);
    let result = normalize(&[contract()], &[a, b], &input, Budget::default()).unwrap();
    assert_eq!(
        result.value,
        Term::Tuple(vec![ty(P::Char), ty(P::Char), holder(ty(P::Char))])
    );
    assert_eq!(result.steps.len(), 2);
    assert_eq!(result.steps[0].resolution.evidence.implementation, id(4));
    assert_eq!(result.steps[1].resolution.evidence.implementation, id(5));
    assert!(
        result
            .to_string()
            .starts_with("(associated-type-normalization (value (tuple")
    );
}

#[test]
fn recursive_normalization_rejects_cycles_without_poisoning_later_queries() {
    let first = holder(ty(P::Bool));
    let second = holder(ty(P::Char));
    let root = projection(first.clone(), vec![]);
    let a = implementation(first, projection(second.clone(), vec![]));
    let mut b = implementation(second, root.clone());
    b.declaration = id(5);
    let mut safe = implementation(holder(ty(P::U8)), ty(P::Bool));
    safe.declaration = id(6);
    with_world(&[contract()], &[a, b, safe], |world| {
        assert!(
            world
                .normalize_associated_types(&root, Budget::default())
                .unwrap_err()
                .contains("cyclic")
        );
        let good = world
            .normalize_associated_types(&projection(holder(ty(P::U8)), vec![]), Budget::default())
            .unwrap();
        assert_eq!(good.value, ty(P::Bool));
        assert_eq!(good.steps.len(), 1);
    });
}

#[test]
fn normalization_walks_callable_record_and_opaque_input_structure() {
    let subject = holder(ty(P::Bool));
    let query = projection(subject.clone(), vec![]);
    let wrap = |item: Term| Term::Closure {
        environment: vec![Term::Record(vec![(
            "field\nquoted\"".into(),
            Term::Pointer(Box::new(item.clone())),
        )])],
        signature: Box::new(Term::ComptimeFunction(Box::new(Term::Function {
            parameters: vec![nir::Parameter {
                sort: Sort::Runtime,
                ty: item.clone(),
            }],
            result: Box::new(Term::NormalForm {
                head: id(88),
                sort: Sort::Type,
                arguments: vec![Term::TupleValue(vec![item])],
            }),
        }))),
    };
    let result = normalize(
        &[contract()],
        &[implementation(subject, ty(P::Char))],
        &wrap(query),
        Budget::default(),
    )
    .unwrap();
    assert_eq!(result.value, wrap(ty(P::Char)));
    assert_eq!(result.steps.len(), 1);
    assert!(result.to_string().contains("field\\nquoted\\\""));
}

#[test]
fn normalization_keeps_binder_scope_and_waits_for_dependent_projection_inputs() {
    let subject = holder(ty(P::Bool));
    let bound = Term::Bound {
        depth: 0,
        index: 0,
        sort: Sort::Type,
    };
    let signature = |result| Term::Function {
        parameters: vec![nir::Parameter {
            sort: Sort::Type,
            ty: ty(P::Type),
        }],
        result: Box::new(result),
    };
    let input = signature(Term::Tuple(vec![
        bound.clone(),
        projection(subject.clone(), vec![]),
    ]));
    let impl_ = implementation(subject, ty(P::Char));
    assert_eq!(
        normalize(&[contract()], &[impl_.clone()], &input, Budget::default())
            .unwrap()
            .value,
        signature(Term::Tuple(vec![bound.clone(), ty(P::Char)]))
    );
    let dependent = signature(projection(bound, vec![]));
    assert!(normalize(&[contract()], &[impl_], &dependent, Budget::default()).is_err());
}

#[test]
fn normalization_enforces_depth_budget_and_caches_per_world_only() {
    let subject = holder(ty(P::Bool));
    let root = projection(subject.clone(), vec![]);
    for expected in [P::Bool, P::Char] {
        let result = normalize(
            &[contract()],
            &[implementation(subject.clone(), ty(expected))],
            &root,
            Budget::default(),
        )
        .unwrap();
        assert_eq!(result.value, ty(expected));
    }
    assert!(
        normalize(
            &[contract()],
            &[implementation(subject.clone(), ty(P::Bool))],
            &root,
            Budget {
                steps: 0,
                max_depth: 128
            }
        )
        .is_err()
    );
    assert!(
        normalize(
            &[contract()],
            &[implementation(subject, ty(P::Bool))],
            &Term::Pointer(Box::new(root)),
            Budget {
                steps: 100_000,
                max_depth: 1
            }
        )
        .unwrap_err()
        .contains("depth")
    );
}

#[test]
fn normalization_cache_hits_charge_the_expanded_type_payload() {
    let subject = holder(ty(P::Bool));
    let root = projection(subject.clone(), vec![]);
    let large = Term::Record(vec![("x".repeat(4096), ty(P::Bool))]);
    let impl_ = implementation(subject, large);
    assert!(normalize(&[contract()], &[impl_.clone()], &root, Budget::default()).is_ok());
    // The query key is tiny, but each use must return an owned record label.
    let repeated = Term::Tuple(vec![root; 100]);
    assert!(
        normalize(&[contract()], &[impl_], &repeated, Budget::default())
            .unwrap_err()
            .contains("budget")
    );
}

#[test]
fn capability_projection_qualification_shares_cycles_and_recovers_after_failure() {
    use elaboration::implementation::Selection;
    let source = implementation(ty(P::Bool), ty(P::Bool));
    let mut target = source.clone();
    target.declaration = id(100);
    target.interface = Some(id(101));
    target.associated.clear();
    target.capabilities = vec![TraitRequirement {
        interface: id(102),
        subject: projection(ty(P::Bool), vec![]),
        interface_arguments: vec![],
    }];
    let mut allowed = source.clone();
    allowed.declaration = id(103);
    allowed.interface = Some(id(102));
    allowed.associated.clear();
    let traits = [contract()];
    let mut cyclic = source.clone();
    cyclic.capabilities = vec![TraitRequirement {
        interface: id(101),
        subject: ty(P::Bool),
        interface_arguments: vec![],
    }];
    let mut excluded = target.clone();
    excluded.capabilities.push(TraitRequirement {
        interface: id(104),
        subject: ty(P::Bool),
        interface_arguments: vec![],
    });
    with_world(
        &traits,
        &[cyclic.clone(), excluded, allowed.clone()],
        |world| {
            let result = world
                .resolve(id(101), &ty(P::Bool), Budget::default())
                .unwrap();
            assert_eq!(result.types.selection, Selection::NoApplicableCandidate);
        },
    );
    with_world(
        &traits,
        &[cyclic, target.clone(), allowed.clone()],
        |world| {
            let result = world
                .resolve(id(101), &ty(P::Bool), Budget::default())
                .unwrap();
            assert!(!matches!(result.types.selection, Selection::Selected(_)));
            assert!(result.qualifications.values().any(|q| matches!(q,
            elaboration::capabilities::Qualification::Pending(reason) if reason.contains("recursive") || reason.contains("pending"))));
        },
    );
    with_world(&traits, &[source, target, allowed], |world| {
        let limited = world.resolve(
            id(101),
            &ty(P::Bool),
            Budget {
                steps: 1,
                ..Budget::default()
            },
        );
        assert!(
            limited.is_err() || !matches!(limited.unwrap().types.selection, Selection::Selected(_))
        );
        let result = world
            .resolve(id(101), &ty(P::Bool), Budget::default())
            .unwrap();
        assert_eq!(result.types.selection, Selection::Selected(id(100)));
    });
}

#[test]
fn type_list_inputs_resolve_associated_elements_without_changing_the_head() {
    let subject = holder(ty(P::Bool));
    let wrap = |item| Term::NormalForm {
        head: id(88),
        sort: Sort::Type,
        arguments: vec![Term::TypeListValue(vec![item])],
    };
    let result = normalize(
        &[contract()],
        &[implementation(subject.clone(), ty(P::Char))],
        &wrap(projection(subject, vec![])),
        Budget::default(),
    )
    .unwrap();
    assert_eq!(result.value, wrap(ty(P::Char)));
    assert_eq!(result.steps.len(), 1);
}

#[test]
fn associated_success_resolution_rebuilds_frozen_error_without_unfolding_head() {
    let subject = holder(ty(P::Bool));
    let inner_type = Term::error_result(vec![ty(P::Char)], ty(P::Bool));
    let input = Term::NormalForm {
        head: id(30),
        sort: Sort::Type,
        arguments: vec![Term::ErrorValue {
            ty: Box::new(Term::error_result(
                vec![ty(P::Bool)],
                projection(subject.clone(), vec![]),
            )),
            label: None,
            value: Box::new(Term::ErrorValue {
                ty: Box::new(inner_type.clone()),
                label: Some(Box::new(ty(P::Char))),
                value: Box::new(Term::Char('x')),
            }),
        }],
    };
    let expected = Term::NormalForm {
        head: id(30),
        sort: Sort::Type,
        arguments: vec![Term::ErrorValue {
            ty: Box::new(Term::error_result(
                vec![ty(P::Bool), ty(P::Char)],
                ty(P::Bool),
            )),
            label: Some(Box::new(ty(P::Char))),
            value: Box::new(Term::Char('x')),
        }],
    };
    assert_eq!(
        normalize(
            &[contract()],
            &[implementation(subject.clone(), inner_type)],
            &input,
            Budget::default()
        )
        .unwrap()
        .value,
        expected
    );
    assert!(
        normalize(
            &[contract()],
            &[implementation(subject, ty(P::Bool))],
            &input,
            Budget::default()
        )
        .is_err()
    );
}

#[test]
fn associated_type_expansion_is_bounded_and_a_later_query_can_recover() {
    let argument = Term::Tuple(vec![ty(P::Bool); 16]);
    let mut template = implementation(holder(rigid(8)), Term::Tuple(vec![rigid(8); 256]));
    template.type_parameters = vec![id(8)];
    let root = projection(holder(argument.clone()), vec![]);
    with_world(&[contract()], &[template], |world| {
        let failure = world
            .resolve_associated_type(
                &root,
                Budget {
                    steps: 4000,
                    ..Budget::default()
                },
            )
            .unwrap_err();
        assert!(failure.contains("budget"), "{failure}");
        let recovered = world
            .resolve_associated_type(
                &root,
                Budget {
                    steps: 1_000_000,
                    ..Budget::default()
                },
            )
            .unwrap();
        assert_eq!(recovered.value, Term::Tuple(vec![argument.clone(); 256]));
        assert_eq!(recovered.evidence.type_arguments, vec![argument]);
    });
}

#[test]
fn distinct_projection_sequences_share_work_without_discarding_unused_allowances() {
    let mut selected = implementation(holder(rigid(5)), rigid(5));
    selected.type_parameters = vec![id(5)];
    selected.parameter_types = vec![ty(P::Type)];
    let mut inputs = Vec::new();
    let mut output = Vec::new();
    let mut element = ty(P::Bool);
    for _ in 0..16 {
        inputs.push(projection(holder(element.clone()), vec![]));
        output.push(element.clone());
        element = Term::Pointer(Box::new(element));
    }
    let input = Term::Tuple(inputs);
    let expected = Term::Tuple(output);
    let traits = [contract()];
    let implementations = [selected];
    assert_eq!(
        normalize(
            &traits,
            &implementations,
            &input,
            Budget {
                steps: 100_000,
                ..Budget::default()
            }
        )
        .unwrap()
        .value,
        expected
    );
    assert!(
        normalize(
            &traits,
            &implementations,
            &input,
            Budget {
                steps: 10,
                ..Budget::default()
            }
        )
        .is_err()
    );
    assert_eq!(
        normalize(
            &traits,
            &implementations,
            &input,
            Budget {
                steps: 100_000,
                ..Budget::default()
            }
        )
        .unwrap()
        .value,
        expected
    );
}
