use nir::{DeclarationId as Id, Primitive, Sort, Term};
use type_resolution::implementation::{ImplementationHead, Index, InherentHead, InherentIndex};
use type_resolution::ptc::{Budget, Context};

fn head(package: u64, local: u32, subject: Term) -> ImplementationHead {
    ImplementationHead {
        parameter_types: vec![],
        declaration: Id { package, local },
        interface: Some(Id {
            package: 100,
            local: 1,
        }),
        subject,
        type_parameters: vec![],
        interface_arguments: vec![],
        requirements: vec![],
    }
}
fn object() -> Term {
    Term::Primitive(Primitive::Object)
}

#[test]
fn trusted_packages_extend_builtin_subjects_without_owning_foreign_heads() {
    let subjects = [
        object(),
        Term::Optional(Box::new(object())),
        Term::ErrorResult {
            errors: vec![],
            result: Box::new(object()),
        },
        Term::Pointer(Box::new(object())),
    ];
    for subject in subjects {
        assert!(
            Index::default()
                .insert(head(1, 1, subject.clone()), Budget::default())
                .is_err()
        );
        let mut trusted = Index::for_builtin_packages(None, [1, 2, 3, 4, 5, 6]);
        trusted
            .insert(head(1, 1, subject.clone()), Budget::default())
            .unwrap();
        assert!(
            trusted
                .insert(head(7, 2, subject), Budget::default())
                .is_err()
        );
    }
    for subject in [
        Term::Nominal {
            declaration: Id {
                package: 99,
                local: 1,
            },
            arguments: vec![object()],
        },
        Term::NormalForm {
            head: Id {
                package: 99,
                local: 1,
            },
            sort: Sort::Type,
            arguments: vec![object()],
        },
        Term::Rigid {
            declaration: Id {
                package: 1,
                local: 2,
            },
            sort: Sort::Type,
        },
    ] {
        let mut trusted = Index::for_builtin_packages(None, [1]);
        assert!(
            trusted
                .insert(head(1, 1, subject.clone()), Budget::default())
                .is_err()
        );
        assert!(
            InherentIndex::for_builtin_packages(None, [1])
                .check_ownership(&subject, 1)
                .is_err()
        );
    }
}

#[test]
fn builtin_grants_do_not_bypass_duplicate_coherence_or_budget_checks() {
    let mut index = Index::for_builtin_packages(None, [1, 2]);
    index
        .insert(head(1, 1, object()), Budget::default())
        .unwrap();
    let revision = index.revision();
    assert!(
        index
            .insert(head(1, 1, object()), Budget::default())
            .is_err()
    );
    assert_eq!(index.revision(), revision);
    assert!(
        index
            .insert(
                head(2, 2, object()),
                Budget {
                    steps: 0,
                    ..Budget::default()
                }
            )
            .is_err()
    );
    assert_eq!(index.revision(), revision);
    index
        .insert(head(2, 2, object()), Budget::default())
        .unwrap();
    assert!(
        !index
            .check_type_coherence(&Context::default(), Budget::default())
            .is_proved()
    );
    assert!(index.to_string().contains("(builtin-packages 1 2)"));
}

#[test]
fn inherent_admission_uses_immutable_package_grants() {
    let make = |package| InherentHead {
        parameter_types: vec![],
        declaration: Id { package, local: 1 },
        subject: object(),
        type_parameters: vec![],
        requirements: vec![],
        members: ["length".into()].into(),
    };
    assert!(
        InherentIndex::default()
            .insert(make(1), Budget::default())
            .is_err()
    );
    let mut trusted = InherentIndex::for_builtin_packages(None, [6, 1, 6]);
    trusted.insert(make(1), Budget::default()).unwrap();
    assert!(trusted.insert(make(7), Budget::default()).is_err());
    assert!(trusted.insert(make(1), Budget::default()).is_err());
    assert!(trusted.to_string().contains("(builtin-packages 1 6)"));
    trusted.insert(make(6), Budget::default()).unwrap();
    assert!(
        trusted
            .check_coherence(&Context::default(), Budget::default())
            .is_err()
    );
}

#[test]
fn phase_admission_keeps_foreign_trait_identity_and_package_boundary() {
    let mut index = Index::for_builtin_packages(None, [1]);
    let runtime = index
        .insert_phase(
            head(1, 1, object()),
            hir::ImplementationPhase::Runtime,
            Budget::default(),
        )
        .unwrap();
    let comptime = index
        .insert_phase(
            head(1, 2, object()),
            hir::ImplementationPhase::Comptime,
            Budget::default(),
        )
        .unwrap();
    assert_eq!(runtime, comptime);
    assert!(
        index
            .insert_phase(
                head(7, 3, object()),
                hir::ImplementationPhase::Runtime,
                Budget::default()
            )
            .is_err()
    );
}
