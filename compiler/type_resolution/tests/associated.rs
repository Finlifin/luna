use nir::program::{
    AssociatedContract, Expr, ExprKind, Implementation, Trait, TraitRequirement, TypePredicate,
};
use nir::{AssociatedTypeOrigin, DeclarationId as Id, Primitive as P, Sort, Term};
use type_resolution::{
    capabilities::World,
    implementation::{ImplementationHead, Index},
    ptc::{Budget, Context},
};

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
        comptime_only: false,
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

#[test]
fn declaration_selection_preserves_query_fuel_and_failure_categories() {
    use nir::value_admission::Error;
    use type_resolution::{
        dispatch::DeclarationWorld,
        implementation::{Selection, TypeAssessment},
    };
    let package = nir::program::Package {
        traits: vec![contract()],
        implementations: vec![implementation(ty(P::Bool), ty(P::U8))],
        ..Default::default()
    };
    let (declarations, _) = DeclarationWorld::new(&package, Budget::default(), None).unwrap();
    let world = World {
        index: declarations.index(),
        implementations: &package.implementations,
        traits: &package.traits,
        assumptions: declarations.assumptions(),
        context: declarations.context(),
    };
    let interface = nir::program::InterfaceInstance {
        declaration: id(1),
        arguments: vec![],
    };
    let mut successes = 0;
    let mut exhausted = 0;
    for subject in [ty(P::Bool), ty(P::Char)] {
        for steps in [0, 1, 8, 32, 128, 1024, 100_000] {
            let mut expected_budget = Budget {
                steps,
                ..Default::default()
            };
            let expected =
                world.resolve_instance_metered(&interface, &subject, &mut expected_budget);
            let mut actual_budget = Budget {
                steps,
                ..Default::default()
            };
            let actual = declarations.select_interface(&interface, &subject, &mut actual_budget);
            assert_eq!(actual_budget.steps, expected_budget.steps);
            assert!(actual_budget.steps <= steps);
            match expected {
                Ok(resolution) => match resolution.types.selection {
                    Selection::Selected(identity) => {
                        successes += 1;
                        let Some(TypeAssessment::Applicable(evidence)) =
                            resolution.types.assessments.get(&identity)
                        else {
                            panic!("selected evidence");
                        };
                        assert_eq!(actual.unwrap(), (identity, evidence.type_arguments.clone()));
                    }
                    _ => assert!(matches!(actual, Err(Error::Unresolved(_)))),
                },
                Err(Error::Budget) => {
                    exhausted += 1;
                    assert!(matches!(actual, Err(Error::Budget)));
                }
                Err(error) => assert_eq!(actual.unwrap_err().to_string(), error.to_string()),
            }
        }
    }
    assert!(successes > 0 && exhausted > 0);
    let mut budget = Budget::default();
    let first = declarations
        .select_interface(&interface, &ty(P::Bool), &mut budget)
        .unwrap();
    let after_first = budget.steps;
    assert_eq!(
        declarations
            .select_interface(&interface, &ty(P::Bool), &mut budget)
            .unwrap(),
        first
    );
    assert!(
        budget.steps < after_first,
        "a new query still pays for its own selection"
    );
}
fn query(
    traits: &[Trait],
    implementations: &[Implementation],
    term: &Term,
    budget: Budget,
) -> Result<type_resolution::associated::AssociatedTypeResolution, nir::NominalAdmissionError> {
    with_world(traits, implementations, |world| {
        world.resolve_associated_type(term, budget)
    })
}
fn normalize(
    traits: &[Trait],
    implementations: &[Implementation],
    term: &Term,
    budget: Budget,
) -> Result<type_resolution::associated::AssociatedTypeNormalization, nir::NominalAdmissionError> {
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

fn frozen_types(fields: Vec<nir::Term>) -> nir::Term {
    nir::Term::NominalValue {
        ty: Box::new(nir::Term::NormalForm {
            head: nir::DeclarationId {
                package: 901,
                local: 1,
            },
            sort: nir::Sort::Type,
            arguments: vec![nir::Term::Primitive(nir::Primitive::Type)],
        }),
        variant: None,
        fields,
    }
}

#[test]
fn implementation_equalities_use_replayable_nested_associated_views() {
    use type_resolution::implementation::Selection;
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
            .to_string()
            .contains("unresolved")
    );
    let mut impl_ = implementation(holder(ty(P::Bool)), ty(P::Bool));
    impl_.associated.clear();
    assert!(
        query(&[contract()], &[impl_], &term, Budget::default())
            .unwrap_err()
            .to_string()
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
            .to_string()
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
        .to_string()
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
        .to_string()
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
            .to_string()
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
            .to_string()
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
            .to_string()
            .contains("duplicate")
    );
    impl_.associated.pop();
    impl_.associated[0].1.kind = ExprKind::Unit;
    assert!(
        query(&[contract()], &[impl_], &term, Budget::default())
            .unwrap_err()
            .to_string()
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
        .to_string()
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
        .to_string()
        .contains("budget exhausted")
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
                .to_string()
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
        .to_string()
        .contains("budget")
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
            .to_string()
            .contains("budget")
    );
}

#[test]
fn capability_projection_qualification_shares_cycles_and_recovers_after_failure() {
    use type_resolution::implementation::Selection;
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
            type_resolution::capabilities::Qualification::Pending(reason) if matches!(reason, nir::NominalAdmissionError::Unresolved(message) if message.contains("recursive") || message.contains("pending")))));
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
        arguments: vec![frozen_types(vec![item])],
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
        assert!(
            matches!(failure, nir::NominalAdmissionError::Budget),
            "{failure}"
        );
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

#[test]
fn declaration_world_queries_keep_caches_private_and_failure_fuel_spent() {
    use type_resolution::dispatch::{DeclarationWorld, normalize_concrete_type};
    let package = |result| nir::program::Package {
        traits: vec![contract()],
        implementations: vec![implementation(ty(P::Bool), result)],
        ..Default::default()
    };
    let first = package(ty(P::U8));
    let second = package(ty(P::Char));
    let query = projection(ty(P::Bool), vec![]);
    let budget = Budget::default();
    let (mut a, mut fuel_a) = DeclarationWorld::new(&first, budget, None).unwrap();
    let (mut b, mut fuel_b) = DeclarationWorld::new(&second, budget, None).unwrap();
    assert_eq!(
        a.normalized_type(&query, budget, &mut fuel_a).unwrap(),
        ty(P::U8)
    );
    let before = fuel_a;
    assert_eq!(
        a.normalized_type(&query, budget, &mut fuel_a).unwrap(),
        ty(P::U8)
    );
    assert!(
        fuel_a < before,
        "cached copies still consume their work budget"
    );
    for (invalid, expected_invalid) in [
        (Term::Bool(true), true),
        (projection(ty(P::I32), vec![]), false),
    ] {
        let mut fuel = budget.steps;
        let failure = a.normalized_type(&invalid, budget, &mut fuel).unwrap_err();
        assert!(matches!(failure, nir::NominalAdmissionError::Invalid(_)) == expected_invalid);
        if !expected_invalid {
            assert!(matches!(failure, nir::NominalAdmissionError::Unresolved(_)));
        }
        assert!(fuel > 0 && fuel < budget.steps);
        assert_eq!(
            a.normalized_type(&query, budget, &mut fuel).unwrap(),
            ty(P::U8)
        );
    }
    assert_eq!(
        b.normalized_type(&query, budget, &mut fuel_b).unwrap(),
        ty(P::Char)
    );
    let mut exhausted = 1;
    assert!(normalize_concrete_type(&first, &query, &mut exhausted).is_err());
    assert_eq!(exhausted, 0);
    let mut fresh = budget.steps;
    assert_eq!(
        normalize_concrete_type(&first, &query, &mut fresh).unwrap(),
        ty(P::U8)
    );
    let mut unknown = budget.steps;
    let error =
        normalize_concrete_type(&first, &projection(ty(P::I32), vec![]), &mut unknown).unwrap_err();
    assert!(error.to_string().contains("remaining-steps"), "{error}");
    assert!(
        unknown > 0 && unknown < budget.steps,
        "failed selection returns only unused fuel"
    );
    assert_eq!(
        normalize_concrete_type(&first, &query, &mut unknown).unwrap(),
        ty(P::U8)
    );
}

#[test]
fn prepared_methods_preserve_default_binders_and_pending_contracts() {
    use nir::program::{DispatchInstance, MethodContract};
    use type_resolution::dispatch::DeclarationWorld;
    let make = |use_default| {
        let mut interface = contract();
        interface.methods.push(MethodContract {
            phase: Default::default(),
            is_quote: false,
            pure: false,
            is_unsafe: false,
            declaration: id(20),
            name: "convert".into(),
            type_parameters: vec![id(6)],
            type_arguments: vec![],
            signature: Term::Function {
                parameters: vec![nir::Parameter {
                    sort: Sort::Runtime,
                    ty: rigid(3),
                }],
                result: Box::new(rigid(6)),
            }
            .into(),
            // Preparation retains this obligation; it does not prove it.
            requirements: vec![TypePredicate::Equal(rigid(6), ty(P::Char))],
            capabilities: vec![],
            program: Default::default(),
            parameter_bindings: vec![],
            by_value_receiver: false,
            by_reference_receiver: false,
            default: Some(id(30)),
        });
        let mut selected = implementation(ty(P::Bool), ty(P::U8));
        selected.methods = vec![("convert".into(), if use_default { id(30) } else { id(31) })];
        nir::program::Package {
            traits: vec![interface],
            implementations: vec![selected],
            ..Default::default()
        }
    };
    for use_default in [false, true] {
        let package = make(use_default);
        let budget = Budget::default();
        let (world, mut fuel) = DeclarationWorld::new(&package, budget, None).unwrap();
        let interface = nir::program::InterfaceInstance {
            declaration: id(1),
            arguments: vec![],
        };
        let (selected, arguments) = world
            .select_interface(&interface, &ty(P::Bool), &mut { budget })
            .unwrap();
        let instance = DispatchInstance {
            interface: id(1),
            interface_arguments: vec![],
            subject: ty(P::Bool),
            implementation: selected,
            arguments,
            methods: vec![],
            associated: vec![],
            associated_initializers: vec![],
        };
        assert!(
            world
                .prepare_method(&instance, id(20), &[], false, &mut fuel)
                .unwrap_err()
                .to_string()
                .contains("count mismatch")
        );
        assert!(
            world
                .prepare_method(&instance, id(99), &[ty(P::U8)], false, &mut fuel)
                .unwrap_err()
                .to_string()
                .contains("unknown dispatch member")
        );
        let (method, specialized) = world
            .prepare_method(&instance, id(20), &[ty(P::U8)], false, &mut fuel)
            .unwrap();
        assert_eq!(method.body, if use_default { id(30) } else { id(31) });
        assert_eq!(
            method.arguments,
            if use_default {
                vec![ty(P::Bool), ty(P::U8)]
            } else {
                vec![ty(P::U8)]
            }
        );
        assert_eq!(
            specialized.signature.ordinary().unwrap(),
            &Term::Function {
                parameters: vec![nir::Parameter {
                    sort: Sort::Runtime,
                    ty: ty(P::Bool)
                }],
                result: Box::new(ty(P::U8)),
            }
        );
        assert_eq!(
            specialized.requirements,
            vec![TypePredicate::Equal(ty(P::U8), ty(P::Char))]
        );
        assert!(specialized.type_parameters.is_empty());
        assert_eq!(specialized.type_arguments, vec![ty(P::U8)]);
        assert_eq!(specialized.default, None);
        assert!(
            world
                .prepare_method(&instance, id(20), &[ty(P::U8)], false, &mut 0)
                .is_err()
        );
        assert!(
            world
                .prepare_method(&instance, id(20), &[ty(P::U8)], false, &mut 1000)
                .is_ok()
        );
    }
}

#[test]
fn associated_queries_complete_dependent_type_value_domains() {
    let index = Term::Rigid {
        declaration: id(9),
        sort: Sort::ComptimeValue,
    };
    let mut interface = contract();
    interface.type_parameters = vec![id(8), id(9)];
    interface.parameter_types = vec![ty(P::Type), rigid(8)];
    let mut selected = implementation(ty(P::Bool), holder(index.clone()));
    selected.type_parameters = interface.type_parameters.clone();
    selected.parameter_types = interface.parameter_types.clone();
    selected.interface_arguments = vec![rigid(8), index];
    let inputs = vec![ty(P::Type), ty(P::Bool)];
    let root = projection(ty(P::Bool), inputs.clone());
    with_world(&[interface], &[selected], |world| {
        assert!(
            world
                .resolve_associated_type(
                    &root,
                    Budget {
                        steps: 0,
                        ..Budget::default()
                    }
                )
                .is_err()
        );
        let resolved = world
            .resolve_associated_type(&root, Budget::default())
            .unwrap();
        assert_eq!(resolved.value, holder(ty(P::Bool)));
        assert_eq!(resolved.evidence.type_arguments, inputs);
        let value = world
            .resolve_associated_value_plan_metered(
                &nir::program::InterfaceInstance {
                    declaration: id(1),
                    arguments: inputs,
                },
                id(2),
                &ty(P::Bool),
                &mut Budget::default(),
            )
            .unwrap();
        assert!(
            matches!(value.value.kind, ExprKind::TypeValue(ref result) if *result == holder(ty(P::Bool)))
        );
        let invalid = projection(ty(P::Bool), vec![ty(P::Bool), ty(P::Bool)]);
        assert!(
            world
                .resolve_associated_type(&invalid, Budget::default())
                .is_err()
        );
        assert_eq!(
            world
                .resolve_associated_type(&root, Budget::default())
                .unwrap()
                .value,
            resolved.value
        );
    });
}

#[test]
fn completed_associated_tuples_follow_current_declaration_layout_evidence() {
    use nir::normal_forms::{Head, Kind, Parameter, Role};
    use nir::program::{InterfaceInstance, Literal, LocalId};
    let list = Term::NormalForm {
        head: id(50),
        sort: Sort::Type,
        arguments: vec![ty(P::Type)],
    };
    let head = Head {
        declaration: id(51),
        role: Role::Tuple,
        collector: None,
        requirements: vec![],
        parameters: vec![Parameter {
            declaration: id(52),
            binding: LocalId {
                owner: 51,
                index: 0,
            },
            kind: Kind::Value,
            ty: list.clone(),
        }],
    };
    let tuple = Term::NormalForm {
        head: id(51),
        sort: Sort::Type,
        arguments: vec![Term::NominalValue {
            ty: Box::new(list.clone()),
            variant: None,
            fields: vec![ty(P::Bool)],
        }],
    };
    let mut interface = contract();
    interface.associated[0].ty = tuple.clone();
    interface.associated[0].default = None;
    let mut provider = implementation(ty(P::Bool), ty(P::Bool));
    let mut payload = value(ty(P::Bool));
    payload.ty = tuple;
    let mut flag = value(ty(P::Bool));
    flag.ty = ty(P::Bool);
    flag.kind = ExprKind::Literal(Literal::Bool(true));
    payload.kind = ExprKind::Tuple(vec![flag]);
    provider.associated[0].1 = payload.clone();
    let traits = [interface];
    let implementations = [provider];
    let mut index = Index::default();
    index
        .insert(
            ImplementationHead {
                parameter_types: vec![],
                declaration: id(4),
                interface: Some(id(1)),
                interface_arguments: vec![],
                subject: ty(P::Bool),
                type_parameters: vec![],
                requirements: vec![],
            },
            Budget::default(),
        )
        .unwrap();
    let mut context = Context::default();
    context.admit_tuple_head(id(51));
    context.admit_list_inference_head(id(50));
    let resolve = |index: &Index| {
        World {
            index,
            implementations: &implementations,
            traits: &traits,
            assumptions: &[],
            context: &context,
        }
        .resolve_associated_value_plan_metered(
            &InterfaceInstance {
                declaration: id(1),
                arguments: vec![],
            },
            id(2),
            &ty(P::Bool),
            &mut Budget::default(),
        )
    };
    assert!(resolve(&index).is_err());
    index.set_normal_form_metadata(vec![head.clone()]);
    assert_eq!(resolve(&index).unwrap().value, payload);
    let revision = index.revision();
    index.set_normal_form_metadata(vec![head.clone()]);
    assert_eq!(index.revision(), revision);
    for invalid in [
        Head {
            role: Role::Opaque,
            ..head.clone()
        },
        Head {
            parameters: vec![Parameter {
                ty: Term::NormalForm {
                    head: id(99),
                    sort: Sort::Type,
                    arguments: vec![ty(P::Type)],
                },
                ..head.parameters[0].clone()
            }],
            ..head.clone()
        },
    ] {
        index.set_normal_form_metadata(vec![invalid]);
        assert!(index.revision() > revision);
        assert!(resolve(&index).is_err());
        index.set_normal_form_metadata(vec![head.clone()]);
        assert_eq!(resolve(&index).unwrap().value, payload);
    }
    index.set_normal_form_metadata(vec![head.clone(), head.clone()]);
    assert!(resolve(&index).is_err());
    index.set_normal_form_metadata(vec![head]);
    assert_eq!(resolve(&index).unwrap().value, payload);
}

#[test]
fn declaration_world_failed_retention_does_not_publish_completed_children() {
    use std::collections::HashMap;
    use type_resolution::{associated::charge_projection_copy, dispatch::DeclarationWorld};
    let result = Term::Record(
        (0..96)
            .map(|i| (format!("field{i:03}"), ty(P::U8)))
            .collect(),
    );
    let package = nir::program::Package {
        traits: vec![contract()],
        implementations: vec![implementation(ty(P::Bool), result.clone())],
        ..Default::default()
    };
    let query = Term::Pointer(Box::new(projection(ty(P::Bool), vec![])));
    let expected = Term::Pointer(Box::new(result));
    let make = || {
        DeclarationWorld::new(&package, Budget::default(), None)
            .unwrap()
            .0
    };
    let mut fresh = make();
    let mut full_fuel = 100_000;
    assert_eq!(
        fresh
            .normalized_type(&query, Budget::default(), &mut full_fuel)
            .unwrap(),
        expected
    );
    let full_work = 100_000 - full_fuel;
    let mut copy_fuel = 100_000;
    charge_projection_copy(&expected, &mut copy_fuel).unwrap();
    let copy_work = 100_000 - copy_fuel;
    let probe = make();
    let world = World {
        index: probe.index(),
        implementations: &package.implementations,
        traits: &package.traits,
        assumptions: probe.assumptions(),
        context: probe.context(),
    };

    // Find a real boundary where normalization finishes, but retaining the
    // root cannot. Do not assume observed PTC work is a minimum success budget.
    let (allowance, remaining) = (full_work.saturating_sub(copy_work)..full_work)
        .rev()
        .find_map(|steps| {
            let mut fuel = steps;
            charge_projection_copy(&query, &mut fuel).ok()?;
            let (normalized, mut remaining) = world
                .normalize_associated_types_cached(
                    &query,
                    Budget {
                        steps: fuel,
                        ..Budget::default()
                    },
                    &mut HashMap::new(),
                )
                .ok()?;
            assert_eq!(normalized.value, expected);
            charge_projection_copy(&normalized.value, &mut remaining)
                .is_err()
                .then_some((steps, remaining))
        })
        .expect("the final root retention has a nonzero resource boundary");
    let mut queried = make();
    let mut fuel = allowance;
    assert!(matches!(
        queried.normalized_type(&query, Budget::default(), &mut fuel),
        Err(nir::NominalAdmissionError::Budget)
    ));
    assert_eq!(
        fuel, remaining,
        "failure returns actual normalization and copy consumption"
    );
    let mut retry_fuel = 100_000;
    assert_eq!(
        queried
            .normalized_type(&query, Budget::default(), &mut retry_fuel)
            .unwrap(),
        expected
    );
    assert_eq!(
        retry_fuel, full_fuel,
        "failed root retention must not warm a child cache"
    );
    let mut warm_fuel = 100_000;
    queried
        .normalized_type(&query, Budget::default(), &mut warm_fuel)
        .unwrap();
    assert!(warm_fuel > retry_fuel && warm_fuel < 100_000);
}

#[test]
fn cached_type_views_charge_failed_queries_without_publishing_children() {
    use nir::NominalAdmissionError as Failure;
    use std::collections::HashMap;
    let subject = holder(ty(P::Bool));
    let valid = projection(subject.clone(), vec![]);
    let mut invalid = valid.clone();
    if let Term::AssociatedType { origin, .. } = &mut invalid {
        origin.member = id(99);
    }
    let unresolved = projection(holder(ty(P::Char)), vec![]);
    with_world(
        &[contract()],
        &[implementation(subject, ty(P::U8))],
        |world| {
            let mut fresh_budget = Budget::default();
            let mut fresh_cache = HashMap::new();
            assert_eq!(
                world
                    .associated_type_view_cached_metered(
                        &valid,
                        &mut fresh_budget,
                        &mut fresh_cache
                    )
                    .unwrap()
                    .value,
                ty(P::U8)
            );
            assert!(fresh_budget.steps < Budget::default().steps);
            assert!(!fresh_cache.is_empty());
            for (tail, expect_invalid) in [(invalid, true), (unresolved, false)] {
                let root = Term::Record(vec![
                    ("first".into(), valid.clone()),
                    ("later".into(), tail),
                ]);
                let mut cache = HashMap::new();
                let mut budget = Budget::default();
                let error = world
                    .associated_type_view_cached_metered(&root, &mut budget, &mut cache)
                    .unwrap_err();
                assert!(
                    if expect_invalid {
                        matches!(error, Failure::Invalid(_))
                    } else {
                        matches!(error, Failure::Unresolved(_))
                    },
                    "{error:?}"
                );
                assert!(budget.steps > 0 && budget.steps < Budget::default().steps);
                assert!(
                    cache.is_empty(),
                    "completed first child must remain private after failure"
                );
                let mut retry = Budget::default();
                assert_eq!(
                    world
                        .associated_type_view_cached_metered(&valid, &mut retry, &mut cache)
                        .unwrap()
                        .value,
                    ty(P::U8)
                );
                assert_eq!(
                    retry.steps, fresh_budget.steps,
                    "failed root did not warm the cache"
                );

                let original = cache.clone();
                world
                    .associated_type_view_cached_metered(&root, &mut Budget::default(), &mut cache)
                    .unwrap_err();
                assert_eq!(
                    cache, original,
                    "a failed query preserves earlier committed views"
                );
            }
            let root = Term::Record(vec![
                ("first".into(), valid.clone()),
                ("again".into(), valid),
            ]);
            let mut succeeded = false;
            let mut exhausted = false;
            for steps in (0..4000).step_by(11) {
                let mut budget = Budget {
                    steps,
                    ..Budget::default()
                };
                let mut cache = HashMap::new();
                match world.associated_type_view_cached_metered(&root, &mut budget, &mut cache) {
                    Ok(view) => {
                        succeeded = true;
                        assert_eq!(
                            view.value,
                            Term::Record(vec![
                                ("first".into(), ty(P::U8)),
                                ("again".into(), ty(P::U8))
                            ])
                        );
                    }
                    Err(Failure::Budget) => {
                        exhausted = true;
                        assert!(cache.is_empty());
                    }
                    error => panic!("steps {steps}: {error:?}"),
                }
                assert!(budget.steps <= steps);
            }
            assert!(succeeded && exhausted);
            let mut budget = Budget {
                max_depth: 1,
                ..Budget::default()
            };
            let mut cache = HashMap::new();
            assert!(matches!(
                world.associated_type_view_cached_metered(&root, &mut budget, &mut cache),
                Err(Failure::Budget)
            ));
            assert!(budget.steps > 0 && budget.steps < Budget::default().steps);
            assert!(cache.is_empty());
        },
    );
}
