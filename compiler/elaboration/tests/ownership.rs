use elaboration::verification::{
    PlaceId,
    ownership::{Availability as A, Error, Place, Shape, State},
};
fn place(fields: &[usize]) -> Place {
    Place {
        root: PlaceId { owner: 1, local: 0 },
        fields: fields.to_vec(),
    }
}
fn state() -> State {
    let mut state = State::default();
    state
        .declare(
            place(&[]).root,
            &Shape::Fields(vec![
                Shape::Value,
                Shape::Fields(vec![Shape::Value, Shape::Value]),
            ]),
            true,
            &mut 100,
        )
        .unwrap();
    state
}
#[test]
fn partial_moves_preserve_siblings_and_install_restores_the_aggregate() {
    let mut state = state();
    state.take(&place(&[1, 0])).unwrap();
    assert_eq!(state.require_present(&place(&[])), Err(Error::NotPresent));
    assert_eq!(state.take(&place(&[1])), Err(Error::NotPresent));
    state.require_present(&place(&[0])).unwrap();
    state.require_present(&place(&[1, 1])).unwrap();
    assert_eq!(state.remaining().len(), 2);
    state.install(&place(&[1, 0])).unwrap();
    state.take(&place(&[])).unwrap();
    assert!(state.remaining().is_empty());
    state.install(&place(&[])).unwrap();
    state.require_present(&place(&[])).unwrap();
}
#[test]
fn branch_join_tracks_conditional_responsibility_without_granting_access() {
    let full = state();
    let mut moved = full.clone();
    moved.take(&place(&[0])).unwrap();
    let mut joined = full.join(&moved, &mut 100).unwrap();
    assert_eq!(joined.remaining()[0], (place(&[0]), A::Conditional));
    assert_eq!(joined.require_present(&place(&[0])), Err(Error::NotPresent));
    assert_eq!(joined.install(&place(&[0])), Err(Error::NotEmpty));
    joined.require_present(&place(&[1])).unwrap();
    assert_eq!(
        full.join(&moved, &mut 100).unwrap(),
        moved.join(&full, &mut 100).unwrap()
    );
    assert_eq!(moved.join(&moved, &mut 100).unwrap(), moved);
}

#[test]
fn construction_guards_use_proved_implications_without_inventing_sibling_access() {
    let full = state();
    let mut absent = full.clone();
    absent.take(&place(&[])).unwrap();
    let joined = full.join(&absent, &mut 100).unwrap();
    let present = joined
        .under_construction_guard(&place(&[]), true, &mut 100)
        .unwrap();
    let empty = joined
        .under_construction_guard(&place(&[]), false, &mut 100)
        .unwrap();
    // Here the whole object was moved on one branch. The live-token branch
    // therefore retains all fields; the unguarded join still grants no access.
    present.require_present(&place(&[])).unwrap();
    assert_eq!(joined.require_present(&place(&[])), Err(Error::NotPresent));
    assert_eq!(empty.require_present(&place(&[0])), Err(Error::NotPresent));
    assert_eq!(
        present
            .cleanup_tree(&place(&[]), &mut 100)
            .unwrap()
            .unwrap()
            .availability,
        A::Present
    );
    assert_eq!(
        empty
            .cleanup_tree(&place(&[]), &mut 100)
            .unwrap()
            .unwrap()
            .availability,
        A::Empty
    );
    let leaf = joined
        .under_construction_guard(&place(&[0]), true, &mut 100)
        .unwrap();
    leaf.require_present(&place(&[0])).unwrap();
    assert_eq!(leaf.require_present(&place(&[1])), Err(Error::NotPresent));
    assert_eq!(joined, full.join(&absent, &mut 100).unwrap());
    assert_eq!(
        joined.under_construction_guard(&place(&[]), true, &mut 0),
        Err(Error::Budget)
    );
    assert_eq!(
        full.under_construction_guard(&place(&[]), false, &mut 100),
        Err(Error::ContradictoryGuard)
    );
    assert_eq!(
        absent.under_construction_guard(&place(&[]), true, &mut 100),
        Err(Error::ContradictoryGuard)
    );
    assert_eq!(
        joined.under_construction_guard(&place(&[99]), true, &mut 100),
        Err(Error::MissingPlace)
    );
    assert!(
        joined
            .under_construction_guard(&place(&[0]), true, &mut 100)
            .is_ok()
    );
}
#[test]
fn failed_transfers_and_undeclared_paths_do_not_change_responsibilities() {
    let mut state = state();
    let before = state.clone();
    assert_eq!(state.install(&place(&[])), Err(Error::NotEmpty));
    assert_eq!(state.take(&place(&[2])), Err(Error::MissingPlace));
    assert_eq!(state.take(&place(&[0, 0])), Err(Error::MissingPlace));
    assert_eq!(state, before);
    state.take(&place(&[0])).unwrap();
    let after = state.clone();
    assert_eq!(state.take(&place(&[0])), Err(Error::NotPresent));
    assert_eq!(state, after);
}
#[test]
fn zero_field_aggregates_still_have_an_affine_responsibility() {
    let mut state = State::default();
    state
        .declare(place(&[]).root, &Shape::Fields(vec![]), true, &mut 10)
        .unwrap();
    assert_eq!(state.remaining().len(), 1);
    state.take(&place(&[])).unwrap();
    assert_eq!(state.take(&place(&[])), Err(Error::NotPresent));
    let mut scalar = State::default();
    scalar
        .declare(place(&[]).root, &Shape::Value, false, &mut 10)
        .unwrap();
    assert_eq!(state.join(&scalar, &mut 10), Err(Error::ShapeMismatch));
}
#[test]
fn bounded_declaration_and_join_are_transactional_and_dump_is_stable() {
    let mut state = State::default();
    let before = state.clone();
    assert_eq!(
        state.declare(
            place(&[]).root,
            &Shape::Fields(vec![Shape::Value]),
            true,
            &mut 1
        ),
        Err(Error::Budget)
    );
    assert_eq!(state, before);
    state
        .declare(place(&[]).root, &Shape::Value, false, &mut 10)
        .unwrap();
    assert_eq!(state.require_present(&place(&[])), Err(Error::NotPresent));
    state.install(&place(&[])).unwrap();
    assert_eq!(
        state.to_string(),
        "(ownership-state (root (owner 1) (local 0) present))"
    );
    assert_eq!(state.join(&state, &mut 0), Err(Error::Budget));
    assert_eq!(
        state.join(&State::default(), &mut 10),
        Err(Error::ShapeMismatch)
    );
}

#[test]
fn all_three_field_path_pairs_match_concrete_availability() {
    fn mask_state(mask: u8) -> State {
        let mut state = State::default();
        state
            .declare(
                place(&[]).root,
                &Shape::Fields(vec![Shape::Value; 3]),
                false,
                &mut 10,
            )
            .unwrap();
        for field in 0..3 {
            if mask & (1 << field) != 0 {
                state.install(&place(&[field])).unwrap();
            }
        }
        state
    }
    for left in 0u8..8 {
        for right in 0u8..8 {
            let joined = mask_state(left).join(&mask_state(right), &mut 100).unwrap();
            assert_eq!(
                joined.require_present(&place(&[])).is_ok(),
                left == 7 && right == 7
            );
            for field in 0..3 {
                let bit = 1 << field;
                assert_eq!(
                    joined.require_present(&place(&[field])).is_ok(),
                    left & bit != 0 && right & bit != 0
                );
                let mut updated = joined.clone();
                assert_eq!(
                    updated.install(&place(&[field])).is_ok(),
                    left & bit == 0 && right & bit == 0
                );
                let responsibility = joined
                    .remaining()
                    .into_iter()
                    .find(|(p, _)| *p == place(&[field]));
                let expected = match (left & bit != 0, right & bit != 0) {
                    (false, false) => None,
                    (true, true) => Some((place(&[field]), A::Present)),
                    _ => Some((place(&[field]), A::Conditional)),
                };
                assert_eq!(responsibility, expected);
            }
            for third in 0u8..8 {
                let a = joined.join(&mask_state(third), &mut 100).unwrap();
                let b = mask_state(left)
                    .join(
                        &mask_state(right)
                            .join(&mask_state(third), &mut 100)
                            .unwrap(),
                        &mut 100,
                    )
                    .unwrap();
                assert_eq!(a, b);
            }
        }
    }
}

#[test]
fn replacement_preserves_siblings_and_returns_conditional_cleanup() {
    let full = state();
    let mut moved = full.clone();
    moved.take(&place(&[1, 0])).unwrap();
    let mut joined = full.join(&moved, &mut 100).unwrap();
    let before = joined.clone();
    assert_eq!(joined.replace(&place(&[5])), Err(Error::MissingPlace));
    assert_eq!(joined, before);
    let cleanup = joined.replace(&place(&[1])).unwrap();
    assert_eq!(
        cleanup,
        vec![
            (place(&[1, 0]), A::Conditional),
            (place(&[1, 1]), A::Present)
        ]
    );
    joined.require_present(&place(&[])).unwrap();
    joined.take(&place(&[0])).unwrap();
    assert!(joined.replace(&place(&[0])).unwrap().is_empty());
    assert_eq!(joined.retire(place(&[]).root).unwrap().len(), 3);
    assert_eq!(
        joined.require_present(&place(&[])),
        Err(Error::MissingPlace)
    );
}

#[test]
fn moving_all_fields_is_distinct_from_moving_the_constructed_object() {
    let mut fields_moved = state();
    for path in [&[0][..], &[1, 0], &[1, 1]] {
        fields_moved.take(&place(path)).unwrap();
    }
    assert!(fields_moved.remaining().is_empty());
    let tree = fields_moved
        .cleanup_tree(&place(&[]), &mut 100)
        .unwrap()
        .unwrap();
    assert_eq!(tree.availability, A::Present);
    let children = tree.children.as_ref().unwrap();
    assert_eq!(
        children.len(),
        1,
        "the nested completed object also survives"
    );
    assert_eq!(children[0].place, place(&[1]));
    assert_eq!(children[0].availability, A::Present);
    assert!(children[0].children.as_ref().unwrap().is_empty());
    assert_eq!(fields_moved.install(&place(&[])), Err(Error::NotEmpty));

    let mut whole_moved = state();
    whole_moved.take(&place(&[])).unwrap();
    assert!(whole_moved.remaining().is_empty());
    assert!(
        whole_moved
            .cleanup_tree(&place(&[]), &mut 100)
            .unwrap()
            .is_none()
    );
    assert_ne!(fields_moved, whole_moved);
    let joined = fields_moved.join(&whole_moved, &mut 100).unwrap();
    let tree = joined.cleanup_tree(&place(&[]), &mut 100).unwrap().unwrap();
    assert_eq!(tree.availability, A::Conditional);
    assert_eq!(tree.children.unwrap()[0].availability, A::Conditional);
}

#[test]
fn partial_initialization_has_no_whole_object_destructor_until_completed() {
    let mut state = State::default();
    state
        .declare(
            place(&[]).root,
            &Shape::Fields(vec![Shape::Value; 2]),
            false,
            &mut 100,
        )
        .unwrap();
    state.install(&place(&[0])).unwrap();
    let tree = state.cleanup_tree(&place(&[]), &mut 100).unwrap().unwrap();
    assert_eq!(tree.availability, A::Empty);
    assert_eq!(tree.children.as_ref().unwrap().len(), 1);
    assert_eq!(tree.children.unwrap()[0].availability, A::Present);
    assert_eq!(state.require_present(&place(&[])), Err(Error::NotPresent));
    state.install(&place(&[1])).unwrap();
    state.require_present(&place(&[])).unwrap();
    assert_eq!(
        state
            .cleanup_tree(&place(&[]), &mut 100)
            .unwrap()
            .unwrap()
            .availability,
        A::Present
    );
    state.take(&place(&[0])).unwrap();
    assert_eq!(
        state
            .cleanup_tree(&place(&[]), &mut 100)
            .unwrap()
            .unwrap()
            .availability,
        A::Present
    );
}

#[test]
fn cleanup_hierarchy_is_parent_first_reverse_fields_and_bounded() {
    let state = state();
    let before = state.clone();
    assert_eq!(state.cleanup_tree(&place(&[]), &mut 1), Err(Error::Budget));
    assert_eq!(state, before);
    let tree = state.cleanup_tree(&place(&[]), &mut 100).unwrap().unwrap();
    fn visit(
        tree: &elaboration::verification::ownership::CleanupTree,
        output: &mut Vec<Vec<usize>>,
    ) {
        output.push(tree.place.fields.clone());
        if let Some(children) = &tree.children {
            for child in children {
                visit(child, output);
            }
        }
    }
    let mut paths = vec![];
    visit(&tree, &mut paths);
    assert_eq!(
        paths,
        vec![vec![], vec![1], vec![1, 1], vec![1, 0], vec![0]]
    );
    assert!(tree.to_string().starts_with("(cleanup-tree (owned-place"));
    assert!(state.to_string().contains("(fields (constructed present)"));
    assert_eq!(
        state.cleanup_tree(&place(&[5]), &mut 100),
        Err(Error::MissingPlace)
    );
}

#[test]
fn aggregate_construction_flags_join_independently_of_field_liveness() {
    fn value(completed: bool, mask: u8) -> State {
        let mut state = State::default();
        state
            .declare(
                place(&[]).root,
                &Shape::Fields(vec![Shape::Value; 3]),
                completed,
                &mut 100,
            )
            .unwrap();
        for field in 0..3 {
            if completed && mask & (1 << field) == 0 {
                state.take(&place(&[field])).unwrap();
            }
            if !completed && mask & (1 << field) != 0 {
                state.install(&place(&[field])).unwrap();
            }
        }
        state
    }
    for complete_a in [false, true] {
        for complete_b in [false, true] {
            for mask_a in 0..8 {
                for mask_b in 0..8 {
                    let a = value(complete_a, mask_a);
                    let b = value(complete_b, mask_b);
                    let joined = a.join(&b, &mut 100).unwrap();
                    assert_eq!(joined, b.join(&a, &mut 100).unwrap());
                    let parent_a = complete_a || mask_a == 7;
                    let parent_b = complete_b || mask_b == 7;
                    let expected = match (parent_a, parent_b) {
                        (false, false) => A::Empty,
                        (true, true) => A::Present,
                        _ => A::Conditional,
                    };
                    let tree = joined.cleanup_tree(&place(&[]), &mut 100).unwrap();
                    if !parent_a && !parent_b && mask_a == 0 && mask_b == 0 {
                        assert!(tree.is_none());
                    } else {
                        assert_eq!(tree.unwrap().availability, expected);
                    }
                    assert_eq!(
                        joined.require_present(&place(&[])).is_ok(),
                        mask_a == 7 && mask_b == 7
                    );
                }
            }
        }
    }
}

#[test]
fn guarded_field_facts_match_every_nonempty_set_of_concrete_construction_states() {
    let cases: Vec<_> = [false, true]
        .into_iter()
        .flat_map(|constructed| {
            (0u8..4)
                .filter(move |mask| constructed || *mask != 3)
                .map(move |mask| (constructed, mask))
        })
        .collect();
    let concrete = |constructed: bool, mask: u8| {
        let mut state = State::default();
        state
            .declare(
                place(&[]).root,
                &Shape::Fields(vec![Shape::Value; 2]),
                constructed,
                &mut 100,
            )
            .unwrap();
        for index in 0..2 {
            if mask & (1 << index) != 0 {
                if !constructed {
                    state.install(&place(&[index])).unwrap();
                }
            } else if constructed {
                state.take(&place(&[index])).unwrap();
            }
        }
        state
    };
    for set in 1u8..128 {
        let selected: Vec<_> = cases
            .iter()
            .enumerate()
            .filter(|(index, _)| set & (1 << index) != 0)
            .map(|(_, case)| *case)
            .collect();
        let states: Vec<_> = selected.iter().map(|(c, m)| concrete(*c, *m)).collect();
        let merged = states
            .iter()
            .skip(1)
            .fold(states[0].clone(), |a, b| a.join(b, &mut 10000).unwrap());
        let reversed = states
            .iter()
            .rev()
            .skip(1)
            .fold(states.last().unwrap().clone(), |a, b| {
                a.join(b, &mut 10000).unwrap()
            });
        assert_eq!(merged, reversed, "subset {set}");
        assert_eq!(merged.join(&merged, &mut 10000).unwrap(), merged);
        let present: Vec<_> = selected
            .iter()
            .filter(|(constructed, _)| *constructed)
            .collect();
        let guarded = merged.under_construction_guard(&place(&[]), true, &mut 10000);
        if present.is_empty() {
            assert_eq!(guarded, Err(Error::ContradictoryGuard));
            continue;
        }
        let guarded = guarded.unwrap();
        for index in 0..2 {
            let expected_present = present.iter().all(|(_, mask)| mask & (1 << index) != 0);
            let expected_empty = present.iter().all(|(_, mask)| mask & (1 << index) == 0);
            assert_eq!(
                guarded.require_present(&place(&[index])).is_ok(),
                expected_present,
                "subset {set} field {index}: {guarded}"
            );
            assert_eq!(
                guarded
                    .cleanup_tree(&place(&[index]), &mut 100)
                    .unwrap()
                    .is_none(),
                expected_empty,
                "subset {set} field {index}"
            );
        }
    }
}

#[test]
fn reconstruction_cannot_reuse_facts_from_the_former_live_parent_paths() {
    let mut live = State::default();
    live.declare(
        place(&[]).root,
        &Shape::Fields(vec![Shape::Value; 2]),
        true,
        &mut 100,
    )
    .unwrap();
    live.take(&place(&[0])).unwrap();
    live.take(&place(&[1])).unwrap();
    let mut partial = State::default();
    partial
        .declare(
            place(&[]).root,
            &Shape::Fields(vec![Shape::Value; 2]),
            false,
            &mut 100,
        )
        .unwrap();
    partial.install(&place(&[1])).unwrap();
    let original = live.join(&partial, &mut 1000).unwrap();
    let before = original
        .under_construction_guard(&place(&[]), true, &mut 1000)
        .unwrap();
    assert!(
        before
            .cleanup_tree(&place(&[1]), &mut 100)
            .unwrap()
            .is_none()
    );
    for replace in [false, true] {
        let mut changed = original.clone();
        if replace {
            changed.replace(&place(&[0])).unwrap();
        } else {
            changed.install(&place(&[0])).unwrap();
        }
        let guarded = changed
            .under_construction_guard(&place(&[]), true, &mut 1000)
            .unwrap();
        assert_eq!(
            guarded
                .cleanup_tree(&place(&[1]), &mut 100)
                .unwrap()
                .unwrap()
                .availability,
            A::Conditional
        );
    }
    assert_eq!(
        original.under_construction_guard(&place(&[]), true, &mut 0),
        Err(Error::Budget)
    );
    assert_eq!(original, live.join(&partial, &mut 1000).unwrap());
}

#[test]
fn construction_implications_remain_sound_after_every_two_field_join_and_mutation() {
    let concrete = |constructed: bool, mask: u8| {
        let mut state = State::default();
        state
            .declare(
                place(&[]).root,
                &Shape::Fields(vec![Shape::Value; 2]),
                constructed,
                &mut 100,
            )
            .unwrap();
        for index in 0..2 {
            if mask & (1 << index) != 0 {
                if !constructed {
                    state.install(&place(&[index])).unwrap();
                }
            } else if constructed {
                state.take(&place(&[index])).unwrap();
            }
        }
        state
    };
    let cases: Vec<_> = [false, true]
        .into_iter()
        .flat_map(|constructed| {
            (0u8..4)
                .filter(move |mask| constructed || *mask != 3)
                .map(move |mask| (constructed, mask))
        })
        .collect();
    for &(lc, lm) in &cases {
        for &(rc, rm) in &cases {
            for field in 0..2 {
                for replace in [false, true] {
                    let mut left = concrete(lc, lm);
                    let mut right = concrete(rc, rm);
                    let mut abstract_state = left.join(&right, &mut 1000).unwrap();
                    if !replace && (lm & (1 << field) == 0 || rm & (1 << field) == 0) {
                        continue;
                    }
                    for state in [&mut left, &mut right, &mut abstract_state] {
                        if replace {
                            state.replace(&place(&[field])).unwrap();
                        } else {
                            state.take(&place(&[field])).unwrap();
                        }
                    }
                    let concrete_live: Vec<_> = [left, right]
                        .into_iter()
                        .filter(|state| {
                            state
                                .cleanup_tree(&place(&[]), &mut 100)
                                .unwrap()
                                .is_some_and(|tree| tree.availability == A::Present)
                        })
                        .collect();
                    if concrete_live.is_empty() {
                        continue;
                    }
                    let guarded = abstract_state
                        .under_construction_guard(&place(&[]), true, &mut 1000)
                        .unwrap();
                    for tested in 0..2 {
                        let observed = guarded.cleanup_tree(&place(&[tested]), &mut 100).unwrap();
                        for actual in &concrete_live {
                            if observed.is_none() {
                                assert!(
                                    actual
                                        .cleanup_tree(&place(&[tested]), &mut 100)
                                        .unwrap()
                                        .is_none()
                                );
                            }
                            if guarded.require_present(&place(&[tested])).is_ok() {
                                actual.require_present(&place(&[tested])).unwrap();
                            }
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn reconstruction_retains_positive_facts_and_proves_fields_of_newly_completed_parents() {
    let full = state();
    let mut moved = full.clone();
    moved.take(&place(&[])).unwrap();
    let mut joined = full.join(&moved, &mut 1000).unwrap();
    joined.replace(&place(&[0])).unwrap();
    joined
        .under_construction_guard(&place(&[]), true, &mut 1000)
        .unwrap()
        .require_present(&place(&[]))
        .unwrap();
    assert_eq!(joined.require_present(&place(&[])), Err(Error::NotPresent));

    let mut partially_built = moved.clone();
    partially_built.install(&place(&[1])).unwrap();
    let mut reconstruction = partially_built.join(&moved, &mut 1000).unwrap();
    reconstruction.install(&place(&[0])).unwrap();
    assert_eq!(
        reconstruction
            .cleanup_tree(&place(&[]), &mut 100)
            .unwrap()
            .unwrap()
            .availability,
        A::Conditional
    );
    reconstruction
        .under_construction_guard(&place(&[]), true, &mut 1000)
        .unwrap()
        .require_present(&place(&[]))
        .unwrap();
    assert_eq!(
        reconstruction.require_present(&place(&[1])),
        Err(Error::NotPresent)
    );
    let snapshot = reconstruction.clone();
    assert_eq!(
        reconstruction.under_construction_guard(&place(&[]), true, &mut 0),
        Err(Error::Budget)
    );
    assert_eq!(snapshot, reconstruction);
}

#[test]
fn budgeted_reconstruction_rolls_back_every_incomplete_prefix_and_preserves_other_owners() {
    let full = state();
    let mut empty = full.clone();
    empty.take(&place(&[])).unwrap();
    let mut partial = empty.clone();
    partial.install(&place(&[1])).unwrap();
    let mut original = partial.join(&empty, &mut 1000).unwrap();
    let other = Place {
        root: PlaceId { owner: 2, local: 0 },
        fields: vec![],
    };
    original
        .declare(other.root, &Shape::Value, true, &mut 100)
        .unwrap();
    for replace in [false, true] {
        let mut saw_failure = false;
        let mut saw_success = false;
        for budget in 0..200 {
            let mut changed = original.clone();
            let result = if replace {
                changed
                    .replace_with_budget(&place(&[0]), &mut { budget })
                    .map(|_| ())
            } else {
                changed.install_with_budget(&place(&[0]), &mut { budget })
            };
            match result {
                Err(Error::Budget) => {
                    saw_failure = true;
                    assert_eq!(changed, original, "budget {budget}, replace {replace}");
                }
                Ok(()) => {
                    saw_success = true;
                    changed.require_present(&other).unwrap();
                    changed
                        .under_construction_guard(&place(&[]), true, &mut 1000)
                        .unwrap()
                        .require_present(&place(&[]))
                        .unwrap();
                }
                other => panic!("unexpected reconstruction result: {other:?}"),
            }
        }
        assert!(saw_failure && saw_success);
    }
}
