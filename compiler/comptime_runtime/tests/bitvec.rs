use comptime_runtime::{
    Budget,
    bitvec::{BitVec, Error},
};
fn vector(bits: &[bool]) -> BitVec {
    let mut words = vec![0; bits.len().div_ceil(32)];
    for (i, bit) in bits.iter().enumerate() {
        if *bit {
            words[i / 32] |= 1 << (i % 32);
        }
    }
    BitVec::from_words(bits.len(), &words, &mut Budget::default()).unwrap()
}
fn bits(value: &BitVec) -> Vec<bool> {
    (0..value.width())
        .map(|i| value.words()[i / 32] & (1 << (i % 32)) != 0)
        .collect()
}
#[test]
fn cross_word_slices_and_indexing_match_bit_sequence_oracle() {
    for width in 0..=160 {
        let input: Vec<_> = (0..width).map(|i| (i * 7 + i / 3) % 11 < 5).collect();
        let v = vector(&input);
        for lo in 0..=width {
            for hi in lo..=width {
                assert_eq!(
                    bits(&v.slice(lo, hi, &mut Budget::default()).unwrap()),
                    input[lo..hi]
                );
            }
        }
        for i in 0..width {
            assert_eq!(bits(&v.bit(i, &mut Budget::default()).unwrap()), [input[i]]);
        }
        assert_eq!(
            v.bit(width, &mut Budget::default()),
            Err(Error::OutOfBounds)
        );
    }
}
#[test]
fn concatenation_places_right_operand_low_without_host_endianness() {
    for m in 0..=97 {
        for n in 0..=97 {
            let high: Vec<_> = (0..m).map(|i| i % 3 == 0).collect();
            let low: Vec<_> = (0..n).map(|i| i % 7 < 3).collect();
            let joined = vector(&high)
                .concat(&vector(&low), &mut Budget::default())
                .unwrap();
            assert_eq!(
                bits(&joined),
                low.iter().chain(&high).copied().collect::<Vec<_>>()
            );
        }
    }
    let a = BitVec::from_words(8, &[0xaf], &mut Budget::default()).unwrap();
    let b = BitVec::from_words(8, &[0x3c], &mut Budget::default()).unwrap();
    assert_eq!(
        a.concat(&b, &mut Budget::default()).unwrap().words(),
        &[0xaf3c]
    );
}
#[test]
fn logical_operations_mask_padding_and_widening_never_narrows() {
    for width in 0..=160 {
        let a: Vec<_> = (0..width).map(|i| i % 3 == 0).collect();
        let b: Vec<_> = (0..width).map(|i| i % 5 == 0).collect();
        let x = vector(&a);
        let y = vector(&b);
        assert_eq!(
            bits(&x.not(&mut Budget::default()).unwrap()),
            a.iter().map(|a| !a).collect::<Vec<_>>()
        );
        assert_eq!(
            bits(&x.and(&y, &mut Budget::default()).unwrap()),
            a.iter().zip(&b).map(|(a, b)| a & b).collect::<Vec<_>>()
        );
        assert_eq!(
            bits(&x.or(&y, &mut Budget::default()).unwrap()),
            a.iter().zip(&b).map(|(a, b)| a | b).collect::<Vec<_>>()
        );
        let widened = x.widen(width + 37, &mut Budget::default()).unwrap();
        let mut expected = a;
        expected.resize(width + 37, false);
        assert_eq!(bits(&widened), expected);
        assert_eq!(
            widened.widen(width, &mut Budget::default()),
            Err(Error::Narrowing)
        );
        // Equal zero values with different widths are distinct typed values.
        assert_eq!(
            x.and(&widened, &mut Budget::default()),
            Err(Error::WidthMismatch)
        );
        let inverted = x.not(&mut Budget::default()).unwrap();
        assert!(BitVec::from_words(width, inverted.words(), &mut Budget::default()).is_ok());
    }
}
#[test]
fn invalid_inputs_and_resource_exhaustion_preserve_original_values() {
    for (width, words) in [(0, vec![0]), (1, vec![2]), (32, vec![]), (33, vec![0, 2])] {
        assert_eq!(
            BitVec::from_words(width, &words, &mut Budget::default()),
            Err(Error::InvalidRepresentation)
        );
    }
    let v = vector(&[true, false, true]);
    let original = v.clone();
    for (lo, hi) in [(4, 4), (2, 1), (0, usize::MAX)] {
        assert_eq!(
            v.slice(lo, hi, &mut Budget::default()),
            Err(Error::OutOfBounds)
        );
    }
    let mut budget = Budget {
        steps: 0,
        max_limbs: 10,
    };
    assert_eq!(
        v.not(&mut budget),
        Err(Error::Resource(comptime_runtime::Error::BudgetExhausted))
    );
    assert_eq!(
        v.widen(usize::MAX, &mut Budget::default()),
        Err(Error::Resource(comptime_runtime::Error::SizeLimit))
    );
    assert_eq!(v, original);
    assert_eq!(v.not(&mut Budget::default()).unwrap().words(), &[2]);
    assert_eq!(v.to_string(), "(bitvec (width 3) (hex \"00000005\"))");
    let empty = vector(&[]);
    assert_eq!(empty.not(&mut Budget::default()).unwrap(), empty);
    assert_eq!(empty.slice(0, 0, &mut Budget::default()).unwrap(), empty);
}

#[test]
fn integer_literal_conversion_checks_range_without_host_width_truncation() {
    use comptime_runtime::Integer;
    let wide =
        Integer::parse_literal(&format!("0x1{}", "0".repeat(256)), &mut Budget::default()).unwrap();
    let value = wide.to_bitvec(1025, &mut Budget::default()).unwrap();
    assert_eq!(value.words().len(), 33);
    assert_eq!(value.words()[32], 1);
    assert!(value.words()[..32].iter().all(|w| *w == 0));
    assert_eq!(
        wide.to_bitvec(1024, &mut Budget::default()),
        Err(Error::InvalidRepresentation)
    );
    assert_eq!(
        Integer::from_i128(-1).to_bitvec(1025, &mut Budget::default()),
        Err(Error::InvalidRepresentation)
    );
    assert_eq!(
        Integer::zero()
            .to_bitvec(0, &mut Budget::default())
            .unwrap(),
        vector(&[])
    );
    assert_eq!(
        Integer::from_u128(1).to_bitvec(0, &mut Budget::default()),
        Err(Error::InvalidRepresentation)
    );
    let zeros = Integer::parse_literal("0x0001", &mut Budget::default()).unwrap();
    assert_eq!(
        zeros.to_bitvec(1, &mut Budget::default()).unwrap(),
        vector(&[true])
    );
    assert_eq!(
        zeros.to_bitvec(
            64,
            &mut Budget {
                steps: 100,
                max_limbs: 1
            }
        ),
        Err(Error::Resource(comptime_runtime::Error::SizeLimit))
    );
}

#[test]
fn replacement_preserves_every_unselected_bit_against_sequence_oracle() {
    for width in 0..=130 {
        let original: Vec<_> = (0..width).map(|i| i % 5 < 2).collect();
        let input = vector(&original);
        for start in 0..=width {
            for end in start..=width {
                let replacement: Vec<_> = (start..end).map(|i| i % 3 == 0).collect();
                let mut expected = original.clone();
                expected[start..end].copy_from_slice(&replacement);
                let changed = input
                    .replaced(start, &vector(&replacement), &mut Budget::default())
                    .unwrap();
                assert_eq!(bits(&changed), expected, "width {width}, [{start}..{end})");
                assert_eq!(bits(&input), original);
                assert!(BitVec::from_words(width, changed.words(), &mut Budget::default()).is_ok());
            }
        }
    }
    let value = vector(&[true, false, true]);
    let rhs = vector(&[false, true]);
    assert_eq!(
        value.replaced(2, &rhs, &mut Budget::default()),
        Err(Error::OutOfBounds)
    );
    assert_eq!(
        value.replaced(usize::MAX, &rhs, &mut Budget::default()),
        Err(Error::OutOfBounds)
    );
    assert_eq!(
        value.replaced(
            0,
            &rhs,
            &mut Budget {
                steps: 0,
                max_limbs: 4
            }
        ),
        Err(Error::Resource(comptime_runtime::Error::BudgetExhausted))
    );
    assert_eq!(bits(&value), [true, false, true]);
}
