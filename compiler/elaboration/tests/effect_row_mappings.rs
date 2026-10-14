use elaboration::effects::{Error, embed_row, select_layer};
use elaboration::ptc::{Budget, Context, Formula};
use nir::effects::Operation;
use nir::{DeclarationId, Primitive, Sort, Term};

fn op(local: u32, arguments: Vec<Term>) -> Operation {
    Operation {
        declaration: DeclarationId { package: 1, local },
        arguments,
    }
}
fn variable(local: u32) -> Term {
    Term::Rigid {
        declaration: DeclarationId { package: 2, local },
        sort: Sort::Type,
    }
}

#[test]
fn embeddings_preserve_occurrences_and_repeated_requests_reuse_slots() {
    let a = op(1, vec![]);
    let b = op(2, vec![]);
    let extra = op(3, vec![]);
    let source = [a.clone(), b.clone(), a.clone()];
    let target = [extra, b, a.clone(), a];
    let context = Context::default();
    let embedding = embed_row(&source, &target, &context, Budget::default()).unwrap();
    for _ in 0..3 {
        assert_eq!(
            (0..3)
                .map(|i| embedding.target_index(i).unwrap())
                .collect::<Vec<_>>(),
            [2, 1, 3]
        );
    }
    assert_eq!(embedding.target_index(3), None);
    assert_eq!(embedding.source(), source);
    assert_eq!(embedding.target(), target);
    assert!(
        embedding
            .to_string()
            .ends_with("(mapping (occurrence 0 2) (occurrence 1 1) (occurrence 2 3)))")
    );
    assert!(matches!(
        embed_row(&source, &target[..3], &context, Budget::default()),
        Err(Error::MissingEntry)
    ));
    assert!(embed_row(&[], &target, &context, Budget::default()).is_ok());
}

#[test]
fn selecting_a_layer_keeps_other_occurrences_and_unrelated_operations() {
    let a = op(1, vec![]);
    let b = op(2, vec![]);
    let row = [a.clone(), b.clone(), a.clone()];
    let context = Context::default();
    let selected = select_layer(&row, &a, 1, &context, Budget::default()).unwrap();
    assert_eq!(selected.index(), 2);
    assert_eq!(selected.operation(), &a);
    assert_eq!(selected.remaining().collect::<Vec<_>>(), [&a, &b]);
    assert!(
        selected
            .to_string()
            .starts_with("(effect-row-layer (index 2) (operation (operation")
    );
    let first = select_layer(&row, &a, 0, &context, Budget::default()).unwrap();
    assert_eq!(first.index(), 0);
    assert_eq!(first.remaining().collect::<Vec<_>>(), [&b, &a]);
    assert!(matches!(
        select_layer(&row, &a, 2, &context, Budget::default()),
        Err(Error::MissingEntry)
    ));
    assert!(matches!(
        select_layer(&row, &a, usize::MAX, &context, Budget::default()),
        Err(Error::MissingEntry)
    ));
}

#[test]
fn unknown_earlier_identity_blocks_selection_and_cannot_be_skipped() {
    let abstract_label = op(1, vec![variable(1)]);
    let boolean_label = op(1, vec![Term::Primitive(Primitive::Bool)]);
    let row = [abstract_label, boolean_label.clone()];
    let source = [boolean_label.clone()];
    let context = Context::default();
    assert!(matches!(
        embed_row(&source, &row, &context, Budget::default()),
        Err(Error::PendingIdentity)
    ));
    assert!(matches!(
        select_layer(&row, &boolean_label, 0, &context, Budget::default()),
        Err(Error::PendingIdentity)
    ));
    let mut same = Context::default();
    same.assume(Formula::equal(
        variable(1),
        Term::Primitive(Primitive::Bool),
    ));
    assert_eq!(
        embed_row(&source, &row, &same, Budget::default())
            .unwrap()
            .target_index(0),
        Some(0)
    );
    assert_eq!(
        select_layer(&row, &boolean_label, 1, &same, Budget::default())
            .unwrap()
            .index(),
        1
    );
    let mut different = Context::default();
    different.assume(Formula::equal(variable(1), Term::Primitive(Primitive::Bool)).negate());
    assert_eq!(
        embed_row(&source, &row, &different, Budget::default())
            .unwrap()
            .target_index(0),
        Some(1)
    );
    assert_eq!(
        select_layer(&row, &boolean_label, 0, &different, Budget::default())
            .unwrap()
            .index(),
        1
    );
    assert!(matches!(
        select_layer(&row, &boolean_label, 1, &different, Budget::default()),
        Err(Error::MissingEntry)
    ));
    // Neither successful context supplies facts to the original context.
    assert!(matches!(
        select_layer(&row, &boolean_label, 0, &context, Budget::default()),
        Err(Error::PendingIdentity)
    ));
}

#[test]
fn row_mapping_checks_scope_width_depth_and_payload_before_comparing() {
    let context = Context::default();
    let invalid = [op(
        1,
        vec![Term::Bound {
            depth: 0,
            index: 0,
            sort: Sort::Runtime,
        }],
    )];
    assert!(matches!(
        embed_row(&invalid, &invalid, &context, Budget::default()),
        Err(Error::InvalidInput)
    ));
    let wide = vec![op(1, vec![]); 1000];
    assert!(matches!(
        embed_row(
            &wide,
            &wide,
            &context,
            Budget {
                steps: 10,
                max_depth: 64
            }
        ),
        Err(Error::BudgetExhausted)
    ));
    let payload = [op(1, vec![Term::String("x".repeat(10_000).into())])];
    assert!(matches!(
        embed_row(
            &payload,
            &payload,
            &context,
            Budget {
                steps: 100,
                max_depth: 64
            }
        ),
        Err(Error::BudgetExhausted)
    ));
    let mut deep = Term::Primitive(Primitive::Bool);
    for _ in 0..70 {
        deep = Term::Pointer(Box::new(deep));
    }
    let deep = [op(1, vec![deep])];
    assert!(matches!(
        select_layer(&deep, &deep[0], 0, &context, Budget::default()),
        Err(Error::BudgetExhausted)
    ));
    let valid = [op(1, vec![])];
    assert_eq!(
        embed_row(&valid, &valid, &context, Budget::default())
            .unwrap()
            .target_index(0),
        Some(0)
    );
}

#[test]
fn every_small_closed_embedding_maps_to_distinct_ordered_matching_slots() {
    let context = Context::default();
    for source_bits in 0..8usize {
        for target_bits in 0..16usize {
            let source: Vec<_> = (0..3)
                .map(|i| op(((source_bits >> i) & 1) as u32, vec![]))
                .collect();
            let target: Vec<_> = (0..4)
                .map(|i| op(((target_bits >> i) & 1) as u32, vec![]))
                .collect();
            let enough = (0..2).all(|label| {
                source
                    .iter()
                    .filter(|op| op.declaration.local == label)
                    .count()
                    <= target
                        .iter()
                        .filter(|op| op.declaration.local == label)
                        .count()
            });
            let result = embed_row(&source, &target, &context, Budget::default());
            assert_eq!(result.is_ok(), enough);
            if let Ok(mapping) = result {
                let indices: Vec<_> = (0..3).map(|i| mapping.target_index(i).unwrap()).collect();
                assert_eq!(
                    indices
                        .iter()
                        .collect::<std::collections::BTreeSet<_>>()
                        .len(),
                    3
                );
                for i in 0..3 {
                    assert_eq!(source[i], target[indices[i]]);
                    for j in 0..i {
                        if source[i] == source[j] {
                            assert!(indices[j] < indices[i]);
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn composed_maps_keep_occurrence_positions_and_reject_foreign_scopes() {
    let a = op(1, vec![]);
    let b = op(2, vec![]);
    let c = op(3, vec![]);
    let source = [a.clone(), a.clone()];
    let middle = [b.clone(), a.clone(), a.clone()];
    let target = [a.clone(), c, a.clone(), b];
    let context = Context::default();
    let first = embed_row(&source, &middle, &context, Budget::default()).unwrap();
    let second = embed_row(&middle, &target, &context, Budget::default()).unwrap();
    let composed = first.then(&second, Budget::default()).unwrap();
    assert_eq!(composed.target_index(0), Some(0));
    assert_eq!(composed.target_index(1), Some(2));
    let other_context = Context::default();
    let foreign = embed_row(&middle, &target, &other_context, Budget::default()).unwrap();
    assert!(matches!(
        first.then(&foreign, Budget::default()),
        Err(Error::InvalidInput)
    ));
    let different_middle = [a];
    let wrong = embed_row(&different_middle, &target, &context, Budget::default()).unwrap();
    assert!(matches!(
        first.then(&wrong, Budget::default()),
        Err(Error::InvalidInput)
    ));
    assert!(matches!(
        first.then(
            &second,
            Budget {
                steps: 0,
                max_depth: 64
            }
        ),
        Err(Error::BudgetExhausted)
    ));
    assert_eq!(
        first
            .then(&second, Budget::default())
            .unwrap()
            .target_index(1),
        Some(2)
    );
}
