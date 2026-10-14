use elaboration::calls::Scheme;
use elaboration::ptc::{Budget, Context};
use nir::effects::Operation;
use nir::{DeclarationId, Parameter, Primitive as P, Sort, Term};
fn ty(p: P) -> Term {
    Term::Primitive(p)
}
fn variable(index: usize) -> Term {
    Term::Bound {
        depth: 1,
        index,
        sort: Sort::Type,
    }
}
fn op(local: u32, arguments: Vec<Term>) -> Operation {
    Operation {
        declaration: DeclarationId { package: 1, local },
        arguments,
    }
}
fn comp(effects: Vec<Operation>, result: Term) -> Term {
    Term::computation(effects, result)
}
fn scheme(types: usize, input: Term, output: Term) -> Scheme {
    Scheme {
        type_parameters: types,
        signature: Term::Function {
            parameters: vec![Parameter {
                sort: Sort::Runtime,
                ty: input,
            }],
            result: Box::new(output),
        },
    }
}

#[test]
fn effect_inference_extracts_results_and_unique_label_inputs_across_permutations() {
    let signature = scheme(
        2,
        comp(vec![op(1, vec![variable(0)]), op(2, vec![])], variable(1)),
        variable(0),
    );
    let actual = comp(vec![op(2, vec![]), op(1, vec![ty(P::Char)])], ty(P::Bool));
    let instance = signature
        .infer_types(&[actual], None, Budget::default())
        .unwrap()
        .finish()
        .unwrap();
    assert_eq!(instance.type_arguments, [ty(P::Char), ty(P::Bool)]);
    assert_eq!(instance.result, ty(P::Char));
}

#[test]
fn effect_inference_cancels_known_labels_before_matching_repeated_declarations() {
    let signature = scheme(
        1,
        comp(
            vec![op(1, vec![variable(0)]), op(1, vec![ty(P::Bool)])],
            ty(P::Bool),
        ),
        variable(0),
    );
    for labels in [
        vec![op(1, vec![ty(P::Bool)]), op(1, vec![ty(P::Char)])],
        vec![op(1, vec![ty(P::Char)]), op(1, vec![ty(P::Bool)])],
    ] {
        let instance = signature
            .infer_types(&[comp(labels, ty(P::Bool))], None, Budget::default())
            .unwrap()
            .finish()
            .unwrap();
        assert_eq!(instance.type_arguments, [ty(P::Char)]);
    }
}

#[test]
fn effect_inference_keeps_unmatched_effects_inside_result_substitution() {
    let signature = scheme(1, comp(vec![op(1, vec![])], variable(0)), variable(0));
    let actual = comp(
        vec![op(2, vec![]), op(1, vec![]), op(1, vec![])],
        ty(P::Bool),
    );
    let instance = signature
        .infer_types(&[actual], None, Budget::default())
        .unwrap()
        .finish()
        .unwrap();
    let residual = comp(vec![op(2, vec![]), op(1, vec![])], ty(P::Bool));
    assert_eq!(instance.type_arguments, [residual.clone()]);
    assert_eq!(instance.result, residual);
}

#[test]
fn ambiguous_instances_wait_for_context_instead_of_choosing_first_permutation() {
    let signature = scheme(
        2,
        comp(
            vec![op(1, vec![variable(0)]), op(1, vec![variable(1)])],
            ty(P::Bool),
        ),
        Term::Tuple(vec![variable(0), variable(1)]),
    );
    let actual = comp(
        vec![op(1, vec![ty(P::Bool)]), op(1, vec![ty(P::Char)])],
        ty(P::Bool),
    );
    assert!(
        signature
            .infer_types(&[actual.clone()], None, Budget::default())
            .unwrap()
            .finish()
            .is_err()
    );
    for expected in [
        Term::Tuple(vec![ty(P::Bool), ty(P::Char)]),
        Term::Tuple(vec![ty(P::Char), ty(P::Bool)]),
    ] {
        let instance = signature
            .infer_types(&[actual.clone()], Some(&expected), Budget::default())
            .unwrap()
            .finish()
            .unwrap();
        assert_eq!(instance.result, expected);
    }
}

#[test]
fn repeated_identical_labels_infer_without_coalescing_occurrences() {
    let signature = scheme(
        1,
        comp(vec![op(1, vec![variable(0)]); 2], ty(P::Bool)),
        variable(0),
    );
    let actual = comp(vec![op(1, vec![ty(P::Char)]); 2], ty(P::Bool));
    let instance = signature
        .infer_types(&[actual], None, Budget::default())
        .unwrap()
        .finish()
        .unwrap();
    assert_eq!(instance.type_arguments, [ty(P::Char)]);
    let missing = comp(vec![op(1, vec![ty(P::Char)])], ty(P::Bool));
    assert!(
        signature
            .infer_types(&[missing], None, Budget::default())
            .unwrap()
            .finish()
            .is_err()
    );
}

#[test]
fn failed_effect_inference_does_not_unfold_opaque_inputs_or_mutate_scheme() {
    let opaque = |input| Term::NormalForm {
        head: DeclarationId {
            package: 2,
            local: 1,
        },
        arguments: vec![input],
        sort: Sort::Type,
    };
    let signature = scheme(
        1,
        comp(vec![op(1, vec![opaque(variable(0))])], ty(P::Bool)),
        variable(0),
    );
    let original = signature.signature.clone();
    let actual = comp(vec![op(1, vec![opaque(ty(P::Char))])], ty(P::Bool));
    assert!(
        signature
            .infer_types(&[actual.clone()], None, Budget::default())
            .unwrap()
            .finish()
            .is_err()
    );
    let mut context = Context::default();
    context.admit_congruence(DeclarationId {
        package: 2,
        local: 1,
    });
    let instance = signature
        .infer_partial_types_in_context(
            &[Some(actual)],
            Some(&ty(P::Char)),
            &context,
            Budget::default(),
        )
        .unwrap()
        .finish()
        .unwrap();
    assert_eq!(instance.type_arguments, [ty(P::Char)]);
    assert_eq!(signature.signature, original);
}

#[test]
fn effect_inference_cannot_export_a_nested_runtime_binder() {
    let callable = |result| Term::Function {
        parameters: vec![Parameter {
            sort: Sort::Runtime,
            ty: ty(P::Bool),
        }],
        result: Box::new(result),
    };
    let template_type = Term::Bound {
        depth: 2,
        index: 0,
        sort: Sort::Type,
    };
    let signature = scheme(
        1,
        callable(comp(vec![op(1, vec![template_type])], ty(P::Bool))),
        variable(0),
    );
    let dependent = Term::NormalForm {
        head: DeclarationId {
            package: 3,
            local: 1,
        },
        sort: Sort::Type,
        arguments: vec![Term::Bound {
            depth: 0,
            index: 0,
            sort: Sort::Runtime,
        }],
    };
    let actual = callable(comp(vec![op(1, vec![dependent])], ty(P::Bool)));
    assert!(
        signature
            .infer_types(&[actual], None, Budget::default())
            .unwrap()
            .finish()
            .is_err()
    );
    let closed = callable(comp(vec![op(1, vec![ty(P::Char)])], ty(P::Bool)));
    let instance = signature
        .infer_types(&[closed], None, Budget::default())
        .unwrap()
        .finish()
        .unwrap();
    assert_eq!(instance.type_arguments, [ty(P::Char)]);
}

#[test]
fn effect_inference_bounds_label_payload_and_recovers_after_failure() {
    let signature = scheme(
        1,
        comp(
            vec![
                op(1, vec![variable(0)]),
                op(2, vec![Term::String("x".repeat(10_000).into())]),
            ],
            ty(P::Bool),
        ),
        variable(0),
    );
    let actual = comp(
        vec![
            op(1, vec![ty(P::Char)]),
            op(2, vec![Term::String("x".repeat(10_000).into())]),
        ],
        ty(P::Bool),
    );
    assert!(
        signature
            .infer_types(
                &[actual.clone()],
                None,
                Budget {
                    steps: 200,
                    max_depth: 64
                }
            )
            .is_err()
    );
    let instance = signature
        .infer_types(
            &[actual],
            None,
            Budget {
                steps: 1_000_000,
                max_depth: 128,
            },
        )
        .unwrap()
        .finish()
        .unwrap();
    assert_eq!(instance.type_arguments, [ty(P::Char)]);
}
