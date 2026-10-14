use elaboration::implementation::{
    ImplementationHead, InherentHead, InherentIndex, Selection, TypeAssessment,
};
use elaboration::ptc::{Budget, Context, Formula};
use nir::program::TypePredicate;
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
fn boxed(t: Term) -> Term {
    Term::Nominal {
        declaration: id(10),
        arguments: vec![t],
    }
}
fn head(n: u32, subject: Term, parameters: Vec<Id>, names: &[&str]) -> InherentHead {
    InherentHead {
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
