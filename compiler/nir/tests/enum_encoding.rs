use comptime_runtime::{Budget, bitvec::BitVec};
use nir::enum_encoding::{Error, Field, Layout, Variant};

#[test]
fn symbol_encodings_share_checked_construction_and_discrimination() {
    use nir::enum_encoding::{SymbolRepresentation, SymbolVariant};
    let entries = vec![
        SymbolVariant {
            name: "client.ready".into(),
            value: "wire_ready".into(),
        },
        SymbolVariant {
            name: "closed".into(),
            value: "wire_\"λ".into(),
        },
    ];
    let layout = SymbolRepresentation::new(entries.clone(), &mut Budget::default()).unwrap();
    for index in 0..entries.len() {
        let raw = layout.encode(index).unwrap();
        assert_eq!(
            layout.discriminate(raw, &mut Budget::default()).unwrap(),
            index
        );
    }
    assert_eq!(layout.encode(2), None);
    assert_eq!(
        layout.discriminate("unknown", &mut Budget::default()),
        Err(Error::InvalidValue)
    );
    assert!(layout.to_string().contains("(symbol \"wire_\\\"λ\")"));
    assert_eq!(
        SymbolRepresentation::new(
            vec![
                entries[0].clone(),
                SymbolVariant {
                    name: "other".into(),
                    value: "wire_ready".into()
                }
            ],
            &mut Budget::default()
        ),
        Err(Error::SymbolOverlap {
            first: 0,
            second: 1,
            witness: "wire_ready".into()
        })
    );
    for duplicate in [
        SymbolVariant {
            name: "other".into(),
            value: "wire_ready".into(),
        },
        SymbolVariant {
            name: "client.ready".into(),
            value: "other".into(),
        },
    ] {
        assert!(
            SymbolRepresentation::new(vec![entries[0].clone(), duplicate], &mut Budget::default())
                .is_err()
        );
    }
    assert!(
        SymbolRepresentation::new(
            entries.clone(),
            &mut Budget {
                steps: 1,
                ..Budget::default()
            }
        )
        .is_err()
    );
    assert!(
        layout
            .discriminate(
                "wire_ready",
                &mut Budget {
                    steps: 1,
                    ..Budget::default()
                }
            )
            .is_err()
    );
    let recovered = SymbolRepresentation::new(entries, &mut Budget::default()).unwrap();
    assert_eq!(
        recovered
            .discriminate("wire_ready", &mut Budget::default())
            .unwrap(),
        0
    );
    assert_eq!(
        SymbolRepresentation::new(vec![], &mut Budget::default())
            .unwrap()
            .discriminate("anything", &mut Budget::default()),
        Err(Error::InvalidValue)
    );
}

fn bits(width: usize, value: u32) -> BitVec {
    BitVec::from_words(width, &[value], &mut Budget::default()).unwrap()
}

fn instruction(name: &str, value: u32) -> Variant {
    Variant::new(
        name.into(),
        bits(32, 0xfe00707f),
        bits(32, value),
        [("rs2", 20), ("rs1", 15), ("rd", 7)]
            .into_iter()
            .map(|(name, start)| Field {
                name: name.into(),
                start,
                width: 5,
            })
            .collect(),
        &mut Budget::default(),
    )
    .unwrap()
}

#[test]
fn risc_v_instructions_share_construction_discrimination_and_payload_positions() {
    let mut budget = Budget::default();
    let layout = Layout::new(
        32,
        vec![instruction("add", 0x33), instruction("sub", 0x40000033)],
        &mut budget,
    )
    .unwrap();
    for (index, raw) in [(0, 0x003100b3), (1, 0x403100b3)] {
        let payloads = vec![bits(5, 3), bits(5, 2), bits(5, 1)];
        let encoded = layout.variants()[index]
            .construct(&payloads, &mut budget)
            .unwrap();
        assert_eq!(encoded, bits(32, raw));
        assert_eq!(layout.discriminate(&encoded, &mut budget).unwrap(), index);
        assert_eq!(
            layout.variants()[index]
                .extract(&encoded, &mut budget)
                .unwrap(),
            payloads
        );
    }
    let missing = layout.uncovered(&mut budget).unwrap().unwrap();
    assert!(matches!(
        layout.discriminate(&missing, &mut budget),
        Err(Error::InvalidValue)
    ));
    assert!(matches!(
        layout.discriminate(&bits(32, 0), &mut budget),
        Err(Error::InvalidValue)
    ));
    assert!(matches!(
        layout.variants()[0].extract(&bits(32, 0x403100b3), &mut budget),
        Err(Error::InvalidValue)
    ));
    let dump = layout.to_string();
    assert!(dump.starts_with("(enum-encoding (width 32)"));
    assert!(dump.contains("(field (name \"rs2\") (start 20) (width 5))"));
}

#[test]
fn overlap_reports_an_actual_shared_encoding() {
    let first = instruction("add", 0x33);
    let second = instruction("alias", 0x33);
    let mut budget = Budget::default();
    let error = Layout::new(32, vec![first.clone(), second.clone()], &mut budget).unwrap_err();
    let Error::Overlap {
        first: 0,
        second: 1,
        witness,
    } = error
    else {
        panic!("{error}");
    };
    assert!(first.accepts(&witness, &mut budget).unwrap());
    assert!(second.accepts(&witness, &mut budget).unwrap());
}

#[test]
fn payload_validation_never_truncates_or_invents_bits() {
    let mut budget = Budget::default();
    for fields in [
        vec![Field {
            name: "x".into(),
            start: 0,
            width: 5,
        }],
        vec![Field {
            name: "x".into(),
            start: 4,
            width: 5,
        }],
        vec![Field {
            name: "x".into(),
            start: 4,
            width: 3,
        }],
        vec![
            Field {
                name: "x".into(),
                start: 4,
                width: 4,
            },
            Field {
                name: "y".into(),
                start: 4,
                width: 4,
            },
        ],
        vec![
            Field {
                name: "x".into(),
                start: 4,
                width: 2,
            },
            Field {
                name: "x".into(),
                start: 6,
                width: 2,
            },
        ],
        vec![Field {
            name: "x".into(),
            start: usize::MAX,
            width: 2,
        }],
    ] {
        assert!(Variant::new("bad".into(), bits(8, 15), bits(8, 3), fields, &mut budget).is_err());
    }
    assert!(Variant::new("bad".into(), bits(8, 15), bits(8, 16), vec![], &mut budget).is_err());
    let variant = instruction("add", 0x33);
    assert!(variant.construct(&[], &mut budget).is_err());
    assert!(
        variant
            .construct(&[bits(6, 32), bits(5, 0), bits(5, 0)], &mut budget)
            .is_err()
    );
}

#[test]
fn coverage_agrees_with_exhaustive_small_domains() {
    let mut budget = Budget::default();
    // Every subset of eight singleton encodings: coverage and membership are
    // checked against a separate enumeration of the full input domain.
    for selected in 0u32..256 {
        let variants = (0..8)
            .filter(|value| selected & (1 << value) != 0)
            .map(|value| {
                Variant::new(
                    value.to_string(),
                    bits(3, 7),
                    bits(3, value),
                    vec![],
                    &mut budget,
                )
                .unwrap()
            })
            .collect();
        let layout = Layout::new(3, variants, &mut budget).unwrap();
        let missing = layout.uncovered(&mut budget).unwrap();
        assert_eq!(missing.is_none(), selected == 255);
        if let Some(missing) = missing {
            assert_eq!(selected & (1 << missing.words()[0]), 0);
        }
        for raw in 0..8 {
            assert_eq!(
                layout.discriminate(&bits(3, raw), &mut budget).is_ok(),
                selected & (1 << raw) != 0
            );
        }
    }
}

#[test]
fn masked_payload_domains_match_exhaustive_intersection_and_coverage() {
    // Each bit independently takes one of {fixed zero, fixed one, payload}.
    let mut cubes = Vec::new();
    for mut code in 0..27 {
        let (mut mask, mut value) = (0, 0);
        let mut fields = Vec::new();
        for bit in 0..3 {
            match code % 3 {
                0 => mask |= 1 << bit,
                1 => {
                    mask |= 1 << bit;
                    value |= 1 << bit;
                }
                _ => fields.push(Field {
                    name: format!("b{bit}"),
                    start: bit,
                    width: 1,
                }),
            }
            code /= 3;
        }
        cubes.push((mask, value, fields));
    }
    for (mask_a, value_a, fields_a) in &cubes {
        for (mask_b, value_b, fields_b) in &cubes {
            let mut budget = Budget::default();
            let a = Variant::new(
                "a".into(),
                bits(3, *mask_a),
                bits(3, *value_a),
                fields_a.clone(),
                &mut budget,
            )
            .unwrap();
            let b = Variant::new(
                "b".into(),
                bits(3, *mask_b),
                bits(3, *value_b),
                fields_b.clone(),
                &mut budget,
            )
            .unwrap();
            let overlap = (0..8).any(|raw| raw & mask_a == *value_a && raw & mask_b == *value_b);
            match Layout::new(3, vec![a, b], &mut budget) {
                Err(Error::Overlap { witness, .. }) => {
                    assert!(overlap);
                    assert_eq!(witness.words()[0] & mask_a, *value_a);
                    assert_eq!(witness.words()[0] & mask_b, *value_b);
                }
                Ok(layout) => {
                    assert!(!overlap);
                    let covered =
                        (0..8).all(|raw| raw & mask_a == *value_a || raw & mask_b == *value_b);
                    let missing = layout.uncovered(&mut budget).unwrap();
                    assert_eq!(missing.is_none(), covered);
                    if let Some(missing) = missing {
                        assert_ne!(missing.words()[0] & mask_a, *value_a);
                        assert_ne!(missing.words()[0] & mask_b, *value_b);
                    }
                }
                Err(error) => panic!("unexpected error: {error}"),
            }
        }
    }
}

#[test]
fn whole_width_payloads_and_non_word_aligned_encodings_round_trip() {
    let mut budget = Budget::default();
    let width = 257;
    let zero = BitVec::from_words(width, &[0; 9], &mut budget).unwrap();
    let payload = BitVec::from_words(
        width,
        &[u32::MAX, u32::MAX, 2, 3, 4, 5, 6, 7, 1],
        &mut budget,
    )
    .unwrap();
    let variant = Variant::new(
        "x\"\n".into(),
        zero.clone(),
        zero,
        vec![Field {
            name: "payload".into(),
            start: 0,
            width,
        }],
        &mut budget,
    )
    .unwrap();
    let layout = Layout::new(width, vec![variant], &mut budget).unwrap();
    assert_eq!(layout.uncovered(&mut budget).unwrap(), None);
    let result = layout.variants()[0]
        .construct(&[payload.clone()], &mut budget)
        .unwrap();
    assert_eq!(
        layout.variants()[0].extract(&result, &mut budget).unwrap(),
        vec![payload]
    );
    assert!(layout.to_string().contains("(name \"x\\\"\\n\")"));
    let empty = Layout::new(width, vec![], &mut budget).unwrap();
    assert!(empty.uncovered(&mut budget).unwrap().is_some());
}

#[test]
fn exhausted_checks_never_publish_coverage_or_overlap_results() {
    let layout = Layout::new(32, vec![instruction("add", 0x33)], &mut Budget::default()).unwrap();
    let mut exhausted = Budget {
        steps: 0,
        max_limbs: 100,
    };
    assert!(layout.uncovered(&mut exhausted).is_err());
    assert!(
        layout
            .discriminate(&bits(32, 0x33), &mut exhausted)
            .is_err()
    );
    assert!(
        layout.variants()[0]
            .construct(&[bits(5, 0), bits(5, 0), bits(5, 0)], &mut exhausted)
            .is_err()
    );
    assert!(Layout::new(32, vec![], &mut exhausted).is_err());
    assert!(
        Layout::new(
            32,
            vec![instruction("add", 0x33)],
            &mut Budget {
                steps: 100,
                max_limbs: 0
            }
        )
        .is_err()
    );
    assert!(layout.uncovered(&mut Budget::default()).unwrap().is_some());
    let constant = Variant::new(
        "constant".into(),
        bits(32, u32::MAX),
        bits(32, 1),
        vec![],
        &mut Budget::default(),
    )
    .unwrap();
    assert!(
        constant
            .construct(
                &[],
                &mut Budget {
                    steps: 100,
                    max_limbs: 0
                }
            )
            .is_err()
    );
}

#[test]
fn empty_payloads_preserve_identity_without_occupying_encoding_bits() {
    let mut budget = Budget::default();
    let mask = BitVec::from_words(8, &[255], &mut budget).unwrap();
    let value = BitVec::from_words(8, &[170], &mut budget).unwrap();
    let empty = BitVec::from_words(0, &[], &mut budget).unwrap();
    let fields: Vec<_> = [0, 4, 8]
        .into_iter()
        .enumerate()
        .map(|(i, start)| Field {
            name: format!("empty{i}"),
            start,
            width: 0,
        })
        .collect();
    let variant = Variant::new(
        "data".into(),
        mask.clone(),
        value.clone(),
        fields.clone(),
        &mut budget,
    )
    .unwrap();
    assert_eq!(
        variant
            .construct(&vec![empty.clone(); 3], &mut budget)
            .unwrap(),
        value
    );
    assert_eq!(
        variant.extract(&value, &mut budget).unwrap(),
        vec![empty; 3]
    );
    assert!(variant.construct(&[], &mut budget).is_err());
    for invalid in [
        vec![Field {
            name: "outside".into(),
            start: 9,
            width: 0,
        }],
        vec![fields[0].clone(), fields[0].clone()],
    ] {
        assert!(
            Variant::new(
                "bad".into(),
                mask.clone(),
                value.clone(),
                invalid,
                &mut budget
            )
            .is_err()
        );
    }
}

#[test]
fn contiguous_niche_finds_largest_gap_for_scalar_widths() {
    let mut budget = Budget::default();

    // Packet { empty = 0, data: 0xA (payload: Nibble) }
    let empty = Variant::new(
        "empty".into(),
        bits(8, 0xff),
        bits(8, 0),
        vec![],
        &mut budget,
    )
    .unwrap();
    let data = Variant::new(
        "data".into(),
        bits(8, 0xf0),
        bits(8, 0xa0),
        vec![Field {
            name: "payload".into(),
            start: 0,
            width: 4,
        }],
        &mut budget,
    )
    .unwrap();
    let sparse_u8 = Layout::new(8, vec![empty, data], &mut budget).unwrap();
    assert_eq!(sparse_u8.contiguous_niche(), Some((1, 159)));

    // Exhaustive u8 coverage leaves no niche.
    let low = Variant::new(
        "low".into(),
        bits(8, 0x80),
        bits(8, 0x00),
        vec![Field {
            name: "lo".into(),
            start: 0,
            width: 7,
        }],
        &mut budget,
    )
    .unwrap();
    let high = Variant::new(
        "high".into(),
        bits(8, 0x80),
        bits(8, 0x80),
        vec![Field {
            name: "hi".into(),
            start: 0,
            width: 7,
        }],
        &mut budget,
    )
    .unwrap();
    let full_u8 = Layout::new(8, vec![low, high], &mut budget).unwrap();
    assert_eq!(full_u8.contiguous_niche(), None);

    // 32-bit RISC-V instructions leave large contiguous gaps disjoint from every valid encoding.
    let riscv = Layout::new(
        32,
        vec![instruction("add", 0x33), instruction("sub", 0x40000033)],
        &mut budget,
    )
    .unwrap();
    let (start, count) = riscv.contiguous_niche().unwrap();
    assert!(count > 0);
    let witness = bits(32, start as u32);
    assert!(matches!(
        riscv.discriminate(&witness, &mut budget),
        Err(Error::InvalidValue)
    ));

    // Non-machine widths (e.g., 12 bits) do not publish scalar niches.
    let narrow = Layout::new(
        12,
        vec![
            Variant::new(
                "zero".into(),
                bits(12, 0xfff),
                bits(12, 0),
                vec![],
                &mut budget,
            )
            .unwrap(),
        ],
        &mut budget,
    )
    .unwrap();
    assert_eq!(narrow.contiguous_niche(), None);
}

#[test]
fn discriminant_intervals_decompose_contiguous_and_residual_variants_exactly() {
    use nir::enum_encoding::merge_contiguous_ranges;
    let mut budget = Budget::default();

    let v_const = |name: &str, val: u32, budget: &mut Budget| {
        Variant::new(name.into(), bits(8, 0xff), bits(8, val), vec![], budget).unwrap()
    };
    let v_p2 = Variant::new(
        "p2".into(),
        bits(8, 0xf0),
        bits(8, 0x20),
        vec![Field {
            name: "x".into(),
            start: 0,
            width: 4,
        }],
        &mut budget,
    )
    .unwrap();
    let v_p3 = Variant::new(
        "p3".into(),
        bits(8, 0xf0),
        bits(8, 0x30),
        vec![Field {
            name: "y".into(),
            start: 0,
            width: 4,
        }],
        &mut budget,
    )
    .unwrap();
    let v_mid = Variant::new(
        "mid".into(),
        bits(8, 0xc3),
        bits(8, 0x81),
        vec![Field {
            name: "z".into(),
            start: 2,
            width: 4,
        }],
        &mut budget,
    )
    .unwrap();
    let variants = vec![
        v_const("c10", 0x10, &mut budget),
        v_const("c11", 0x11, &mut budget),
        v_const("c12", 0x12, &mut budget),
        v_const("c13", 0x13, &mut budget),
        v_p2,
        v_p3,
        v_mid,
    ];
    let layout = Layout::new(8, variants, &mut budget).unwrap();

    // Constants 0x10..=0x13 plus prefix-tagged 0x20..=0x2F and 0x30..=0x3F merge into two exact
    // ranges while middle-field `mid` (0b10_zzzz_01) stays residual.
    let decomposed = layout
        .discriminant_intervals(&[5, 0, 6, 3, 1, 4, 2, 0])
        .unwrap();
    assert_eq!(decomposed.ranges, vec![(0x10, 0x13), (0x20, 0x3f)]);
    assert_eq!(decomposed.residual_variants, vec![6]);

    for x in 0u32..=255 {
        let bv = bits(8, x);
        let in_decomposed = decomposed
            .ranges
            .iter()
            .any(|&(lo, hi)| (lo..=hi).contains(&u128::from(x)))
            || decomposed
                .residual_variants
                .iter()
                .any(|&idx| layout.variants()[idx].accepts(&bv, &mut budget).unwrap());
        let expected = layout.discriminate(&bv, &mut budget).is_ok();
        assert_eq!(in_decomposed, expected, "mismatch at byte {x:#04x}");
    }

    assert!(layout.discriminant_intervals(&[99]).is_none());

    // 128-bit range boundary check: 0 and u128::MAX must not overflow.
    let zero128 = Variant::new(
        "z".into(),
        BitVec::from_words(128, &[u32::MAX; 4], &mut budget).unwrap(),
        BitVec::from_words(128, &[0; 4], &mut budget).unwrap(),
        vec![],
        &mut budget,
    )
    .unwrap();
    let max128 = Variant::new(
        "m".into(),
        BitVec::from_words(128, &[u32::MAX; 4], &mut budget).unwrap(),
        BitVec::from_words(128, &[u32::MAX; 4], &mut budget).unwrap(),
        vec![],
        &mut budget,
    )
    .unwrap();
    let layout128 = Layout::new(128, vec![zero128, max128], &mut budget).unwrap();
    let dec128 = layout128.discriminant_intervals(&[0, 1]).unwrap();
    assert_eq!(dec128.ranges, vec![(0, 0), (u128::MAX, u128::MAX)]);
    assert!(dec128.residual_variants.is_empty());

    assert_eq!(
        merge_contiguous_ranges(vec![(u128::MAX - 1, u128::MAX - 1), (u128::MAX, u128::MAX)]),
        vec![(u128::MAX - 1, u128::MAX)]
    );
}

#[test]
fn is_exhaustive_detects_full_and_sparse_encodings_across_widths() {
    let mut budget = Budget::default();

    // Full 2-bit split: 0b00, 0b01, 0b1x (mask 0b10 val 0b10 with 1 free bit).
    let v00 = Variant::new(
        "a".into(),
        bits(2, 0b11),
        bits(2, 0b00),
        vec![],
        &mut budget,
    )
    .unwrap();
    let v01 = Variant::new(
        "b".into(),
        bits(2, 0b11),
        bits(2, 0b01),
        vec![],
        &mut budget,
    )
    .unwrap();
    let v1x = Variant::new(
        "c".into(),
        bits(2, 0b10),
        bits(2, 0b10),
        vec![Field {
            name: "p".into(),
            start: 0,
            width: 1,
        }],
        &mut budget,
    )
    .unwrap();
    let full2 = Layout::new(2, vec![v00.clone(), v01, v1x], &mut budget).unwrap();
    assert!(full2.is_exhaustive());

    // Missing 0b01 -> non-exhaustive.
    let sparse2 = Layout::new(2, vec![v00], &mut budget).unwrap();
    assert!(!sparse2.is_exhaustive());

    // Exact 128-bit split (two variants covering 2^127 patterns each).
    let low_zero = Variant::new(
        "even".into(),
        BitVec::from_words(128, &[1, 0, 0, 0], &mut budget).unwrap(),
        BitVec::from_words(128, &[0, 0, 0, 0], &mut budget).unwrap(),
        vec![Field {
            name: "hi".into(),
            start: 1,
            width: 127,
        }],
        &mut budget,
    )
    .unwrap();
    let low_one = Variant::new(
        "odd".into(),
        BitVec::from_words(128, &[1, 0, 0, 0], &mut budget).unwrap(),
        BitVec::from_words(128, &[1, 0, 0, 0], &mut budget).unwrap(),
        vec![Field {
            name: "hi".into(),
            start: 1,
            width: 127,
        }],
        &mut budget,
    )
    .unwrap();
    let full128 = Layout::new(128, vec![low_zero.clone(), low_one], &mut budget).unwrap();
    assert!(full128.is_exhaustive());
    let half128 = Layout::new(128, vec![low_zero], &mut budget).unwrap();
    assert!(!half128.is_exhaustive());

    // >128-bit encoding: 130-bit split across top bit.
    let v130_a = Variant::new(
        "lo".into(),
        BitVec::from_words(130, &[0, 0, 0, 0, 0b10], &mut budget).unwrap(),
        BitVec::from_words(130, &[0, 0, 0, 0, 0], &mut budget).unwrap(),
        vec![Field {
            name: "payload".into(),
            start: 0,
            width: 129,
        }],
        &mut budget,
    )
    .unwrap();
    let v130_b = Variant::new(
        "hi".into(),
        BitVec::from_words(130, &[0, 0, 0, 0, 0b10], &mut budget).unwrap(),
        BitVec::from_words(130, &[0, 0, 0, 0, 0b10], &mut budget).unwrap(),
        vec![Field {
            name: "payload".into(),
            start: 0,
            width: 129,
        }],
        &mut budget,
    )
    .unwrap();
    let full130 = Layout::new(130, vec![v130_a.clone(), v130_b], &mut budget).unwrap();
    assert!(full130.is_exhaustive());
    let half130 = Layout::new(130, vec![v130_a], &mut budget).unwrap();
    assert!(!half130.is_exhaustive());
}
