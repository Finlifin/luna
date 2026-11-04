use nir::program::{Implementation, Trait, TraitRequirement};
use nir::{DeclarationId as Id, Primitive, Sort, Term};
use type_resolution::{
    capabilities::{Qualification, World},
    implementation::{ImplementationHead, Index, Selection},
    ptc::{Budget, Context},
};
fn id(local: u32) -> Id {
    Id { package: 0, local }
}
fn var() -> Term {
    Term::Rigid {
        declaration: id(1),
        sort: Sort::Type,
    }
}
fn boxed(t: Term) -> Term {
    Term::Nominal {
        declaration: id(10),
        arguments: vec![t],
    }
}
fn implementation(
    n: u32,
    interface: u32,
    subject: Term,
    parameters: Vec<Id>,
    capabilities: Vec<TraitRequirement>,
) -> Implementation {
    Implementation {
        phases: Default::default(),
        associated_initializers: vec![],
        parameter_types: vec![],
        associated: vec![],
        interface_arguments: vec![],
        declaration: id(n),
        interface: Some(id(interface)),
        subject,
        type_parameters: parameters,
        requirements: vec![],
        capabilities,
        methods: vec![],
    }
}
fn index(implementations: &[Implementation]) -> Index {
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
    index
}
fn requirement(interface: u32, subject: Term) -> TraitRequirement {
    TraitRequirement {
        interface_arguments: vec![],
        interface: id(interface),
        subject,
    }
}
fn context() -> Context {
    let mut c = Context::default();
    c.admit_nominal_constructor(id(10));
    c
}

#[test]
fn unknown_conditional_trait_domains_do_not_exclude_the_candidate() {
    use nir::program::TypePredicate;
    use type_resolution::implementation::TypeAssessment;
    use type_resolution::ptc::Formula;
    let ty = |n| Term::Rigid {
        declaration: id(n),
        sort: Sort::Type,
    };
    let nf = |value| Term::NormalForm {
        head: id(90),
        sort: Sort::Type,
        arguments: vec![value],
    };
    let input = Term::Rigid {
        declaration: id(203),
        sort: Sort::ComptimeValue,
    };
    let subject = Term::Primitive(Primitive::Bool);
    let given = TraitRequirement {
        interface: id(101),
        subject: subject.clone(),
        interface_arguments: vec![input],
    };
    let mut candidate = implementation(20, 100, subject.clone(), vec![], vec![]);
    candidate.requirements = vec![TypePredicate::Trait(given.clone())];
    let implementations = vec![candidate];
    let mut traits = vec![marker(100), marker(101)];
    traits[1].type_parameters = vec![id(202)];
    traits[1].parameter_types = vec![nf(ty(201))];
    let mut ix = index(&implementations);
    ix.register_parameter_domain(id(203), nf(ty(204))).unwrap();
    let assumptions = [given];
    let mut ctx = context();
    ctx.assume(Formula::equal(ty(201), ty(204)));
    let world = World {
        index: &ix,
        implementations: &implementations,
        traits: &traits,
        assumptions: &assumptions,
        context: &ctx,
    };
    let result = world.resolve(id(100), &subject, Budget::default()).unwrap();
    assert!(
        matches!(result.types.assessments.get(&id(20)),
        Some(TypeAssessment::Pending(reason)) if reason.contains("UnresolvedPrerequisite") && reason.contains("UnvalidatedModel")),
        "{:?}",
        result.types.assessments
    );
    assert!(!matches!(
        result.types.selection,
        Selection::NoApplicableCandidate
    ));
    ctx.admit_congruence(id(90));
    let world = World {
        context: &ctx,
        index: &ix,
        implementations: &implementations,
        traits: &traits,
        assumptions: &assumptions,
    };
    let result = world.resolve(id(100), &subject, Budget::default()).unwrap();
    assert!(matches!(result.types.selection, Selection::Selected(selected) if selected == id(20)));
    world
        .verify_condition_support(&result, Budget::default())
        .unwrap();

    // An unresolved alternative must not block a separately established
    // disjunct, nor become a negative fact used to discharge that disjunct.
    let known = requirement(102, subject.clone());
    let mut alternatives = implementations.clone();
    alternatives[0].requirements = vec![TypePredicate::Or(vec![
        alternatives[0].requirements[0].clone(),
        TypePredicate::Trait(known.clone()),
    ])];
    let mut ix = index(&alternatives);
    ix.register_parameter_domain(id(203), nf(ty(204))).unwrap();
    let mut traits = traits.clone();
    traits.push(marker(102));
    let assumptions = [assumptions[0].clone(), known];
    let mut incomplete = context();
    incomplete.assume(Formula::equal(ty(201), ty(204)));
    let world = World {
        index: &ix,
        implementations: &alternatives,
        traits: &traits,
        assumptions: &assumptions,
        context: &incomplete,
    };
    let result = world.resolve(id(100), &subject, Budget::default()).unwrap();
    assert!(matches!(result.types.selection, Selection::Selected(selected) if selected == id(20)));
    world
        .verify_condition_support(&result, Budget::default())
        .unwrap();
}
#[test]
fn qualification_absence_is_world_local_and_does_not_poison_later_queries() {
    let mut implementations = vec![
        implementation(20, 100, var(), vec![id(1)], vec![]),
        implementation(
            21,
            100,
            boxed(var()),
            vec![id(1)],
            vec![requirement(101, var())],
        ),
    ];
    let context = context();
    let old_index = index(&implementations);
    let world = World {
        index: &old_index,
        implementations: &implementations,
        traits: &[],
        assumptions: &[],
        context: &context,
    };
    let subject = boxed(Term::Primitive(Primitive::Bool));
    let result = world.resolve(id(100), &subject, Budget::default()).unwrap();
    assert_eq!(result.types.selection, Selection::Selected(id(20)));
    assert!(matches!(
        result.qualifications[&id(21)],
        Qualification::Excluded(_)
    ));
    assert!(result.to_string().contains("absent-in-world"));
    implementations.push(implementation(
        22,
        101,
        Term::Primitive(Primitive::Bool),
        vec![],
        vec![],
    ));
    let new_index = index(&implementations);
    let world = World {
        index: &new_index,
        implementations: &implementations,
        traits: &[],
        assumptions: &[],
        context: &context,
    };
    assert_eq!(
        world
            .resolve(id(100), &subject, Budget::default())
            .unwrap()
            .types
            .selection,
        Selection::Selected(id(21))
    );
    assert!(!new_index.type_resolution_is_current(&result.types));
}
#[test]
fn qualification_abstract_absence_is_pending_and_parent_assumptions_are_scoped() {
    let implementations = vec![
        implementation(20, 100, var(), vec![id(1)], vec![]),
        implementation(
            21,
            100,
            boxed(var()),
            vec![id(1)],
            vec![requirement(101, var())],
        ),
    ];
    let index = index(&implementations);
    let context = context();
    let traits = vec![Trait {
        comptime_only: false,
        parameter_types: Vec::new(),
        associated: vec![],
        requirements: vec![],
        capabilities: vec![],
        type_parameters: vec![],
        declaration: id(102),
        self_parameter: id(102),
        name: "Child".into(),
        parents: vec![id(101).into()],
        methods: vec![],
    }];
    let assumptions = vec![requirement(102, var())];
    let world = World {
        index: &index,
        implementations: &implementations,
        traits: &traits,
        assumptions: &assumptions,
        context: &context,
    };
    let result = world
        .resolve(id(100), &boxed(var()), Budget::default())
        .unwrap();
    assert_eq!(result.types.selection, Selection::Selected(id(21)));
    assert!(result.to_string().contains("given"));
    let empty = World {
        assumptions: &[],
        ..world
    };
    assert!(matches!(
        empty
            .resolve(id(100), &boxed(var()), Budget::default())
            .unwrap()
            .types
            .selection,
        Selection::Deferred { .. }
    ));
    assert!(
        empty
            .resolve(
                id(100),
                &boxed(var()),
                Budget {
                    steps: 0,
                    ..Budget::default()
                }
            )
            .is_err()
    );
    assert!(matches!(
        empty
            .resolve(id(100), &boxed(var()), Budget::default())
            .unwrap()
            .types
            .selection,
        Selection::Deferred { .. }
    ));
}
#[test]
fn qualification_cycles_do_not_create_positive_or_negative_proofs() {
    let implementations = vec![
        implementation(20, 100, var(), vec![id(1)], vec![requirement(101, var())]),
        implementation(21, 101, var(), vec![id(1)], vec![requirement(100, var())]),
    ];
    let index = index(&implementations);
    let context = context();
    let world = World {
        index: &index,
        implementations: &implementations,
        traits: &[],
        assumptions: &[],
        context: &context,
    };
    let result = world
        .resolve(
            id(100),
            &Term::Primitive(Primitive::Bool),
            Budget::default(),
        )
        .unwrap();
    assert!(matches!(result.types.selection, Selection::Deferred { .. }));
    assert!(matches!(
        result.qualifications[&id(20)],
        Qualification::Pending(_)
    ));
    assert!(result.to_string().contains("recursive capability"));
}

#[test]
fn qualification_definite_conjunct_failure_is_not_hidden_by_a_pending_sibling() {
    for reverse in [false, true] {
        let mut requirements = vec![requirement(101, var()), requirement(102, var())];
        if reverse {
            requirements.reverse();
        }
        let implementations = vec![
            implementation(20, 100, var(), vec![id(1)], requirements),
            implementation(21, 101, var(), vec![id(1)], vec![requirement(101, var())]),
        ];
        let index = index(&implementations);
        let context = context();
        let world = World {
            index: &index,
            implementations: &implementations,
            traits: &[],
            assumptions: &[],
            context: &context,
        };
        let result = world
            .resolve(
                id(100),
                &Term::Primitive(Primitive::Bool),
                Budget::default(),
            )
            .unwrap();
        assert_eq!(result.types.selection, Selection::NoApplicableCandidate);
        assert!(matches!(
            result.qualifications[&id(20)],
            Qualification::Excluded(_)
        ));
    }
}

#[test]
fn parameterized_capability_resolution_and_cycles_use_complete_instances() {
    use nir::program::InterfaceInstance;
    let subject = Term::Primitive(Primitive::Bool);
    let mut dependency = requirement(100, subject.clone());
    dependency.interface_arguments = vec![Term::Primitive(Primitive::Char)];
    let mut first = implementation(20, 100, subject.clone(), vec![], vec![dependency]);
    first.interface_arguments = vec![Term::Primitive(Primitive::Bool)];
    let mut second = implementation(21, 100, subject.clone(), vec![], vec![]);
    second.interface_arguments = vec![Term::Primitive(Primitive::Char)];
    let implementations = vec![first, second];
    let index = index(&implementations);
    let context = context();
    let world = World {
        index: &index,
        implementations: &implementations,
        traits: &[],
        assumptions: &[],
        context: &context,
    };
    for (argument, expected) in [(Primitive::Bool, id(20)), (Primitive::Char, id(21))] {
        let interface = InterfaceInstance {
            declaration: id(100),
            arguments: vec![Term::Primitive(argument)],
        };
        let result = world
            .resolve_instance(&interface, &subject, Budget::default())
            .unwrap();
        assert_eq!(result.types.selection, Selection::Selected(expected));
    }
    let missing = InterfaceInstance {
        declaration: id(100),
        arguments: vec![Term::Primitive(Primitive::Str)],
    };
    assert_eq!(
        world
            .resolve_instance(&missing, &subject, Budget::default())
            .unwrap()
            .types
            .selection,
        Selection::NoApplicableCandidate
    );
}

#[test]
fn parameterized_parent_assumptions_substitute_all_interface_arguments() {
    use nir::program::InterfaceInstance;
    let index = Index::default();
    let context = context();
    let traits = vec![Trait {
        comptime_only: false,
        parameter_types: Vec::new(),
        associated: vec![],
        requirements: vec![],
        capabilities: vec![],
        declaration: id(102),
        self_parameter: id(103),
        type_parameters: vec![id(1)],
        name: "Child".into(),
        parents: vec![InterfaceInstance {
            declaration: id(101),
            arguments: vec![var()],
        }],
        methods: vec![],
    }];
    let mut assumption = requirement(102, boxed(Term::Primitive(Primitive::Char)));
    assumption.interface_arguments = vec![Term::Primitive(Primitive::Bool)];
    let assumptions = vec![assumption];
    let world = World {
        index: &index,
        implementations: &[],
        traits: &traits,
        assumptions: &assumptions,
        context: &context,
    };
    let mut required = requirement(101, boxed(Term::Primitive(Primitive::Char)));
    required.interface_arguments = vec![Term::Primitive(Primitive::Bool)];
    assert!(world.given(&required, Budget::default()).unwrap().is_some());
    required.interface_arguments = vec![Term::Primitive(Primitive::Char)];
    assert!(world.given(&required, Budget::default()).unwrap().is_none());
}

#[test]
fn subject_guarantees_preserve_substitution_scope_and_budget() {
    use nir::program::{InterfaceInstance, TypePredicate};
    use type_resolution::capabilities::subject_requirements;
    let rigid = |local| Term::Rigid {
        declaration: id(local),
        sort: Sort::Type,
    };
    let same = Trait {
        comptime_only: false,
        parameter_types: Vec::new(),
        associated: vec![],
        declaration: id(100),
        self_parameter: id(101),
        type_parameters: vec![id(102)],
        name: "Same".into(),
        parents: vec![],
        methods: vec![],
        capabilities: vec![],
        requirements: vec![
            TypePredicate::Equal(rigid(101), rigid(102)),
            TypePredicate::Equal(rigid(102), Term::Primitive(Primitive::Bool)),
        ],
    };
    let child = Trait {
        comptime_only: false,
        parameter_types: Vec::new(),
        associated: vec![],
        declaration: id(110),
        self_parameter: id(111),
        type_parameters: vec![id(112)],
        name: "Child".into(),
        parents: vec![InterfaceInstance {
            declaration: id(100),
            arguments: vec![rigid(112)],
        }],
        methods: vec![],
        capabilities: vec![],
        requirements: vec![],
    };
    let assumptions = vec![TraitRequirement {
        interface: id(110),
        interface_arguments: vec![rigid(2)],
        subject: rigid(3),
    }];
    let traits = vec![same, child];
    let facts = subject_requirements(&traits, &assumptions, Budget::default());
    // Parameter-only requirements cannot bootstrap interface formation.
    assert_eq!(facts, vec![TypePredicate::Equal(rigid(3), rigid(2))]);
    assert!(subject_requirements(&traits, &[], Budget::default()).is_empty());
    assert!(
        subject_requirements(
            &traits,
            &assumptions,
            Budget {
                steps: 0,
                ..Budget::default()
            }
        )
        .is_empty()
    );
    let mut malformed = assumptions.clone();
    malformed[0].interface_arguments.clear();
    assert!(subject_requirements(&traits, &malformed, Budget::default()).is_empty());
    let mut cyclic = traits.clone();
    cyclic[0].parents.push(InterfaceInstance {
        declaration: id(110),
        arguments: vec![rigid(102)],
    });
    assert_eq!(
        subject_requirements(&cyclic, &assumptions, Budget::default()),
        facts
    );
}

#[test]
fn given_capabilities_follow_parameter_edges_with_full_subject_identity() {
    use nir::program::InterfaceInstance;
    let rigid = |local| Term::Rigid {
        declaration: id(local),
        sort: Sort::Type,
    };
    let traits = vec![
        Trait {
            comptime_only: false,
            parameter_types: Vec::new(),
            associated: vec![],
            declaration: id(100),
            self_parameter: id(101),
            type_parameters: vec![id(102)],
            name: "Adapter".into(),
            parents: vec![],
            methods: vec![],
            requirements: vec![],
            capabilities: vec![TraitRequirement {
                subject: rigid(102),
                interface: id(110),
                interface_arguments: vec![rigid(101)],
            }],
        },
        Trait {
            comptime_only: false,
            parameter_types: Vec::new(),
            associated: vec![],
            declaration: id(110),
            self_parameter: id(111),
            type_parameters: vec![id(112)],
            name: "Read".into(),
            parents: vec![],
            methods: vec![],
            requirements: vec![],
            capabilities: vec![TraitRequirement {
                subject: rigid(112),
                interface: id(100),
                interface_arguments: vec![rigid(111)],
            }],
        },
    ];
    let assumption = TraitRequirement {
        subject: rigid(1),
        interface: id(100),
        interface_arguments: vec![rigid(2)],
    };
    let assumptions = vec![assumption.clone()];
    let index = Index::default();
    let context = Context::default();
    let world = World {
        index: &index,
        implementations: &[],
        traits: &traits,
        assumptions: &assumptions,
        context: &context,
    };
    let requirement = TraitRequirement {
        subject: rigid(2),
        interface: id(110),
        interface_arguments: vec![rigid(1)],
    };
    assert_eq!(
        world.given(&requirement, Budget::default()).unwrap(),
        Some(assumption)
    );
    let mut wrong = requirement.clone();
    wrong.subject = rigid(3);
    assert!(world.given(&wrong, Budget::default()).unwrap().is_none());
    wrong = requirement.clone();
    wrong.interface_arguments = vec![rigid(3)];
    assert!(world.given(&wrong, Budget::default()).unwrap().is_none());
    assert!(
        world
            .given(
                &requirement,
                Budget {
                    steps: 0,
                    ..Budget::default()
                }
            )
            .is_err_and(|error| matches!(error, nir::NominalAdmissionError::Budget))
    );
    let sources =
        type_resolution::capabilities::capability_sources(&traits, &assumptions, Budget::default())
            .unwrap();
    assert!(sources.contains(&requirement));
    assert!(sources.len() <= 3);
    assert!(
        type_resolution::capabilities::capability_sources(
            &traits,
            &assumptions,
            Budget {
                steps: 1,
                ..Budget::default()
            }
        )
        .is_err(),
        "partial member sources must not be mistaken for a complete lookup"
    );
    let mut parents = traits.clone();
    parents[0].capabilities.clear();
    parents[0].parents = vec![InterfaceInstance {
        declaration: id(110),
        arguments: vec![rigid(102)],
    }];
    parents[1].capabilities.clear();
    let sources = type_resolution::capabilities::capability_sources(
        &parents,
        &assumptions,
        Budget::default(),
    )
    .unwrap();
    assert_eq!(
        sources, assumptions,
        "parent members must keep child shadowing"
    );
}

fn marker(n: u32) -> Trait {
    Trait {
        comptime_only: false,
        parameter_types: Vec::new(),
        associated: vec![],
        requirements: vec![],
        capabilities: vec![],
        type_parameters: vec![],
        declaration: id(n),
        self_parameter: id(n),
        name: format!("Marker{n}"),
        parents: vec![],
        methods: vec![],
    }
}

#[test]
fn scoped_given_evidence_follows_parents_and_parameter_capabilities() {
    use nir::program::InterfaceInstance;
    use nir::{Parameter, Primitive};
    let bound = |index| Term::Bound {
        depth: 0,
        index,
        sort: Sort::Type,
    };
    let rigid = |local| Term::Rigid {
        declaration: id(local),
        sort: Sort::Type,
    };
    let mut child = marker(100);
    child.type_parameters = vec![id(101)];
    child.parents.push(InterfaceInstance {
        declaration: id(110),
        arguments: vec![rigid(101)],
    });
    let mut parent = marker(110);
    parent.type_parameters = vec![id(111)];
    parent.capabilities.push(TraitRequirement {
        interface: id(120),
        subject: rigid(111),
        interface_arguments: vec![rigid(110)],
    });
    let mut target = marker(120);
    target.type_parameters = vec![id(121)];
    // Return to the original assumption; a failed lookup must still terminate.
    target.capabilities.push(TraitRequirement {
        interface: id(100),
        subject: rigid(121),
        interface_arguments: vec![rigid(120)],
    });
    let traits = vec![child, parent, target];
    let assumption = TraitRequirement {
        interface: id(100),
        subject: bound(0),
        interface_arguments: vec![bound(1)],
    };
    let assumptions = vec![assumption.clone()];
    let index = Index::default();
    let context = Context::default();
    let world = World {
        index: &index,
        implementations: &[],
        traits: &traits,
        assumptions: &assumptions,
        context: &context,
    };
    let scope = vec![
        Parameter {
            sort: Sort::Type,
            ty: Term::Primitive(Primitive::Type)
        };
        2
    ];
    let requested = TraitRequirement {
        interface: id(120),
        subject: bound(1),
        interface_arguments: vec![bound(0)],
    };
    assert_eq!(
        world
            .given_in_scope(&requested, Budget::default(), &scope)
            .unwrap(),
        Some(assumption.clone())
    );
    let sources = type_resolution::capabilities::capability_sources_in_scope(
        &traits,
        &assumptions,
        Budget::default(),
        &[vec![Sort::Type, Sort::Type]],
    )
    .unwrap();
    assert!(sources.contains(&assumption));
    assert!(sources.contains(&requested));
    assert!(
        !sources.iter().any(|source| source.interface == id(110)),
        "parent members must remain under their child lookup root"
    );
    assert!(
        type_resolution::capabilities::capability_sources_in_scope(
            &traits,
            &assumptions,
            Budget {
                steps: 1,
                ..Budget::default()
            },
            &[vec![Sort::Type, Sort::Type]],
        )
        .is_err(),
        "partial lookup roots cannot establish an unambiguous member"
    );
    assert!(
        type_resolution::capabilities::capability_sources_in_scope(
            &traits,
            &assumptions,
            Budget::default(),
            &[vec![Sort::Type]],
        )
        .is_err()
    );
    assert!(
        type_resolution::capabilities::capability_sources(&traits, &assumptions, Budget::default())
            .is_err()
    );
    let mut invalid_traits = traits.clone();
    invalid_traits[1].capabilities[0].subject = Term::Bound {
        depth: 0,
        index: 9,
        sort: Sort::Type,
    };
    assert!(
        type_resolution::capabilities::capability_sources_in_scope(
            &invalid_traits,
            &assumptions,
            Budget::default(),
            &[vec![Sort::Type, Sort::Type]],
        )
        .is_err(),
        "a broken inherited edge must not silently remove a competing member source"
    );
    assert!(
        world
            .given(&requested, Budget::default())
            .is_err_and(|error| matches!(error, nir::NominalAdmissionError::Invalid(_)))
    );
    let mut wrong = requested.clone();
    wrong.subject = bound(0);
    assert!(
        world
            .given_in_scope(&wrong, Budget::default(), &scope)
            .is_err_and(|error| matches!(error, nir::NominalAdmissionError::Unresolved(_)))
    );
    wrong = requested.clone();
    wrong.interface_arguments = vec![bound(1)];
    assert!(
        world
            .given_in_scope(&wrong, Budget::default(), &scope)
            .is_err_and(|error| matches!(error, nir::NominalAdmissionError::Unresolved(_)))
    );
    wrong = requested.clone();
    wrong.interface = id(130);
    assert!(
        world
            .given_in_scope(&wrong, Budget::default(), &scope)
            .unwrap()
            .is_none()
    );
    assert!(
        world
            .given_in_scope(
                &requested,
                Budget {
                    steps: 1,
                    ..Budget::default()
                },
                &scope
            )
            .is_err_and(|error| matches!(error, nir::NominalAdmissionError::Budget))
    );
    assert!(
        world
            .given_in_scope(&assumption, Budget::default(), &scope[..1])
            .is_err_and(|error| matches!(error, nir::NominalAdmissionError::Invalid(_)))
    );
    let invalid_scope = vec![
        Parameter {
            sort: Sort::Type,
            ty: bound(9)
        };
        2
    ];
    assert!(
        world
            .given_in_scope(&assumption, Budget::default(), &invalid_scope)
            .is_err_and(|error| matches!(error, nir::NominalAdmissionError::Invalid(_)))
    );
    // No evidence is cached into a subsequent query with different assumptions.
    let empty = World {
        assumptions: &[],
        ..world
    };
    assert!(
        empty
            .given_in_scope(&requested, Budget::default(), &scope)
            .unwrap()
            .is_none()
    );
}

#[test]
fn scoped_given_evidence_retains_closed_type_equalities() {
    let index = Index::default();
    let mut context = Context::default();
    context.assume(type_resolution::ptc::Formula::equal(
        var(),
        Term::Primitive(Primitive::Bool),
    ));
    let assumptions = vec![requirement(100, var())];
    let traits = vec![marker(100)];
    let world = World {
        index: &index,
        implementations: &[],
        traits: &traits,
        assumptions: &assumptions,
        context: &context,
    };
    let scope = vec![nir::Parameter {
        sort: Sort::Type,
        ty: Term::Primitive(Primitive::Type),
    }];
    assert_eq!(
        world
            .given_in_scope(
                &requirement(100, Term::Primitive(Primitive::Bool)),
                Budget::default(),
                &scope
            )
            .unwrap(),
        Some(assumptions[0].clone())
    );
    assert!(
        world
            .given_in_scope(
                &requirement(100, Term::Primitive(Primitive::Char)),
                Budget::default(),
                &scope
            )
            .unwrap()
            .is_none()
    );
    let malformed = vec![requirement(100, Term::Bool(true))];
    let invalid_world = World {
        assumptions: &malformed,
        ..world
    };
    assert!(
        invalid_world
            .given_in_scope(&malformed[0], Budget::default(), &scope)
            .is_err_and(|error| matches!(error, nir::NominalAdmissionError::Invalid(_)))
    );
}
#[test]
fn symbol_indexed_world_queries_check_domains_and_preserve_evidence() {
    use type_resolution::{
        capabilities::TraitTruth,
        ptc::{Formula, Outcome},
    };
    let mut contract = marker(100);
    contract.type_parameters = vec![id(2), id(3)];
    contract.parameter_types = vec![
        Term::Primitive(Primitive::Symbol),
        Term::Primitive(Primitive::Type),
    ];
    let traits = vec![contract];
    let mut supplied = implementation(20, 100, Term::Primitive(Primitive::Integer), vec![], vec![]);
    supplied.interface_arguments = vec![Term::Symbol("px".into()), Term::Primitive(Primitive::U32)];
    let implementations = vec![supplied];
    let ix = index(&implementations);
    let context = Context::default();
    let world = World {
        index: &ix,
        implementations: &implementations,
        traits: &traits,
        assumptions: &[],
        context: &context,
    };
    let query = type_resolution::ptc::TraitQuery {
        interface: id(100),
        subject: Term::Primitive(Primitive::Integer),
        arguments: implementations[0].interface_arguments.clone(),
    };
    assert_eq!(
        world
            .decide_trait(&query, Budget::default())
            .unwrap()
            .truth(),
        TraitTruth::Established
    );
    let proof = world
        .prove_proposition(Formula::Trait(query.clone()), Budget::default())
        .unwrap();
    assert!(matches!(proof.outcome(), Outcome::Entailed(_)));
    proof.verify(&world, Budget::default()).unwrap();
    let mut missing = query.clone();
    missing.arguments[0] = Term::Symbol("em".into());
    assert_eq!(
        world
            .decide_trait(&missing, Budget::default())
            .unwrap()
            .truth(),
        TraitTruth::Excluded
    );
    for invalid in [
        Term::String("px".into()),
        Term::Primitive(Primitive::Symbol),
        Term::Bool(true),
    ] {
        let mut wrong = query.clone();
        wrong.arguments[0] = invalid;
        assert!(world.decide_trait(&wrong, Budget::default()).is_err());
        assert!(
            world
                .resolve_instance(
                    &nir::program::InterfaceInstance {
                        declaration: wrong.interface,
                        arguments: wrong.arguments.clone(),
                    },
                    &wrong.subject,
                    Budget::default()
                )
                .is_err()
        );
    }
    assert!(
        world
            .decide_trait(
                &query,
                Budget {
                    steps: 1,
                    ..Budget::default()
                }
            )
            .is_err()
    );
    assert_eq!(
        world
            .decide_trait(&query, Budget::default())
            .unwrap()
            .truth(),
        TraitTruth::Established
    );
    assert!(traits[0].to_string().contains("(parameter-types"));
}

#[test]
fn symbol_indexed_parent_assumptions_keep_input_sorts() {
    let mut parent = marker(100);
    parent.type_parameters = vec![id(2)];
    parent.parameter_types = vec![Term::Primitive(Primitive::Symbol)];
    parent.capabilities.push(TraitRequirement {
        interface: id(102),
        subject: Term::Rigid {
            declaration: parent.self_parameter,
            sort: Sort::Type,
        },
        interface_arguments: vec![Term::Rigid {
            declaration: id(2),
            sort: Sort::ComptimeValue,
        }],
    });
    let mut target = marker(102);
    target.type_parameters = vec![id(4)];
    target.parameter_types = vec![Term::Primitive(Primitive::Symbol)];
    let mut child = marker(101);
    child.type_parameters = vec![id(3)];
    child.parameter_types = vec![Term::Primitive(Primitive::Symbol)];
    child.parents.push(nir::program::InterfaceInstance {
        declaration: id(100),
        arguments: vec![Term::Rigid {
            declaration: id(3),
            sort: Sort::ComptimeValue,
        }],
    });
    let traits = vec![parent, child, target];
    let subject = Term::Primitive(Primitive::Integer);
    let assumptions = vec![TraitRequirement {
        interface: id(101),
        subject: subject.clone(),
        interface_arguments: vec![Term::Symbol("px".into())],
    }];
    let ix = Index::default();
    let context = Context::default();
    let world = World {
        index: &ix,
        implementations: &[],
        traits: &traits,
        assumptions: &assumptions,
        context: &context,
    };
    let mut query = TraitRequirement {
        interface: id(100),
        subject,
        interface_arguments: vec![Term::Symbol("px".into())],
    };
    assert!(world.given(&query, Budget::default()).unwrap().is_some());
    query.interface_arguments[0] = Term::Symbol("em".into());
    assert!(world.given(&query, Budget::default()).unwrap().is_none());
    let sources =
        type_resolution::capabilities::capability_sources(&traits, &assumptions, Budget::default())
            .unwrap();
    assert!(sources.iter().any(|source| source.interface == id(102)
        && source.interface_arguments == vec![Term::Symbol("px".into())]));
    let mut invalid = traits[0].clone();
    invalid
        .parameter_types
        .push(Term::Primitive(Primitive::Type));
    assert!(invalid.parameter_binders(&mut 1000).is_err());
    invalid = traits[0].clone();
    invalid.parameter_types[0] = Term::Bool(true);
    assert!(invalid.parameter_binders(&mut 1000).is_err());
}

fn trait_query(n: u32, subject: Term) -> type_resolution::ptc::TraitQuery {
    type_resolution::ptc::TraitQuery {
        interface: id(n),
        arguments: vec![],
        subject,
    }
}

#[test]
fn trait_decisions_bind_absence_and_logical_proofs_to_the_exact_world() {
    use type_resolution::{
        capabilities::TraitTruth,
        ptc::{Formula, Outcome},
    };
    let traits = vec![marker(100)];
    let ctx = context();
    let empty = index(&[]);
    let world = World {
        index: &empty,
        implementations: &[],
        traits: &traits,
        assumptions: &[],
        context: &ctx,
    };
    let query = trait_query(100, Term::Primitive(Primitive::Bool));
    let absent = world.decide_trait(&query, Budget::default()).unwrap();
    assert_eq!(absent.truth(), TraitTruth::Excluded);
    assert!(absent.belongs_to(&world));
    let proposition = world
        .prove_proposition(Formula::Trait(query.clone()).negate(), Budget::default())
        .unwrap();
    let Outcome::Entailed(proof) = proposition.outcome() else {
        panic!("missing negative proof")
    };
    assert!(ctx.verify_proof(proof, Budget::default()).is_err());
    let implementations = vec![implementation(
        20,
        100,
        query.subject.clone(),
        vec![],
        vec![],
    )];
    let new_index = index(&implementations);
    let new_world = World {
        index: &new_index,
        implementations: &implementations,
        traits: &traits,
        assumptions: &[],
        context: &ctx,
    };
    assert!(!absent.belongs_to(&new_world));
    let present = new_world.decide_trait(&query, Budget::default()).unwrap();
    assert_eq!(present.truth(), TraitTruth::Established);
    assert!(matches!(
        new_world
            .prove_proposition(Formula::Trait(query), Budget::default())
            .unwrap()
            .outcome(),
        Outcome::Entailed(_)
    ));
    assert_eq!(absent.truth(), TraitTruth::Excluded);
    println!("TRAIT_DECISION_DUMP={absent}");
}

#[test]
fn trait_decisions_keep_open_and_recursive_queries_pending() {
    use type_resolution::capabilities::TraitTruth;
    let ctx = context();
    let traits = vec![marker(100)];
    let empty = index(&[]);
    let world = World {
        index: &empty,
        implementations: &[],
        traits: &traits,
        assumptions: &[],
        context: &ctx,
    };
    assert_eq!(
        world
            .decide_trait(&trait_query(100, var()), Budget::default())
            .unwrap()
            .truth(),
        TraitTruth::Pending
    );
    let subject = Term::Primitive(Primitive::Bool);
    let implementations = vec![implementation(
        20,
        100,
        subject.clone(),
        vec![],
        vec![requirement(100, subject.clone())],
    )];
    let recursive = index(&implementations);
    let world = World {
        index: &recursive,
        implementations: &implementations,
        traits: &traits,
        assumptions: &[],
        context: &ctx,
    };
    let decision = world
        .decide_trait(&trait_query(100, subject), Budget::default())
        .unwrap();
    assert_eq!(decision.truth(), TraitTruth::Pending);
    assert!(decision.resolution().is_some());
}

#[test]
fn trait_decisions_retain_nested_exclusion_evidence_and_given_origins() {
    use type_resolution::capabilities::TraitTruth;
    let ctx = context();
    let traits = vec![marker(100), marker(101)];
    let subject = Term::Primitive(Primitive::Bool);
    let implementations = vec![implementation(
        20,
        100,
        subject.clone(),
        vec![],
        vec![requirement(101, subject.clone())],
    )];
    let ix = index(&implementations);
    let world = World {
        index: &ix,
        implementations: &implementations,
        traits: &traits,
        assumptions: &[],
        context: &ctx,
    };
    let excluded = world
        .decide_trait(&trait_query(100, subject), Budget::default())
        .unwrap();
    assert_eq!(excluded.truth(), TraitTruth::Excluded);
    assert!(matches!(
        excluded.resolution().unwrap().qualifications[&id(20)],
        Qualification::Excluded(_)
    ));
    let assumptions = vec![requirement(100, var())];
    let world = World {
        index: &ix,
        implementations: &implementations,
        traits: &traits,
        assumptions: &assumptions,
        context: &ctx,
    };
    let given = world
        .decide_trait(&trait_query(100, var()), Budget::default())
        .unwrap();
    assert_eq!(given.truth(), TraitTruth::Established);
    assert!(given.resolution().is_none());
}

#[test]
fn trait_decisions_validate_formation_and_recover_after_exhaustion() {
    use type_resolution::capabilities::TraitTruth;
    let ctx = context();
    let traits = vec![marker(100)];
    let ix = index(&[]);
    let world = World {
        index: &ix,
        implementations: &[],
        traits: &traits,
        assumptions: &[],
        context: &ctx,
    };
    let query = trait_query(100, Term::Primitive(Primitive::Bool));
    assert!(
        world
            .decide_trait(
                &query,
                Budget {
                    steps: 0,
                    ..Budget::default()
                }
            )
            .is_err()
    );
    assert!(
        world
            .decide_trait(
                &query,
                Budget {
                    max_depth: 1,
                    ..Budget::default()
                }
            )
            .is_err()
    );
    assert!(
        world
            .decide_trait(&trait_query(101, query.subject.clone()), Budget::default())
            .is_err()
    );
    assert!(
        world
            .decide_trait(&trait_query(100, Term::Bool(true)), Budget::default())
            .is_err()
    );
    let mut malformed = query.clone();
    malformed.arguments.push(query.subject.clone());
    assert!(world.decide_trait(&malformed, Budget::default()).is_err());
    assert_eq!(
        world
            .decide_trait(&query, Budget::default())
            .unwrap()
            .truth(),
        TraitTruth::Excluded
    );
}

#[test]
fn world_propositions_combine_positive_negative_and_type_evidence() {
    use type_resolution::ptc::{Formula as F, Outcome};
    let ctx = context();
    let traits = vec![marker(100), marker(101)];
    let subject = Term::Primitive(Primitive::Bool);
    let implementations = vec![implementation(20, 100, subject.clone(), vec![], vec![])];
    let ix = index(&implementations);
    let world = World {
        index: &ix,
        implementations: &implementations,
        traits: &traits,
        assumptions: &[],
        context: &ctx,
    };
    let a = F::Trait(trait_query(100, subject.clone()));
    let b = F::Trait(trait_query(101, subject.clone()));
    let goal = F::And(vec![
        a.clone(),
        b.clone().negate(),
        F::equal(subject.clone(), subject),
    ]);
    let proof = world.prove_proposition(goal, Budget::default()).unwrap();
    assert!(matches!(proof.outcome(), Outcome::Entailed(_)));
    assert_eq!(proof.decisions().len(), 2);
    proof.verify(&world, Budget::default()).unwrap();
    println!("WORLD_PROPOSITION_DUMP={proof}");
    let equivalent_world = World {
        index: &ix,
        implementations: &implementations,
        traits: &traits,
        assumptions: &[],
        context: &ctx,
    };
    assert!(proof.verify(&equivalent_world, Budget::default()).is_err());
    let rejected = world
        .prove_proposition(F::And(vec![a.negate(), b]), Budget::default())
        .unwrap();
    assert!(!matches!(rejected.outcome(), Outcome::Entailed(_)));
}

#[test]
fn world_propositions_use_atoms_from_assumptions_and_keep_unknowns_open() {
    use type_resolution::ptc::{Formula as F, Outcome};
    let mut ctx = context();
    let traits = vec![marker(100), marker(101)];
    let a = F::Trait(trait_query(100, var()));
    let b = F::Trait(trait_query(101, Term::Primitive(Primitive::Bool)));
    ctx.assume(F::Or(vec![a.clone(), b]));
    let ix = index(&[]);
    let world = World {
        index: &ix,
        implementations: &[],
        traits: &traits,
        assumptions: &[],
        context: &ctx,
    };
    let result = world
        .prove_proposition(a.clone(), Budget::default())
        .unwrap();
    assert!(matches!(result.outcome(), Outcome::Entailed(_)));
    result.verify(&world, Budget::default()).unwrap();
    assert_eq!(result.decisions().len(), 2);
    assert!(
        world
            .prove_proposition(
                a.clone(),
                Budget {
                    steps: 1,
                    ..Budget::default()
                }
            )
            .is_err()
    );
    assert!(matches!(
        world
            .prove_proposition(a, Budget::default())
            .unwrap()
            .outcome(),
        Outcome::Entailed(_)
    ));
}

#[test]
fn world_propositions_do_not_turn_invalid_interface_formation_into_absence() {
    use nir::program::TypePredicate;
    let ctx = context();
    let mut contract = marker(100);
    contract.requirements.push(TypePredicate::Bool(false));
    let traits = vec![contract];
    let ix = index(&[]);
    let world = World {
        index: &ix,
        implementations: &[],
        traits: &traits,
        assumptions: &[],
        context: &ctx,
    };
    assert!(
        world
            .decide_trait(
                &trait_query(100, Term::Primitive(Primitive::Bool)),
                Budget::default()
            )
            .is_err()
    );
}

#[test]
fn world_propositions_leave_conflicting_candidates_pending() {
    use type_resolution::{
        capabilities::TraitTruth,
        ptc::{Formula, Outcome},
    };
    let ctx = context();
    let traits = vec![marker(100)];
    let subject = Term::Primitive(Primitive::Bool);
    let implementations = vec![
        implementation(20, 100, subject.clone(), vec![], vec![]),
        implementation(21, 100, subject.clone(), vec![], vec![]),
    ];
    let ix = index(&implementations);
    let world = World {
        index: &ix,
        implementations: &implementations,
        traits: &traits,
        assumptions: &[],
        context: &ctx,
    };
    let query = trait_query(100, subject);
    let decision = world.decide_trait(&query, Budget::default()).unwrap();
    assert_eq!(decision.truth(), TraitTruth::Pending);
    assert!(matches!(
        decision.resolution().unwrap().types.selection,
        Selection::Conflict(_)
    ));
    let proposition = world
        .prove_proposition(Formula::Trait(query).negate(), Budget::default())
        .unwrap();
    assert!(!matches!(proposition.outcome(), Outcome::Entailed(_)));
}

#[test]
fn matched_candidate_requirements_are_proved_only_in_the_selected_world() {
    use nir::program::TypePredicate;
    use type_resolution::ptc::{Formula, Outcome};
    let ctx = context();
    let traits = vec![marker(100), marker(101)];
    let mut candidate = implementation(20, 100, var(), vec![id(1)], vec![]);
    candidate.requirements = vec![TypePredicate::Not(Box::new(TypePredicate::Trait(
        requirement(101, var()),
    )))];
    let head = ImplementationHead {
        parameter_types: candidate.parameter_types.clone(),
        declaration: candidate.declaration,
        interface: candidate.interface,
        interface_arguments: vec![],
        subject: candidate.subject.clone(),
        type_parameters: candidate.type_parameters.clone(),
        requirements: candidate.requirements.clone(),
    };
    let subject = Term::Primitive(Primitive::Bool);
    let matched = head
        .match_types(&subject, &[], &ctx, Budget::default())
        .unwrap();
    let goal = Formula::And(
        matched
            .requirements()
            .iter()
            .map(Formula::from_predicate)
            .collect(),
    );
    let implementations = vec![candidate.clone()];
    let ix = index(&implementations);
    let world = World {
        index: &ix,
        implementations: &implementations,
        traits: &traits,
        assumptions: &[],
        context: &ctx,
    };
    let established = world
        .prove_proposition(goal.clone(), Budget::default())
        .unwrap();
    assert!(matches!(established.outcome(), Outcome::Entailed(_)));
    established.verify(&world, Budget::default()).unwrap();
    let implementations = vec![candidate, implementation(21, 101, subject, vec![], vec![])];
    let ix = index(&implementations);
    let changed = World {
        index: &ix,
        implementations: &implementations,
        traits: &traits,
        assumptions: &[],
        context: &ctx,
    };
    assert!(established.verify(&changed, Budget::default()).is_err());
    assert!(!matches!(
        changed
            .prove_proposition(goal, Budget::default())
            .unwrap()
            .outcome(),
        Outcome::Entailed(_)
    ));
}

#[test]
fn conditional_implementation_selection_retains_world_support_and_changes_with_world() {
    use nir::program::TypePredicate as Q;
    let ctx = context();
    let traits = vec![marker(100), marker(101)];
    let subject = Term::Primitive(Primitive::Bool);
    let mut candidate = implementation(20, 100, var(), vec![id(1)], vec![]);
    candidate.requirements = vec![Q::Not(Box::new(Q::Trait(requirement(101, var()))))];
    let implementations = vec![candidate.clone()];
    let ix = index(&implementations);
    let world = World {
        index: &ix,
        implementations: &implementations,
        traits: &traits,
        assumptions: &[],
        context: &ctx,
    };
    let selected = world.resolve(id(100), &subject, Budget::default()).unwrap();
    assert_eq!(selected.types.selection, Selection::Selected(id(20)));
    assert!(!selected.condition_support[&id(20)].is_empty());
    world
        .verify_condition_support(&selected, Budget::default())
        .unwrap();
    println!("CONDITIONAL_SELECTION_DUMP={selected}");
    let implementations = vec![
        candidate,
        implementation(21, 101, subject.clone(), vec![], vec![]),
    ];
    let ix = index(&implementations);
    let changed = World {
        index: &ix,
        implementations: &implementations,
        traits: &traits,
        assumptions: &[],
        context: &ctx,
    };
    assert!(
        changed
            .verify_condition_support(&selected, Budget::default())
            .is_err()
    );
    let excluded = changed
        .resolve(id(100), &subject, Budget::default())
        .unwrap();
    assert_eq!(excluded.types.selection, Selection::NoApplicableCandidate);
    changed
        .verify_condition_support(&excluded, Budget::default())
        .unwrap();
}

#[test]
fn conditional_implementation_cycles_never_prove_themselves_or_their_negation() {
    use nir::program::TypePredicate as Q;
    let ctx = context();
    let traits = vec![marker(100), marker(101)];
    let subject = Term::Primitive(Primitive::Bool);
    for negative in [false, true] {
        let mut candidate = implementation(20, 100, var(), vec![id(1)], vec![]);
        let atom = Q::Trait(requirement(100, var()));
        candidate.requirements = vec![if negative {
            Q::Not(Box::new(atom))
        } else {
            atom
        }];
        let implementations = vec![candidate];
        let ix = index(&implementations);
        let world = World {
            index: &ix,
            implementations: &implementations,
            traits: &traits,
            assumptions: &[],
            context: &ctx,
        };
        assert!(matches!(
            world
                .resolve(id(100), &subject, Budget::default())
                .unwrap()
                .types
                .selection,
            Selection::Deferred { .. }
        ));
        assert!(
            world
                .resolve(
                    id(100),
                    &subject,
                    Budget {
                        steps: 0,
                        ..Budget::default()
                    }
                )
                .is_err()
        );
        assert!(matches!(
            world
                .resolve(id(100), &subject, Budget::default())
                .unwrap()
                .types
                .selection,
            Selection::Deferred { .. }
        ));
    }
}

#[test]
fn conditional_implementation_nested_disjunctions_share_the_solver() {
    use nir::program::TypePredicate as Q;
    let ctx = context();
    let traits = vec![marker(100), marker(101), marker(102), marker(103)];
    let subject = Term::Primitive(Primitive::Bool);
    let mut outer = implementation(20, 100, var(), vec![id(1)], vec![]);
    outer.requirements = vec![Q::Or(vec![
        Q::Trait(requirement(101, var())),
        Q::Trait(requirement(103, var())),
    ])];
    let mut inner = implementation(21, 101, var(), vec![id(1)], vec![]);
    inner.requirements = vec![Q::Not(Box::new(Q::Trait(requirement(102, var()))))];
    let implementations = vec![outer, inner];
    let ix = index(&implementations);
    let world = World {
        index: &ix,
        implementations: &implementations,
        traits: &traits,
        assumptions: &[],
        context: &ctx,
    };
    let mut exhausted = Budget {
        steps: 0,
        ..Budget::default()
    };
    assert!(
        world
            .resolve_instance_metered(&id(100).into(), &subject, &mut exhausted)
            .is_err()
    );
    // Repeated independent queries share one caller allowance. Successful
    // proofs charge work, rather than consuming each reserved proof slot.
    let mut shared = Budget::default();
    for _ in 0..8 {
        let before = shared.steps;
        let resolution = world
            .resolve_instance_metered(&id(100).into(), &subject, &mut shared)
            .unwrap();
        assert_eq!(resolution.types.selection, Selection::Selected(id(20)));
        assert!(shared.steps < before);
        world
            .verify_condition_support(&resolution, Budget::default())
            .unwrap();
    }
    let resolution = world.resolve(id(100), &subject, Budget::default()).unwrap();
    assert_eq!(resolution.types.selection, Selection::Selected(id(20)));
    world
        .verify_condition_support(&resolution, Budget::default())
        .unwrap();
}

#[test]
fn conditional_inherent_evidence_preserves_member_subject_and_both_indexes() {
    use nir::program::TypePredicate as Q;
    use type_resolution::implementation::{InherentHead, InherentIndex};
    let ctx = context();
    let traits = vec![marker(100)];
    let mut candidate = implementation(20, 100, boxed(var()), vec![id(1)], vec![]);
    candidate.interface = None;
    candidate.requirements = vec![Q::Not(Box::new(Q::Trait(requirement(100, var()))))];
    let head = InherentHead {
        parameter_types: candidate.parameter_types.clone(),
        declaration: candidate.declaration,
        subject: candidate.subject.clone(),
        type_parameters: candidate.type_parameters.clone(),
        requirements: candidate.requirements.clone(),
        members: ["read".into()].into_iter().collect(),
    };
    let implementations = vec![candidate];
    let ix = Index::default();
    let mut inherent = InherentIndex::default();
    inherent.insert(head.clone(), Budget::default()).unwrap();
    let world = World {
        index: &ix,
        implementations: &implementations,
        traits: &traits,
        assumptions: &[],
        context: &ctx,
    };
    let subject = boxed(Term::Primitive(Primitive::Bool));
    let resolution = world
        .resolve_inherent(&inherent, "read", &subject, Budget::default())
        .unwrap();
    assert_eq!(resolution.types.selection, Selection::Selected(id(20)));
    world
        .verify_inherent_evidence(&inherent, "read", &subject, &resolution, Budget::default())
        .unwrap();
    let mut exhausted = 0;
    let mut completed = 0;
    for steps in (0..4000).step_by(13) {
        use nir::NominalAdmissionError as QueryError;
        use type_resolution::implementation::EvidenceError;
        use type_resolution::ptc::ProofError;
        match world.verify_inherent_evidence(
            &inherent,
            "read",
            &subject,
            &resolution,
            Budget {
                steps,
                ..Budget::default()
            },
        ) {
            Ok(()) => completed += 1,
            Err(
                EvidenceError::Query(QueryError::Budget)
                | EvidenceError::Proof(ProofError::BudgetExhausted),
            ) => exhausted += 1,
            error => panic!("steps {steps}: {error:?}"),
        }
    }
    assert!(exhausted > 0 && completed > 0);
    world
        .verify_inherent_evidence(&inherent, "read", &subject, &resolution, Budget::default())
        .unwrap();
    assert!(
        world
            .verify_inherent_evidence(&inherent, "wrong", &subject, &resolution, Budget::default())
            .is_err()
    );
    assert!(
        world
            .verify_inherent_evidence(
                &inherent,
                "read",
                &boxed(Term::Primitive(Primitive::Char)),
                &resolution,
                Budget::default()
            )
            .is_err()
    );
    assert!(
        world
            .verify_condition_support(&resolution, Budget::default())
            .is_err()
    );
    let other_index = Index::default();
    let changed = World {
        index: &other_index,
        implementations: &implementations,
        traits: &traits,
        assumptions: &[],
        context: &ctx,
    };
    assert!(
        changed
            .verify_inherent_evidence(&inherent, "read", &subject, &resolution, Budget::default())
            .is_err()
    );
    let mut other_inherent = InherentIndex::default();
    other_inherent.insert(head, Budget::default()).unwrap();
    assert!(
        world
            .verify_inherent_evidence(
                &other_inherent,
                "read",
                &subject,
                &resolution,
                Budget::default()
            )
            .is_err()
    );
    println!("INHERENT_CONDITION_DUMP={resolution}");
}

#[test]
fn conditional_evidence_rejects_removed_nested_capability_assumptions() {
    use nir::program::TypePredicate as Q;
    let ctx = context();
    let traits = vec![marker(100), marker(101), marker(102)];
    let subject = Term::Primitive(Primitive::Bool);
    let mut outer = implementation(20, 100, var(), vec![id(1)], vec![]);
    outer.requirements = vec![Q::Trait(requirement(101, var()))];
    let inner = implementation(21, 101, var(), vec![id(1)], vec![requirement(102, var())]);
    let implementations = vec![outer, inner];
    let ix = index(&implementations);
    let assumptions = vec![requirement(102, subject.clone())];
    let world = World {
        index: &ix,
        implementations: &implementations,
        traits: &traits,
        assumptions: &assumptions,
        context: &ctx,
    };
    let resolution = world.resolve(id(100), &subject, Budget::default()).unwrap();
    assert_eq!(resolution.types.selection, Selection::Selected(id(20)));
    world
        .verify_condition_support(&resolution, Budget::default())
        .unwrap();
    let changed = World {
        index: &ix,
        implementations: &implementations,
        traits: &traits,
        assumptions: &[],
        context: &ctx,
    };
    assert!(
        changed
            .verify_condition_support(&resolution, Budget::default())
            .is_err()
    );
    assert_ne!(
        changed
            .resolve(id(100), &subject, Budget::default())
            .unwrap()
            .types
            .selection,
        Selection::Selected(id(20))
    );
}

#[test]
fn condition_scope_covers_type_context_even_without_boolean_support() {
    let ctx = context();
    let traits = vec![marker(100)];
    let subject = Term::Primitive(Primitive::Bool);
    let implementations = vec![implementation(20, 100, subject.clone(), vec![], vec![])];
    let ix = index(&implementations);
    let world = World {
        index: &ix,
        implementations: &implementations,
        traits: &traits,
        assumptions: &[],
        context: &ctx,
    };
    let resolution = world.resolve(id(100), &subject, Budget::default()).unwrap();
    assert_eq!(resolution.types.selection, Selection::Selected(id(20)));
    assert!(resolution.condition_support.is_empty());
    for contracts in [false, true] {
        let mut changed_context = ctx.clone();
        if contracts {
            changed_context.admit_nominal_constructor(id(99));
        } else {
            changed_context.assume(type_resolution::ptc::Formula::True);
        }
        let changed = World {
            index: &ix,
            implementations: &implementations,
            traits: &traits,
            assumptions: &[],
            context: &changed_context,
        };
        assert!(
            changed
                .verify_condition_support(&resolution, Budget::default())
                .is_err()
        );
    }
    let cloned_context = ctx.clone();
    let restored = World {
        index: &ix,
        implementations: &implementations,
        traits: &traits,
        assumptions: &[],
        context: &cloned_context,
    };
    restored
        .verify_condition_support(&resolution, Budget::default())
        .unwrap();
    assert!(
        restored
            .verify_condition_support(
                &resolution,
                Budget {
                    steps: 0,
                    ..Budget::default()
                }
            )
            .is_err()
    );
    restored
        .verify_condition_support(&resolution, Budget::default())
        .unwrap();
    println!("RESOLUTION_SCOPE_DUMP={resolution}");
}

#[test]
fn absence_decisions_use_context_equalities_without_guessing_or_unfolding() {
    use type_resolution::capabilities::TraitTruth;
    use type_resolution::ptc::{Formula, TraitQuery};
    let mut ctx = context();
    ctx.assume(Formula::equal(var(), Term::Primitive(Primitive::Bool)));
    let mut parameterized = marker(101);
    parameterized.type_parameters = vec![id(2)];
    let traits = vec![marker(100), parameterized];
    let ix = Index::default();
    let world = World {
        index: &ix,
        implementations: &[],
        traits: &traits,
        assumptions: &[],
        context: &ctx,
    };
    for query in [
        TraitQuery {
            interface: id(100),
            arguments: vec![],
            subject: var(),
        },
        TraitQuery {
            interface: id(100),
            arguments: vec![],
            subject: boxed(var()),
        },
        TraitQuery {
            interface: id(101),
            arguments: vec![var()],
            subject: Term::Primitive(Primitive::Bool),
        },
    ] {
        assert_eq!(
            world
                .decide_trait(&query, Budget::default())
                .unwrap()
                .truth(),
            TraitTruth::Excluded
        );
    }
    let mut disjunctive = context();
    disjunctive.assume(Formula::Or(vec![
        Formula::equal(var(), Term::Primitive(Primitive::Bool)),
        Formula::equal(var(), Term::Primitive(Primitive::Char)),
    ]));
    let world = World {
        index: &ix,
        implementations: &[],
        traits: &traits,
        assumptions: &[],
        context: &disjunctive,
    };
    let query = TraitQuery {
        interface: id(100),
        arguments: vec![],
        subject: var(),
    };
    let decision = world.decide_trait(&query, Budget::default()).unwrap();
    assert_eq!(decision.truth(), TraitTruth::Pending);
    assert!(decision.pending_reason().is_some());
    assert!(decision.to_string().contains("pending-reason"));
    let opaque = Term::NormalForm {
        head: id(9),
        sort: Sort::Type,
        arguments: vec![var()],
    };
    let world = World {
        index: &ix,
        implementations: &[],
        traits: &traits,
        assumptions: &[],
        context: &ctx,
    };
    let query = TraitQuery {
        interface: id(100),
        arguments: vec![],
        subject: opaque,
    };
    let decision = world.decide_trait(&query, Budget::default()).unwrap();
    assert_eq!(decision.truth(), TraitTruth::Pending);
    assert!(decision.pending_reason().is_some());
    assert!(decision.to_string().contains("pending-reason"));
}

#[test]
fn scoped_given_evidence_uses_equalities_in_the_same_input_scope() {
    let parameters = vec![
        nir::Parameter {
            sort: Sort::Type,
            ty: Term::Primitive(Primitive::Type)
        };
        2
    ];
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
    context.assume(type_resolution::ptc::Formula::equal(a.clone(), b.clone()));
    let assumptions = [requirement(100, a.clone())];
    let index = Index::default();
    let world = World {
        index: &index,
        implementations: &[],
        traits: &[],
        assumptions: &assumptions,
        context: &context,
    };
    assert!(
        world
            .given_in_scope(&requirement(100, b.clone()), Budget::default(), &parameters)
            .unwrap()
            .is_some()
    );
    let empty = Context::with_scope(vec![vec![Sort::Type, Sort::Type]]);
    let world = World {
        context: &empty,
        ..world
    };
    assert!(
        world
            .given_in_scope(&requirement(100, b), Budget::default(), &parameters)
            .unwrap()
            .is_none()
    );
    assert!(
        world
            .given(&requirement(100, a), Budget::default())
            .is_err_and(|error| matches!(error, nir::NominalAdmissionError::Invalid(_)))
    );
}

#[test]
fn scoped_world_queries_preserve_abstract_absence_and_use_proved_input_views() {
    use type_resolution::{
        capabilities::TraitTruth,
        ptc::{Formula as F, Outcome},
    };
    let a = Term::Bound {
        depth: 0,
        index: 0,
        sort: Sort::Type,
    };
    let boolean = Term::Primitive(Primitive::Bool);
    let traits = vec![marker(100), marker(101)];
    let implementations = vec![implementation(20, 100, boolean.clone(), vec![], vec![])];
    let ix = index(&implementations);
    let mut ctx = Context::with_scope(vec![vec![Sort::Type]]);
    let query = trait_query(101, a.clone());
    let world = World {
        index: &ix,
        implementations: &implementations,
        traits: &traits,
        assumptions: &[],
        context: &ctx,
    };
    assert_eq!(
        world
            .decide_trait(&query, Budget::default())
            .unwrap()
            .truth(),
        TraitTruth::Pending
    );
    assert!(!matches!(
        world
            .prove_proposition(F::Trait(query.clone()).negate(), Budget::default())
            .unwrap()
            .outcome(),
        Outcome::Entailed(_)
    ));
    let malformed = trait_query(
        100,
        Term::Bound {
            depth: 0,
            index: 1,
            sort: Sort::Type,
        },
    );
    assert!(world.decide_trait(&malformed, Budget::default()).is_err());
    ctx.assume(F::equal(a.clone(), boolean));
    let world = World {
        index: &ix,
        implementations: &implementations,
        traits: &traits,
        assumptions: &[],
        context: &ctx,
    };
    let proof = world
        .prove_proposition(
            F::And(vec![
                F::Trait(trait_query(100, a)),
                F::Trait(query).negate(),
            ]),
            Budget::default(),
        )
        .unwrap();
    assert!(matches!(proof.outcome(), Outcome::Entailed(_)), "{proof}");
    assert!(format!("{proof}").contains("input-equalities"));
    proof.verify(&world, Budget::default()).unwrap();
}

#[test]
fn scoped_interface_formation_is_checked_before_given_evidence() {
    use nir::program::TypePredicate;
    use type_resolution::{
        capabilities::TraitTruth,
        ptc::{Formula, TraitQuery},
    };
    let a = Term::Bound {
        depth: 1,
        index: 0,
        sort: Sort::Type,
    };
    let b = Term::Bound {
        depth: 0,
        index: 0,
        sort: Sort::Type,
    };
    let mut contract = marker(100);
    contract.type_parameters = vec![id(1)];
    contract.requirements.push(TypePredicate::Equal(
        var(),
        Term::Primitive(Primitive::Bool),
    ));
    contract.capabilities.push(requirement(101, var()));
    let traits = vec![contract, marker(101)];
    let query = TraitQuery {
        interface: id(100),
        arguments: vec![a.clone()],
        subject: b,
    };
    let givens = vec![
        TraitRequirement {
            interface: query.interface,
            interface_arguments: query.arguments.clone(),
            subject: query.subject.clone(),
        },
        requirement(101, a.clone()),
    ];
    let ix = Index::default();
    let mut ctx = Context::with_scope(vec![vec![Sort::Type], vec![Sort::Type]]);
    {
        let world = World {
            index: &ix,
            implementations: &[],
            traits: &traits,
            assumptions: &givens,
            context: &ctx,
        };
        assert!(world.decide_trait(&query, Budget::default()).is_err());
    }
    ctx.assume(Formula::equal(a, Term::Primitive(Primitive::Bool)));
    let world = World {
        index: &ix,
        implementations: &[],
        traits: &traits,
        assumptions: &givens,
        context: &ctx,
    };
    assert_eq!(
        world
            .decide_trait(&query, Budget::default())
            .unwrap()
            .truth(),
        TraitTruth::Established
    );
    let missing_capability = World {
        assumptions: &[],
        ..world
    };
    assert!(
        missing_capability
            .decide_trait(&query, Budget::default())
            .is_err()
    );
}

#[test]
fn metered_world_propositions_share_budget_and_preserve_evidence() {
    use nir::program::TypePredicate;
    use type_resolution::ptc::{Formula, Outcome};
    let ctx = context();
    let traits = vec![marker(100)];
    let subject = Term::Primitive(Primitive::Bool);
    let implementations = vec![implementation(20, 100, subject.clone(), vec![], vec![])];
    let ix = index(&implementations);
    let world = World {
        index: &ix,
        implementations: &implementations,
        traits: &traits,
        assumptions: &[],
        context: &ctx,
    };
    let equality = TypePredicate::Equal(subject.clone(), subject.clone());
    let mut budget = Budget::default();
    assert!(matches!(
        world
            .check_predicate_metered(&equality, &mut budget)
            .unwrap(),
        Outcome::Entailed(_)
    ));
    assert!(budget.steps > 99_000 && budget.steps < 100_000);
    let goal = Formula::Trait(trait_query(100, subject.clone()));
    let before = budget.steps;
    let proof = world.prove_proposition_metered(goal, &mut budget).unwrap();
    assert!(matches!(proof.outcome(), Outcome::Entailed(_)));
    proof.verify(&world, Budget::default()).unwrap();
    assert!(budget.steps > 0 && budget.steps < before);
    let predicate = TypePredicate::Trait(requirement(100, subject));
    let mut empty = Budget {
        steps: 0,
        max_depth: 128,
    };
    assert!(
        world
            .check_predicate_metered(&predicate, &mut empty)
            .is_err()
    );
    assert_eq!(empty.steps, 0);
    let mut tiny = Budget {
        steps: 1,
        max_depth: 128,
    };
    assert!(
        world
            .check_predicate_metered(&predicate, &mut tiny)
            .is_err()
    );
    assert_eq!(tiny.steps, 0);
    assert!(matches!(
        world
            .check_predicate_metered(&predicate, &mut budget)
            .unwrap(),
        Outcome::Entailed(_)
    ));
}

#[test]
fn rigid_value_domains_control_matching_replay_and_index_revision() {
    use type_resolution::implementation::TypeAssessment;
    let parameter = id(701);
    let external = id(702);
    let rigid = |id| Term::Rigid {
        declaration: id,
        sort: Sort::ComptimeValue,
    };
    let head = ImplementationHead {
        declaration: id(703),
        interface: Some(id(704)),
        subject: boxed(rigid(parameter)),
        interface_arguments: vec![rigid(parameter)],
        type_parameters: vec![parameter],
        parameter_types: vec![Term::Primitive(Primitive::Bool)],
        requirements: vec![],
    };
    let context = Context::default();
    let subject = boxed(rigid(external));
    let arguments = [rigid(external)];
    let mut index = Index::default();
    index.insert(head.clone(), Budget::default()).unwrap();
    let before = index
        .resolve_types(id(704), &arguments, &subject, &context, Budget::default())
        .unwrap();
    assert!(!matches!(before.selection, Selection::Selected(_)));
    index
        .register_parameter_domain(external, Term::Primitive(Primitive::Bool))
        .unwrap();
    assert!(!index.type_resolution_is_current(&before));
    let valid = index
        .resolve_types(id(704), &arguments, &subject, &context, Budget::default())
        .unwrap();
    assert!(matches!(valid.selection, Selection::Selected(_)));
    let TypeAssessment::Applicable(evidence) = &valid.assessments[&head.declaration] else {
        panic!("{valid}")
    };
    let domains = std::collections::BTreeMap::from([(external, Term::Primitive(Primitive::Bool))]);
    head.verify_type_evidence_with_domains(
        evidence,
        &subject,
        &arguments,
        &context,
        Budget::default(),
        &domains,
    )
    .unwrap();
    assert!(
        head.verify_type_evidence(evidence, &subject, &arguments, &context, Budget::default())
            .is_err()
    );
    let wrong = std::collections::BTreeMap::from([(external, Term::Primitive(Primitive::Char))]);
    assert!(
        head.verify_type_evidence_with_domains(
            evidence,
            &subject,
            &arguments,
            &context,
            Budget::default(),
            &wrong
        )
        .is_err()
    );
    assert!(
        index
            .register_parameter_domain(external, Term::Primitive(Primitive::Char))
            .is_err()
    );
    assert!(index.type_resolution_is_current(&valid));
    let other = id(705);
    index
        .register_parameter_domain(other, Term::Primitive(Primitive::Bool))
        .unwrap();
    let uncertain = index
        .resolve_types(
            id(704),
            &[rigid(other)],
            &subject,
            &context,
            Budget::default(),
        )
        .unwrap();
    assert!(!matches!(uncertain.selection, Selection::Selected(_)));
    assert!(index.to_string().contains("(parameter-domain"));
}

#[test]
fn external_value_domains_preserve_only_subject_guarantees_through_parents() {
    use nir::program::{InterfaceInstance, TypePredicate};
    use type_resolution::capabilities::subject_requirements_with_domains;
    let rigid = |local, sort| Term::Rigid {
        declaration: id(local),
        sort,
    };
    let parent = Trait {
        comptime_only: false,
        declaration: id(801),
        self_parameter: id(802),
        type_parameters: vec![id(803), id(804)],
        parameter_types: vec![
            Term::Primitive(Primitive::Type),
            Term::Primitive(Primitive::Bool),
        ],
        name: "Parent".into(),
        parents: vec![],
        methods: vec![],
        associated: vec![],
        capabilities: vec![],
        requirements: vec![
            TypePredicate::Equal(rigid(802, Sort::Type), rigid(803, Sort::Type)),
            TypePredicate::Equal(rigid(803, Sort::Type), Term::Primitive(Primitive::Bool)),
        ],
    };
    let child = Trait {
        comptime_only: false,
        declaration: id(811),
        self_parameter: id(812),
        type_parameters: vec![id(813), id(814)],
        parameter_types: parent.parameter_types.clone(),
        name: "Child".into(),
        parents: vec![InterfaceInstance {
            declaration: parent.declaration,
            arguments: vec![rigid(813, Sort::Type), rigid(814, Sort::ComptimeValue)],
        }],
        requirements: vec![],
        methods: vec![],
        associated: vec![],
        capabilities: vec![],
    };
    let assumptions = [TraitRequirement {
        subject: rigid(821, Sort::Type),
        interface: child.declaration,
        interface_arguments: vec![rigid(822, Sort::Type), rigid(823, Sort::ComptimeValue)],
    }];
    let traits = [parent, child];
    let mut domains = std::collections::BTreeMap::new();
    for domain in [None, Some(Term::Primitive(Primitive::Char))] {
        domains.clear();
        if let Some(domain) = domain {
            domains.insert(id(823), domain);
        }
        assert!(
            subject_requirements_with_domains(&traits, &assumptions, Budget::default(), &domains)
                .is_empty()
        );
    }
    domains.insert(id(823), Term::Primitive(Primitive::Bool));
    assert!(
        subject_requirements_with_domains(
            &traits,
            &assumptions,
            Budget {
                steps: 0,
                ..Default::default()
            },
            &domains
        )
        .is_empty()
    );
    assert_eq!(
        subject_requirements_with_domains(&traits, &assumptions, Budget::default(), &domains),
        vec![TypePredicate::Equal(
            rigid(821, Sort::Type),
            rigid(822, Sort::Type)
        )]
    );
}

#[test]
fn repeated_value_indices_do_not_prove_absence_from_an_unknown_input() {
    use type_resolution::{capabilities::TraitTruth, ptc::TraitQuery};
    let value = |n| Term::Rigid {
        declaration: id(n),
        sort: Sort::ComptimeValue,
    };
    let mut imp = implementation(
        200,
        100,
        Term::Primitive(Primitive::Bool),
        vec![id(1)],
        vec![],
    );
    imp.parameter_types = vec![Term::Primitive(Primitive::Bool)];
    imp.interface_arguments = vec![value(1), value(1)];
    let implementations = [imp];
    let mut index = index(&implementations);
    index
        .register_parameter_domain(id(2), Term::Primitive(Primitive::Bool))
        .unwrap();
    let traits = [Trait {
        comptime_only: false,
        declaration: id(100),
        self_parameter: id(101),
        type_parameters: vec![id(102), id(103)],
        parameter_types: vec![Term::Primitive(Primitive::Bool); 2],
        name: "Pair".into(),
        parents: vec![],
        methods: vec![],
        associated: vec![],
        requirements: vec![],
        capabilities: vec![],
    }];
    let context = Context::default();
    let world = World {
        index: &index,
        implementations: &implementations,
        traits: &traits,
        assumptions: &[],
        context: &context,
    };
    for (arguments, expected) in [
        (vec![value(2), Term::Bool(true)], TraitTruth::Pending),
        (vec![Term::Bool(true), value(2)], TraitTruth::Pending),
        (vec![value(2), value(2)], TraitTruth::Established),
        (
            vec![Term::Bool(true), Term::Bool(true)],
            TraitTruth::Established,
        ),
        (
            vec![Term::Bool(true), Term::Bool(false)],
            TraitTruth::Excluded,
        ),
    ] {
        let query = TraitQuery {
            interface: id(100),
            subject: Term::Primitive(Primitive::Bool),
            arguments,
        };
        assert_eq!(
            world
                .decide_trait(&query, Budget::default())
                .unwrap()
                .truth(),
            expected,
            "{query:?}"
        );
    }
}

#[test]
fn nominal_trait_indices_resolve_only_with_valid_layouts() {
    let config_id = id(90);
    let config = Term::Nominal {
        declaration: config_id,
        arguments: vec![],
    };
    let value = Term::NominalValue {
        ty: Box::new(config.clone()),
        variant: None,
        fields: vec![Term::Bool(true)],
    };
    let mut contract = marker(100);
    contract.type_parameters = vec![id(2)];
    contract.parameter_types = vec![config];
    let traits = vec![contract];
    let subject = Term::Primitive(Primitive::Bool);
    let mut supplied = implementation(20, 100, subject.clone(), vec![], vec![]);
    supplied.interface_arguments = vec![value.clone()];
    let implementations = vec![supplied];
    let mut ix = index(&implementations);
    let mut context = Context::default();
    context.admit_nominal_constructor(config_id);
    let query = nir::program::InterfaceInstance {
        declaration: id(100),
        arguments: vec![value],
    };
    let resolve = |ix: &Index, query: &nir::program::InterfaceInstance| {
        World {
            index: ix,
            implementations: &implementations,
            traits: &traits,
            assumptions: &[],
            context: &context,
        }
        .resolve_instance(query, &subject, Budget::default())
    };
    assert!(resolve(&ix, &query).is_err());
    ix.set_nominal_layouts(
        vec![nir::program::Struct {
            declaration: config_id,
            name: "Config".into(),
            type_parameters: vec![],
            parameter_types: vec![],
            fields: vec![("flag".into(), Term::Primitive(Primitive::Bool))],
            requirements: vec![],
            capabilities: vec![],
        }],
        vec![],
        vec![],
    );
    let result = resolve(&ix, &query).unwrap();
    assert_eq!(
        result.types.selection,
        Selection::Selected(id(20)),
        "{}",
        result.types
    );
    let mut forged = query.clone();
    let Term::NominalValue { fields, .. } = &mut forged.arguments[0] else {
        unreachable!()
    };
    fields[0] = Term::Char('x');
    assert!(resolve(&ix, &forged).is_err());
}

#[test]
fn nominal_capability_sources_and_subject_facts_require_layout_evidence() {
    use nir::program::TypePredicate;
    use type_resolution::capabilities::{
        capability_sources_with_layouts, subject_requirements_with_layouts,
    };
    let config_id = id(900);
    let config = Term::Nominal {
        declaration: config_id,
        arguments: vec![],
    };
    let mut contract = marker(100);
    contract.type_parameters = vec![id(2)];
    contract.parameter_types = vec![config.clone()];
    contract.requirements.push(TypePredicate::Equal(
        Term::Rigid {
            declaration: contract.self_parameter,
            sort: Sort::Type,
        },
        Term::Primitive(Primitive::Bool),
    ));
    let traits = vec![contract];
    let assumption = TraitRequirement {
        interface: id(100),
        subject: var(),
        interface_arguments: vec![Term::NominalValue {
            ty: Box::new(config),
            variant: None,
            fields: vec![Term::Bool(true)],
        }],
    };
    let mut package = nir::program::Package::default();
    package.structs.push(nir::program::Struct {
        declaration: config_id,
        name: "Config".into(),
        type_parameters: vec![],
        parameter_types: vec![],
        fields: vec![("flag".into(), Term::Primitive(Primitive::Bool))],
        requirements: vec![],
        capabilities: vec![],
    });
    let assumptions = vec![assumption];
    let domains = Default::default();
    assert!(
        capability_sources_with_layouts(
            &traits,
            &assumptions,
            Budget::default(),
            &[],
            &domains,
            None
        )
        .is_err()
    );
    assert!(
        subject_requirements_with_layouts(&traits, &assumptions, Budget::default(), &domains, None)
            .is_empty()
    );
    assert_eq!(
        capability_sources_with_layouts(
            &traits,
            &assumptions,
            Budget::default(),
            &[],
            &domains,
            Some(&package)
        )
        .unwrap(),
        assumptions
    );
    assert_eq!(
        subject_requirements_with_layouts(
            &traits,
            &assumptions,
            Budget::default(),
            &domains,
            Some(&package)
        ),
        vec![TypePredicate::Equal(
            var(),
            Term::Primitive(Primitive::Bool)
        )]
    );
    package.structs[0].fields[0].1 = Term::Primitive(Primitive::Char);
    assert!(
        capability_sources_with_layouts(
            &traits,
            &assumptions,
            Budget::default(),
            &[],
            &domains,
            Some(&package)
        )
        .is_err()
    );
    assert!(
        subject_requirements_with_layouts(
            &traits,
            &assumptions,
            Budget::default(),
            &domains,
            Some(&package)
        )
        .is_empty()
    );
}

#[test]
fn assumption_lookup_accounts_for_wide_type_inputs_and_recovers() {
    let index = Index::default();
    let context = context();
    let subject = Term::Tuple(vec![Term::Primitive(Primitive::Bool); 256]);
    let requested = requirement(101, subject);
    let assumptions = vec![requested.clone()];
    let world = World {
        index: &index,
        implementations: &[],
        traits: &[],
        assumptions: &assumptions,
        context: &context,
    };
    assert!(
        world
            .given(
                &requested,
                Budget {
                    steps: 32,
                    ..Default::default()
                }
            )
            .is_err_and(|error| matches!(error, nir::NominalAdmissionError::Budget))
    );
    assert!(
        world
            .given(&requested, Budget::default())
            .unwrap()
            .is_some()
    );
    assert!(
        world
            .given(
                &requested,
                Budget {
                    steps: 0,
                    ..Default::default()
                }
            )
            .is_err_and(|error| matches!(error, nir::NominalAdmissionError::Budget))
    );
    assert!(
        world
            .given(&requested, Budget::default())
            .unwrap()
            .is_some()
    );
}

#[test]
fn metered_assumption_lookup_returns_consumed_allowance_on_failure_and_success() {
    let index = Index::default();
    let context = context();
    let requested = requirement(101, Term::Tuple(vec![Term::Primitive(Primitive::Bool); 64]));
    let assumptions = vec![requested.clone()];
    let world = World {
        index: &index,
        implementations: &[],
        traits: &[],
        assumptions: &assumptions,
        context: &context,
    };
    let mut small = Budget {
        steps: 16,
        ..Default::default()
    };
    assert!(
        world
            .given_metered(&requested, &mut small)
            .is_err_and(|error| matches!(error, nir::NominalAdmissionError::Budget))
    );
    assert!(small.steps < 16);
    let mut enough = Budget::default();
    let before = enough.steps;
    assert!(
        world
            .given_metered(&requested, &mut enough)
            .unwrap()
            .is_some()
    );
    assert!(enough.steps + 64 < before);
    assert!(
        world
            .given_metered(&requested, &mut enough)
            .unwrap()
            .is_some()
    );
}

#[test]
fn candidate_search_returns_budget_even_when_no_implementation_matches() {
    let index = Index::default();
    let subject = Term::Tuple(vec![Term::Primitive(Primitive::Bool); 128]);
    let mut small = Budget {
        steps: 8,
        ..Default::default()
    };
    assert!(
        index
            .candidates_metered(Some(id(101)), &[], &subject, &mut small)
            .is_err()
    );
    assert!(small.steps < 8);
    let mut enough = Budget::default();
    let before = enough.steps;
    let result = index
        .candidates_metered(Some(id(101)), &[], &subject, &mut enough)
        .unwrap();
    assert!(result.declarations().is_empty());
    assert!(enough.steps + 128 < before);
}

#[test]
fn repeated_boolean_contract_checks_share_budget_and_keep_world_local_facts() {
    use nir::program::TypePredicate as Q;
    use type_resolution::ptc::{Formula, Outcome};
    let ctx = context();
    let traits = vec![marker(100), marker(101), marker(102)];
    let subject = Term::Primitive(Primitive::Bool);
    let implementations = vec![implementation(20, 101, subject.clone(), vec![], vec![])];
    let ix = index(&implementations);
    let world = World {
        index: &ix,
        implementations: &implementations,
        traits: &traits,
        assumptions: &[],
        context: &ctx,
    };
    let malformed = type_resolution::ptc::TraitQuery::from(&requirement(999, subject.clone()));
    let mut rejected = Budget::default();
    let before = rejected.steps;
    assert!(
        world
            .decide_trait_metered(&malformed, &mut rejected)
            .is_err()
    );
    assert!(rejected.steps < before);
    let predicate = Q::And(vec![
        Q::Not(Box::new(Q::Trait(requirement(100, subject.clone())))),
        Q::Or(vec![
            Q::Trait(requirement(101, subject.clone())),
            Q::Trait(requirement(102, subject)),
        ]),
    ]);
    let mut exhausted = Budget {
        steps: 0,
        ..Budget::default()
    };
    assert!(!matches!(
        world.check_predicate_metered(&predicate, &mut exhausted),
        Ok(Outcome::Entailed(_))
    ));
    assert_eq!(exhausted.steps, 0);
    let mut shared = Budget::default();
    for _ in 0..4 {
        let before = shared.steps;
        assert!(matches!(
            world.check_predicate_metered(&predicate, &mut shared),
            Ok(Outcome::Entailed(_))
        ));
        assert!(shared.steps < before);
    }
    // Returned implementation facts must not become unconditional PTC facts.
    assert!(!matches!(
        ctx.prove(Formula::from_predicate(&predicate), Budget::default()),
        Outcome::Entailed(_)
    ));
    let absent =
        type_resolution::ptc::TraitQuery::from(&requirement(100, Term::Primitive(Primitive::Bool)));
    assert_eq!(
        world
            .decide_trait(&absent, Budget::default())
            .unwrap()
            .truth(),
        type_resolution::capabilities::TraitTruth::Excluded
    );
}

#[test]
fn generic_disjunctive_type_premises_discharge_matching_requirements() {
    use nir::program::TypePredicate as Q;
    use type_resolution::ptc::Formula;
    let subject = Term::Rigid {
        declaration: id(2),
        sort: Sort::Type,
    };
    let condition = |t: Term| {
        Q::Or(vec![
            Q::Equal(t.clone(), Term::Primitive(Primitive::F16)),
            Q::Equal(t, Term::Primitive(Primitive::Bf16)),
        ])
    };
    let mut candidate = implementation(20, 100, var(), vec![id(1)], vec![]);
    candidate.requirements = vec![condition(var())];
    let implementations = vec![
        candidate,
        implementation(21, 100, Term::Pointer(Box::new(var())), vec![id(1)], vec![]),
        implementation(
            22,
            100,
            Term::Optional(Box::new(var())),
            vec![id(1)],
            vec![],
        ),
    ];
    let ix = index(&implementations);
    let traits = vec![marker(100)];
    let mut ctx = context();
    ctx.assume(Formula::from_predicate(&condition(subject.clone())));
    let world = World {
        index: &ix,
        implementations: &implementations,
        traits: &traits,
        assumptions: &[],
        context: &ctx,
    };
    let resolution = world
        .resolve_instance(&id(100).into(), &subject, Budget::default())
        .unwrap();
    assert!(
        matches!(resolution.types.selection, Selection::Selected(_)),
        "{}",
        resolution.types
    );
    for (wrapped, expected) in [
        (Term::Pointer(Box::new(subject.clone())), id(21)),
        (Term::Optional(Box::new(subject.clone())), id(22)),
    ] {
        let resolution = world
            .resolve_instance(&id(100).into(), &wrapped, Budget::default())
            .unwrap();
        assert_eq!(
            resolution.types.selection,
            Selection::Selected(expected),
            "{}",
            resolution.types
        );
    }
}

#[test]
fn trait_proposition_reuses_budget_after_formation_and_given_checks() {
    use nir::program::TypePredicate as Q;
    use type_resolution::ptc::{Formula, Outcome};
    let rigid = |n| Term::Rigid {
        declaration: id(n),
        sort: Sort::Type,
    };
    let mut scalar = implementation(20, 100, var(), vec![id(1)], vec![]);
    scalar.requirements = vec![Q::Or(vec![
        Q::Equal(var(), Term::Primitive(Primitive::F16)),
        Q::Equal(var(), Term::Primitive(Primitive::Bf16)),
    ])];
    let wrapper = |error, value| Term::ErrorResult {
        errors: vec![error],
        result: Box::new(value),
    };
    let mut implementations = vec![
        scalar,
        implementation(
            21,
            100,
            wrapper(rigid(2), var()),
            vec![id(1), id(2)],
            vec![requirement(100, rigid(2)), requirement(100, var())],
        ),
    ];
    for (n, p) in [
        Primitive::Bool,
        Primitive::Char,
        Primitive::Symbol,
        Primitive::U8,
        Primitive::U16,
        Primitive::U32,
        Primitive::U64,
        Primitive::U128,
        Primitive::I8,
        Primitive::I16,
        Primitive::I32,
        Primitive::I64,
        Primitive::I128,
        Primitive::Usize,
        Primitive::Isize,
        Primitive::F32,
        Primitive::F64,
    ]
    .into_iter()
    .enumerate()
    {
        implementations.push(implementation(
            200 + n as u32,
            100,
            Term::Primitive(p),
            vec![],
            vec![],
        ));
    }
    let ix = index(&implementations);
    let traits = vec![marker(100), marker(101), marker(102)];
    let assumptions: Vec<_> = [100, 101, 102]
        .into_iter()
        .flat_map(|interface| {
            [
                requirement(interface, rigid(11)),
                requirement(interface, rigid(12)),
            ]
        })
        .collect();
    let mut ctx = context();
    for assumption in &assumptions {
        ctx.assume(Formula::from_predicate(&Q::Trait(assumption.clone())));
    }
    let world = World {
        index: &ix,
        implementations: &implementations,
        traits: &traits,
        assumptions: &assumptions,
        context: &ctx,
    };
    let query = requirement(100, wrapper(rigid(11), rigid(12)));
    let predicate = Q::Trait(query.clone());
    assert!(matches!(
        world
            .resolve_instance(
                &query.interface_instance(),
                &query.subject,
                Budget::default()
            )
            .unwrap()
            .types
            .selection,
        Selection::Selected(_)
    ));
    let mut exhausted = Budget {
        steps: 1,
        ..Budget::default()
    };
    assert!(!matches!(
        world.check_predicate_metered(&predicate, &mut exhausted),
        Ok(Outcome::Entailed(_))
    ));
    assert!(exhausted.steps <= 1);
    for _ in 0..2 {
        let mut budget = Budget::default();
        let before = budget.steps;
        let outcome = world
            .check_predicate_metered(&predicate, &mut budget)
            .unwrap();
        assert!(matches!(outcome, Outcome::Entailed(_)), "{outcome:?}");
        assert!(budget.steps < before && budget.steps > 0);
        let proposition = world
            .prove_proposition(Formula::from_predicate(&predicate), Budget::default())
            .unwrap();
        proposition.verify(&world, Budget::default()).unwrap();
    }
}

#[test]
fn type_valued_indices_cross_parent_and_capability_edges_without_losing_identity() {
    use nir::program::{InterfaceInstance, TypePredicate};
    use type_resolution::capabilities::{capability_sources, subject_requirements};
    let rigid = |local, sort| Term::Rigid {
        declaration: id(local),
        sort,
    };
    let type_domain = Term::Primitive(Primitive::Type);
    let boolean = Term::Primitive(Primitive::Bool);
    let mut child = marker(100);
    child.type_parameters = vec![id(101), id(102)];
    child.parameter_types = vec![type_domain.clone(), rigid(101, Sort::Type)];
    child.parents.push(InterfaceInstance {
        declaration: id(110),
        arguments: vec![rigid(101, Sort::Type), rigid(102, Sort::ComptimeValue)],
    });
    let mut parent = marker(110);
    parent.type_parameters = vec![id(111), id(112)];
    parent.parameter_types = vec![type_domain.clone(), rigid(111, Sort::Type)];
    parent
        .capabilities
        .push(requirement(120, boxed(rigid(112, Sort::ComptimeValue))));
    parent.requirements.push(TypePredicate::Equal(
        rigid(110, Sort::Type),
        boxed(rigid(112, Sort::ComptimeValue)),
    ));
    let traits = vec![child, parent, marker(120)];
    let assumption = TraitRequirement {
        interface: id(100),
        subject: boxed(boolean.clone()),
        interface_arguments: vec![type_domain, boolean.clone()],
    };
    let assumptions = vec![assumption.clone()];
    let mut context = Context::default();
    context.admit_nominal_constructor(id(10));
    let index = Index::default();
    let world = World {
        index: &index,
        implementations: &[],
        traits: &traits,
        assumptions: &assumptions,
        context: &context,
    };
    let requested = requirement(120, boxed(boolean.clone()));
    assert_eq!(
        world
            .given_in_scope(&requested, Budget::default(), &[])
            .unwrap(),
        Some(assumption.clone())
    );
    assert!(
        world
            .given_in_scope(
                &requirement(120, boxed(Term::Primitive(Primitive::Char))),
                Budget::default(),
                &[]
            )
            .unwrap()
            .is_none()
    );
    let sources = capability_sources(&traits, &assumptions, Budget::default()).unwrap();
    assert!(sources.contains(&requested));
    assert!(!sources.iter().any(|source| source.interface == id(110)));
    let facts = subject_requirements(&traits, &assumptions, Budget::default());
    assert!(facts.contains(&TypePredicate::Equal(
        boxed(boolean.clone()),
        boxed(boolean)
    )));
    let tiny = Budget {
        steps: 1,
        ..Budget::default()
    };
    assert!(capability_sources(&traits, &assumptions, tiny).is_err());
    assert!(
        world
            .given_in_scope(&requested, tiny, &[])
            .is_err_and(|error| matches!(error, nir::NominalAdmissionError::Budget))
    );
    assert_eq!(
        world
            .given_in_scope(&requested, Budget::default(), &[])
            .unwrap(),
        Some(assumption)
    );
}

#[test]
fn type_valued_impl_inputs_specialize_capability_qualification() {
    let value = Term::Rigid {
        declaration: id(2),
        sort: Sort::ComptimeValue,
    };
    let family = |a, b| Term::Nominal {
        declaration: id(10),
        arguments: vec![a, b],
    };
    let boolean = Term::Primitive(Primitive::Bool);
    let subject = family(Term::Primitive(Primitive::Type), boolean.clone());
    let mut candidate = implementation(
        20,
        100,
        family(var(), value.clone()),
        vec![id(1), id(2)],
        vec![requirement(101, boxed(value))],
    );
    candidate.parameter_types = vec![Term::Primitive(Primitive::Type), var()];
    let context = context();
    for available in [false, true] {
        let mut implementations = vec![candidate.clone()];
        if available {
            implementations.push(implementation(
                21,
                101,
                boxed(boolean.clone()),
                vec![],
                vec![],
            ));
        }
        let index = index(&implementations);
        let world = World {
            index: &index,
            implementations: &implementations,
            traits: &[],
            assumptions: &[],
            context: &context,
        };
        let result = world.resolve(id(100), &subject, Budget::default()).unwrap();
        assert_eq!(
            matches!(result.types.selection, Selection::Selected(selected) if selected == id(20)),
            available
        );
        if available {
            assert!(matches!(
                result.qualifications[&id(20)],
                Qualification::Proved(_)
            ));
        }
    }
}

#[test]
fn comptime_parent_phase_can_follow_only_comptime_only_given_evidence() {
    let subject = boxed(Term::Primitive(Primitive::Bool));
    for parent_comptime_only in [false, true] {
        let mut parent = marker(101);
        parent.comptime_only = parent_comptime_only;
        let mut child = marker(102);
        child.comptime_only = true;
        child.parents.push(id(101).into());
        let traits = vec![parent, child];
        let mut child_impl = implementation(20, 102, subject.clone(), vec![], vec![]);
        child_impl.phases.comptime = true;
        let implementations = vec![child_impl];
        let index = index(&implementations);
        let context = context();
        let assumptions = vec![requirement(101, subject.clone())];
        let world = World {
            index: &index,
            implementations: &implementations,
            traits: &traits,
            assumptions: &assumptions,
            context: &context,
        };
        let result = world.resolve(id(102), &subject, Budget::default());
        if parent_comptime_only {
            assert_eq!(result.unwrap().types.selection, Selection::Selected(id(20)));
        } else {
            assert!(
                matches!(result, Err(nir::NominalAdmissionError::Unresolved(reason)) if reason.contains("phase remains unresolved"))
            );
        }
    }
}

#[test]
fn inherent_qualification_retains_caller_scope_and_rejects_scope_changes() {
    use type_resolution::implementation::{InherentHead, InherentIndex};
    let ctx = context().theory_in_scope(vec![vec![Sort::Type]]);
    let mut candidate = implementation(20, 100, boxed(var()), vec![id(1)], vec![]);
    candidate.interface = None;
    let mut inherent = InherentIndex::default();
    inherent
        .insert(
            InherentHead {
                declaration: candidate.declaration,
                type_parameters: candidate.type_parameters.clone(),
                parameter_types: candidate.parameter_types.clone(),
                subject: candidate.subject.clone(),
                requirements: vec![],
                members: ["read".into()].into_iter().collect(),
            },
            Budget::default(),
        )
        .unwrap();
    let implementations = vec![candidate];
    let ix = Index::default();
    let world = World {
        index: &ix,
        implementations: &implementations,
        traits: &[],
        assumptions: &[],
        context: &ctx,
    };
    let subject = boxed(Term::Bound {
        depth: 0,
        index: 0,
        sort: Sort::Type,
    });
    let result = world
        .resolve_inherent(&inherent, "read", &subject, Budget::default())
        .unwrap();
    assert_eq!(result.types.selection, Selection::Selected(id(20)));
    world
        .verify_inherent_evidence(&inherent, "read", &subject, &result, Budget::default())
        .unwrap();
    let closed = context();
    let other = World {
        context: &closed,
        ..world
    };
    assert!(
        other
            .verify_inherent_evidence(&inherent, "read", &subject, &result, Budget::default())
            .is_err()
    );
}

#[test]
fn incomplete_given_equality_does_not_authorize_absence_or_exclusion() {
    use nir::NominalAdmissionError;
    use type_resolution::ptc::Formula;
    let nf = |ty| Term::NormalForm {
        head: id(90),
        sort: Sort::Type,
        arguments: vec![ty],
    };
    let left = var();
    let right = Term::Rigid {
        declaration: id(2),
        sort: Sort::Type,
    };
    let requested = requirement(101, nf(right.clone()));
    let given = requirement(101, nf(left.clone()));
    let implementations = [implementation(
        20,
        100,
        Term::Primitive(Primitive::Bool),
        vec![],
        vec![requested.clone()],
    )];
    let ix = index(&implementations);
    let traits = [marker(100), marker(101)];
    let mut context = Context::default();
    context.assume(Formula::equal(left, right));
    let assumptions = [given];
    let world = World {
        index: &ix,
        implementations: &implementations,
        traits: &traits,
        assumptions: &assumptions,
        context: &context,
    };
    assert!(matches!(
        world.given(&requested, Budget::default()),
        Err(NominalAdmissionError::Unresolved(_))
    ));
    let result = world
        .resolve(
            id(100),
            &Term::Primitive(Primitive::Bool),
            Budget::default(),
        )
        .unwrap();
    assert!(
        matches!(result.qualifications.get(&id(20)), Some(Qualification::Pending(reason)) if matches!(reason, nir::NominalAdmissionError::Unresolved(message) if message.contains("UnvalidatedModel")))
    );
    assert!(!matches!(
        result.types.selection,
        Selection::NoApplicableCandidate
    ));

    // An earlier incomplete premise does not hide an independent exact witness.
    let complete = [assumptions[0].clone(), requested.clone()];
    let with_witness = World {
        assumptions: &complete,
        ..world
    };
    assert_eq!(
        with_witness.given(&requested, Budget::default()),
        Ok(Some(requested.clone()))
    );
    let mut recovered = context.clone();
    recovered.admit_congruence(id(90));
    let resolved = World {
        context: &recovered,
        ..world
    };
    assert_eq!(
        resolved.given(&requested, Budget::default()),
        Ok(Some(assumptions[0].clone()))
    );
    assert_eq!(
        resolved
            .resolve(
                id(100),
                &Term::Primitive(Primitive::Bool),
                Budget::default()
            )
            .unwrap()
            .types
            .selection,
        Selection::Selected(id(20))
    );
}

#[test]
fn given_lookup_preserves_invalid_inherited_edges_and_recovers() {
    let mut child = marker(100);
    child.capabilities.push(requirement(
        101,
        Term::Bound {
            depth: 0,
            index: 9,
            sort: Sort::Type,
        },
    ));
    let traits = [child, marker(101)];
    let requested = requirement(101, Term::Primitive(Primitive::Bool));
    let assumptions = [requirement(100, requested.subject.clone())];
    let index = Index::default();
    let context = Context::default();
    let world = World {
        index: &index,
        implementations: &[],
        traits: &traits,
        assumptions: &assumptions,
        context: &context,
    };
    assert!(matches!(
        world.given(&requested, Budget::default()),
        Err(nir::NominalAdmissionError::Invalid(_))
    ));
    let independent = [assumptions[0].clone(), requested.clone()];
    assert_eq!(
        World {
            assumptions: &independent,
            ..world
        }
        .given(&requested, Budget::default()),
        Ok(Some(requested.clone()))
    );
    assert_eq!(
        World {
            assumptions: &[],
            ..world
        }
        .given(&requested, Budget::default()),
        Ok(None)
    );
}

#[test]
fn given_lookup_compares_type_leaves_inside_ordinary_record_values() {
    use type_resolution::ptc::Formula;
    let boolean = Term::Primitive(Primitive::Bool);
    let a = var();
    let b = Term::Rigid {
        declaration: id(2),
        sort: Sort::Type,
    };
    let mut contract = marker(100);
    contract.type_parameters = vec![id(3)];
    contract.parameter_types = vec![Term::Record(vec![
        ("tag".into(), boolean.clone()),
        ("ty".into(), Term::Primitive(Primitive::Type)),
    ])];
    let query = |tag, ty| TraitRequirement {
        interface: id(100),
        subject: boolean.clone(),
        interface_arguments: vec![Term::RecordValue(vec![
            ("tag".into(), Term::Bool(tag)),
            ("ty".into(), ty),
        ])],
    };
    let assumptions = [query(false, a.clone())];
    let traits = [contract];
    let index = Index::default();
    let mut context = Context::default();
    context.assume(Formula::equal(a, b.clone()));
    let world = World {
        index: &index,
        implementations: &[],
        traits: &traits,
        assumptions: &assumptions,
        context: &context,
    };
    assert_eq!(
        world.given(&query(false, b.clone()), Budget::default()),
        Ok(Some(assumptions[0].clone()))
    );
    assert_eq!(world.given(&query(true, b), Budget::default()), Ok(None));
}

#[test]
fn trait_query_failures_keep_budget_invalid_and_unresolved_distinct() {
    use nir::{NominalAdmissionError as E, program::TypePredicate as Q};
    use type_resolution::capabilities::TraitTruth;
    let ctx = context();
    let mut contract = marker(100);
    contract.requirements = vec![Q::Bool(true); 16];
    let traits = [contract];
    let ix = index(&[]);
    let world = World {
        index: &ix,
        implementations: &[],
        traits: &traits,
        assumptions: &[],
        context: &ctx,
    };
    let query = trait_query(100, Term::Primitive(Primitive::Bool));
    let mut full = Budget::default();
    assert_eq!(
        world
            .decide_trait_metered(&query, &mut full)
            .unwrap()
            .truth(),
        TraitTruth::Excluded
    );
    let cost = Budget::default().steps - full.steps;
    assert!(cost > 16);
    for steps in 0..=cost + 8 {
        let mut budget = Budget {
            steps,
            ..Budget::default()
        };
        match world.decide_trait_metered(&query, &mut budget) {
            Err(E::Budget) => {}
            Ok(decision) => assert_eq!(decision.truth(), TraitTruth::Excluded),
            Err(other) => panic!("budget {steps} misclassified: {other:?}"),
        }
        assert!(budget.steps <= steps);
    }
    let malformed = trait_query(100, Term::Bool(true));
    assert!(matches!(
        world.decide_trait(&malformed, Budget::default()),
        Err(E::Invalid(_))
    ));
    let unformed = [Trait {
        requirements: vec![Q::Equal(var(), Term::Primitive(Primitive::Bool))],
        ..traits[0].clone()
    }];
    let unresolved = World {
        traits: &unformed,
        ..world
    };
    assert!(matches!(
        unresolved.decide_trait(&query, Budget::default()),
        Err(E::Unresolved(_))
    ));
    assert_eq!(
        world
            .decide_trait(&query, Budget::default())
            .unwrap()
            .truth(),
        TraitTruth::Excluded
    );
}

#[test]
fn incomplete_world_queries_cannot_refute_but_independent_evidence_can_prove() {
    use nir::program::TypePredicate as Q;
    use type_resolution::ptc::{Formula as F, Outcome, UnknownReason};
    let ctx = context();
    let subject = Term::Primitive(Primitive::Bool);
    let mut recursive = implementation(20, 100, subject.clone(), vec![], vec![]);
    recursive.requirements = vec![Q::Trait(requirement(100, subject.clone()))];
    let implementations = [
        recursive,
        implementation(21, 101, subject.clone(), vec![], vec![]),
    ];
    let mut unformed = marker(102);
    unformed.requirements = vec![Q::Equal(var(), subject.clone())];
    let traits = [marker(100), marker(101), unformed];
    let ix = index(&implementations);
    let world = World {
        index: &ix,
        implementations: &implementations,
        traits: &traits,
        assumptions: &[],
        context: &ctx,
    };
    // Cover both a successful query with Pending selection and a failed
    // formation query. Neither supplies a closed-world negative fact.
    for interface in [100, 102] {
        let atom = F::Trait(trait_query(interface, subject.clone()));
        for goal in [atom.clone(), atom.clone().negate()] {
            let result = world.prove_proposition(goal, Budget::default()).unwrap();
            assert!(
                matches!(
                    result.outcome(),
                    Outcome::Unknown {
                        reason: UnknownReason::UnresolvedPrerequisite(_),
                        ..
                    }
                ),
                "{result}"
            );
            assert!(result.verify(&world, Budget::default()).is_err());
        }
        for reverse in [false, true] {
            let mut alternatives = vec![atom.clone(), F::Trait(trait_query(101, subject.clone()))];
            if reverse {
                alternatives.reverse();
            }
            let proof = world
                .prove_proposition(F::Or(alternatives), Budget::default())
                .unwrap();
            assert!(matches!(proof.outcome(), Outcome::Entailed(_)), "{proof}");
            proof.verify(&world, Budget::default()).unwrap();
        }
    }
    // Once the final proof runs out of fuel, earlier semantic Pending must
    // not turn the resource failure into an apparently well-funded unknown.
    for steps in 0..200 {
        let mut budget = Budget {
            steps,
            ..Budget::default()
        };
        let result = world
            .prove_proposition_metered(F::Trait(trait_query(100, subject.clone())), &mut budget);
        if budget.steps == 0 {
            match result {
                Err(nir::NominalAdmissionError::Budget) => {}
                Ok(result) => assert!(
                    matches!(
                        result.outcome(),
                        Outcome::Unknown {
                            reason: UnknownReason::BudgetExhausted,
                            ..
                        }
                    ),
                    "steps {steps}: {result}"
                ),
                Err(error) => panic!("steps {steps}: {error:?}"),
            }
        }
    }
    // A completed negative query remains usable and never mutates the caller.
    let absent = F::Trait(trait_query(101, Term::Primitive(Primitive::Char)));
    let proof = world
        .prove_proposition(absent.clone().negate(), Budget::default())
        .unwrap();
    assert!(matches!(proof.outcome(), Outcome::Entailed(_)));
    proof.verify(&world, Budget::default()).unwrap();
    assert!(!matches!(
        ctx.prove(absent.negate(), Budget::default()),
        Outcome::Entailed(_)
    ));
}

#[test]
fn nested_qualification_depth_failure_is_resource_failure_and_retryable() {
    use nir::NominalAdmissionError as Failure;
    use type_resolution::capabilities::TraitTruth;
    use type_resolution::ptc::TraitQuery;
    let subject = Term::Primitive(Primitive::Bool);
    let implementations: Vec<_> = (0..12)
        .map(|level| {
            implementation(
                400 + level,
                500 + level,
                subject.clone(),
                vec![],
                if level == 11 {
                    vec![]
                } else {
                    vec![requirement(501 + level, subject.clone())]
                },
            )
        })
        .collect();
    let traits: Vec<_> = (500..512).map(marker).collect();
    let index = index(&implementations);
    let context = context();
    let world = World {
        index: &index,
        implementations: &implementations,
        traits: &traits,
        assumptions: &[],
        context: &context,
    };
    let query = TraitQuery {
        interface: id(500),
        subject: subject.clone(),
        arguments: vec![],
    };
    for depth in [4, 6, 8] {
        let mut budget = Budget {
            max_depth: depth,
            ..Budget::default()
        };
        assert!(matches!(
            world.resolve_instance_metered(&id(500).into(), &subject, &mut budget),
            Err(Failure::Budget)
        ));
        assert!(budget.steps > 0 && budget.steps < Budget::default().steps);
        let mut budget = Budget {
            max_depth: depth,
            ..Budget::default()
        };
        assert!(matches!(
            world.decide_trait_metered(&query, &mut budget),
            Err(Failure::Budget)
        ));
    }
    for _ in 0..2 {
        let resolution = world.resolve(id(500), &subject, Budget::default()).unwrap();
        assert_eq!(resolution.types.selection, Selection::Selected(id(400)));
        assert!(matches!(
            resolution.qualifications[&id(400)],
            Qualification::Proved(_)
        ));
        assert_eq!(
            world
                .decide_trait(&query, Budget::default())
                .unwrap()
                .truth(),
            TraitTruth::Established
        );
    }
}

#[test]
fn qualification_failure_priority_preserves_independent_exclusions() {
    use nir::NominalAdmissionError as Failure;
    let subject = Term::Primitive(Primitive::Bool);
    for reverse in [false, true] {
        for include_absent in [false, true] {
            let mut requirements = vec![
                requirement(500, subject.clone()),
                requirement(600, subject.clone()),
            ];
            if include_absent {
                requirements.push(requirement(700, subject.clone()));
            }
            if reverse {
                requirements.reverse();
            }
            let mut implementations = vec![
                implementation(20, 100, subject.clone(), vec![], requirements),
                implementation(
                    30,
                    600,
                    subject.clone(),
                    vec![],
                    vec![requirement(600, subject.clone())],
                ),
            ];
            implementations.extend((0..12).map(|level| {
                implementation(
                    400 + level,
                    500 + level,
                    subject.clone(),
                    vec![],
                    if level == 11 {
                        vec![]
                    } else {
                        vec![requirement(501 + level, subject.clone())]
                    },
                )
            }));
            let index = index(&implementations);
            let context = context();
            let world = World {
                index: &index,
                implementations: &implementations,
                traits: &[],
                assumptions: &[],
                context: &context,
            };
            let result = world.resolve(
                id(100),
                &subject,
                Budget {
                    max_depth: 8,
                    ..Budget::default()
                },
            );
            if include_absent {
                let result = result.unwrap();
                assert_eq!(result.types.selection, Selection::NoApplicableCandidate);
                assert!(matches!(
                    result.qualifications[&id(20)],
                    Qualification::Excluded(_)
                ));
            } else {
                assert!(matches!(result, Err(Failure::Budget)), "{result:?}");
            }
        }
    }
}

#[test]
fn qualification_closed_inputs_use_proved_views_without_expanding_heads() {
    use type_resolution::ptc::Formula;
    let subject = Term::Primitive(Primitive::Bool);
    let implementations = [implementation(
        20,
        100,
        subject.clone(),
        vec![],
        vec![requirement(101, var())],
    )];
    let index = index(&implementations);
    let mut context = context();
    let resolve = |context: &Context| {
        World {
            index: &index,
            implementations: &implementations,
            traits: &[],
            assumptions: &[],
            context,
        }
        .resolve(id(100), &subject, Budget::default())
        .unwrap()
    };
    let open = resolve(&context);
    assert!(matches!(open.types.selection, Selection::Deferred { .. }));
    assert!(matches!(
        open.qualifications[&id(20)],
        Qualification::Pending(nir::NominalAdmissionError::Unresolved(_))
    ));
    context.assume(Formula::equal(var(), Term::Primitive(Primitive::Char)));
    let closed = resolve(&context);
    assert_eq!(closed.types.selection, Selection::NoApplicableCandidate);
    assert!(matches!(
        closed.qualifications[&id(20)],
        Qualification::Excluded(_)
    ));
}

#[test]
fn malformed_qualification_is_invalid_and_never_negative_evidence() {
    use nir::NominalAdmissionError as Failure;
    let subject = Term::Primitive(Primitive::Bool);
    let mut implementations = [implementation(
        20,
        100,
        subject.clone(),
        vec![],
        vec![requirement(101, Term::Bool(true))],
    )];
    let index = index(&implementations);
    let context = context();
    let resolve = |implementations: &[Implementation]| {
        World {
            index: &index,
            implementations,
            traits: &[],
            assumptions: &[],
            context: &context,
        }
        .resolve(id(100), &subject, Budget::default())
    };
    assert!(matches!(
        resolve(&implementations),
        Err(Failure::Invalid(_))
    ));
    implementations[0].capabilities.clear();
    assert_eq!(
        resolve(&implementations).unwrap().types.selection,
        Selection::Selected(id(20))
    );
    assert_eq!(
        Qualification::Pending(Failure::Budget).to_string(),
        "(capability-pending (budget-exhausted))"
    );
}

#[test]
fn independent_exclusion_survives_a_failed_capability_payload_reservation() {
    use nir::NominalAdmissionError as Failure;
    let subject = Term::Primitive(Primitive::Bool);
    for reverse in [false, true] {
        for include_absent in [false, true] {
            let mut requirements = vec![requirement(
                101,
                Term::Record(vec![("wide".repeat(50_000), subject.clone())]),
            )];
            if include_absent {
                requirements.push(requirement(102, subject.clone()));
            }
            if reverse {
                requirements.reverse();
            }
            let implementations = [implementation(
                20,
                100,
                subject.clone(),
                vec![],
                requirements,
            )];
            let index = index(&implementations);
            let context = context();
            let world = World {
                index: &index,
                implementations: &implementations,
                traits: &[],
                assumptions: &[],
                context: &context,
            };
            let mut budget = Budget::default();
            let result = world.resolve_instance_metered(&id(100).into(), &subject, &mut budget);
            assert!(budget.steps > 0 && budget.steps < Budget::default().steps);
            if include_absent {
                let result = result.unwrap();
                assert_eq!(result.types.selection, Selection::NoApplicableCandidate);
                assert!(matches!(
                    result.qualifications[&id(20)],
                    Qualification::Excluded(_)
                ));
            } else {
                assert!(matches!(result, Err(Failure::Budget)));
            }
        }
    }
}
