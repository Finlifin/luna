use nir::deferred_signatures::{Annotation as A, Error, Input, Signature};
use nir::type_computations::{Input as Argument, Invocation};
use nir::{DeclarationId, Primitive, Sort, Term};
fn p(p: Primitive) -> Term {
    Term::Primitive(p)
}
fn computation(scope: Vec<Sort>, slot: usize) -> Invocation {
    Invocation::new(
        DeclarationId {
            package: 1,
            local: 1,
        },
        scope.clone(),
        vec![Argument {
            ty: p(Primitive::Type),
            value: Term::Bound {
                depth: 0,
                index: slot,
                sort: scope[slot],
            },
        }],
        &mut 1000,
    )
    .unwrap()
}
#[test]
fn deferred_signatures_keep_input_frames_and_share_only_explicit_computations() {
    let scope = vec![Sort::Type, Sort::Runtime];
    let signature = Signature::new(
        vec![
            Input {
                sort: Sort::Type,
                annotation: A::Known(p(Primitive::Type)),
            },
            Input {
                sort: Sort::Runtime,
                annotation: A::Computed(0),
            },
        ],
        A::Computed(0),
        vec![computation(scope, 0)],
        &mut 1000,
    )
    .unwrap();
    let opaque = Term::NormalForm {
        head: DeclarationId {
            package: 1,
            local: 2,
        },
        sort: Sort::Type,
        arguments: vec![p(Primitive::Bool)],
    };
    let resolved = signature.resolve(&[&opaque], &mut 1000).unwrap();
    let Term::Function { parameters, result } = resolved else {
        panic!("function")
    };
    assert_eq!(parameters[0].sort, Sort::Type);
    assert_eq!(parameters[1].ty, opaque);
    assert_eq!(*result, opaque);
    assert!(
        signature
            .to_string()
            .starts_with("(deferred-signature (inputs")
    );
    assert_eq!(signature.resolve(&[], &mut 1000), Err(Error::ResultCount));
    assert_eq!(signature.resolve(&[&opaque], &mut 0), Err(Error::Budget));
    assert!(matches!(
        signature.resolve(&[&Term::Bool(true)], &mut 1000),
        Err(Error::Scope(_))
    ));
}
#[test]
fn deferred_signatures_reject_forward_reads_and_invalid_annotation_scopes() {
    let type_input = || Input {
        sort: Sort::Type,
        annotation: A::Known(p(Primitive::Type)),
    };
    let runtime = || Input {
        sort: Sort::Runtime,
        annotation: A::Computed(0),
    };
    assert!(matches!(
        Signature::new(
            vec![runtime(), type_input()],
            A::Known(p(Primitive::Bool)),
            vec![computation(vec![Sort::Runtime, Sort::Type], 1)],
            &mut 1000
        ),
        Err(Error::ForwardInput { input: 1, .. })
    ));
    assert!(matches!(
        Signature::new(
            vec![type_input(), runtime()],
            A::Computed(0),
            vec![computation(vec![Sort::Type], 0)],
            &mut 1000
        ),
        Err(Error::InputFrame(0))
    ));
    assert!(matches!(
        Signature::new(vec![runtime()], A::Computed(0), vec![], &mut 1000),
        Err(Error::MissingComputation(0))
    ));
    assert!(matches!(
        Signature::new(
            vec![type_input()],
            A::Known(p(Primitive::Bool)),
            vec![computation(vec![Sort::Type], 0)],
            &mut 1000
        ),
        Err(Error::UnusedComputation(0))
    ));
    assert!(matches!(
        Signature::new(
            vec![Input {
                sort: Sort::Runtime,
                annotation: A::Known(Term::Bound {
                    depth: 0,
                    index: 0,
                    sort: Sort::Type
                })
            }],
            A::Known(p(Primitive::Bool)),
            vec![],
            &mut 1000
        ),
        Err(Error::Scope(_))
    ));
    let runtime_read = Invocation::new(
        DeclarationId {
            package: 1,
            local: 1,
        },
        vec![Sort::Runtime],
        vec![Argument {
            ty: p(Primitive::Bool),
            value: Term::Bound {
                depth: 0,
                index: 0,
                sort: Sort::Runtime,
            },
        }],
        &mut 1000,
    );
    assert!(matches!(
        runtime_read,
        Err(nir::type_computations::Error::RuntimeInput(0))
    ));
}

#[test]
fn known_signature_annotations_keep_dependent_binders() {
    let t = Term::Bound {
        depth: 0,
        index: 0,
        sort: Sort::Type,
    };
    let signature = Signature::new(
        vec![
            Input {
                sort: Sort::Type,
                annotation: A::Known(p(Primitive::Type)),
            },
            Input {
                sort: Sort::Runtime,
                annotation: A::Known(t.clone()),
            },
        ],
        A::Known(t.clone()),
        vec![],
        &mut 1000,
    )
    .unwrap();
    let resolved = signature.resolve(&[], &mut 1000).unwrap();
    resolved.check_scope(&[]).unwrap();
    let Term::Function { parameters, result } = resolved else {
        panic!("function")
    };
    assert_eq!(parameters[1].ty, t);
    assert_eq!(*result, t);
    assert!(matches!(
        Signature::new(
            vec![Input {
                sort: Sort::Type,
                annotation: A::Known(p(Primitive::Bool))
            }],
            A::Known(p(Primitive::Bool)),
            vec![],
            &mut 1000
        ),
        Err(Error::TypeParameterDomain(0))
    ));
    assert_eq!(
        Signature::new(vec![], A::Known(p(Primitive::Bool)), vec![], &mut 0),
        Err(Error::Budget)
    );
}

#[test]
fn saved_signature_budgets_count_labels_payloads_and_empty_effect_occurrences() {
    let head = DeclarationId {
        package: 1,
        local: 1,
    };
    let values = [
        Term::NormalForm {
            head,
            sort: Sort::Type,
            arguments: vec![Term::String("x".repeat(4096).into())],
        },
        Term::Record(vec![("field".repeat(1024), p(Primitive::Bool))]),
        Term::computation(
            vec![
                nir::effects::Operation {
                    declaration: head,
                    arguments: vec![]
                };
                4096
            ],
            p(Primitive::Bool),
        ),
    ];
    for value in values {
        assert_eq!(
            Signature::new(vec![], A::Known(value), vec![], &mut 100),
            Err(Error::Budget)
        );
    }
}
