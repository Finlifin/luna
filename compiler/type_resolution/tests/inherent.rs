use nir::program::TypePredicate;
use nir::{DeclarationId as Id, Primitive as P, Sort, Term};
use type_resolution::implementation::{
    ImplementationHead, InherentHead, InherentIndex, Selection, TypeAssessment,
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
