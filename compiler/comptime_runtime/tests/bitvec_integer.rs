use comptime_runtime::{Budget, Error, Integer, bitvec::BitVec};

#[test]
fn unsigned_conversion_round_trips_wide_values_and_canonical_zero() {
    for width in [0usize, 1, 31, 32, 127, 128, 129, 1024] {
        let mut words = vec![u32::MAX; width.div_ceil(32)];
        if width % 32 != 0 {
            *words.last_mut().unwrap() = (1 << (width % 32)) - 1;
        }
        let bits = BitVec::from_words(width, &words, &mut Budget::default()).unwrap();
        let integer = Integer::from_bitvec(&bits, &mut Budget::default()).unwrap();
        assert!(!integer.is_negative());
        assert_eq!(integer.bit_len(), width);
        assert_eq!(
            integer.to_bitvec(width, &mut Budget::default()).unwrap(),
            bits
        );
        if width > 128 {
            assert_eq!(integer.to_u128(), Err(Error::OutOfRange));
        }
        let zero =
            BitVec::from_words(width, &vec![0; words.len()], &mut Budget::default()).unwrap();
        assert_eq!(
            Integer::from_bitvec(&zero, &mut Budget::default()).unwrap(),
            Integer::zero()
        );
    }
}

#[test]
fn conversion_enforces_budget_and_size_before_allocating_magnitude() {
    let bits = BitVec::from_words(256, &[u32::MAX; 8], &mut Budget::default()).unwrap();
    for steps in 0..9 {
        assert_eq!(
            Integer::from_bitvec(
                &bits,
                &mut Budget {
                    steps,
                    max_limbs: 8
                }
            ),
            Err(Error::BudgetExhausted)
        );
    }
    assert_eq!(
        Integer::from_bitvec(
            &bits,
            &mut Budget {
                steps: 100,
                max_limbs: 7
            }
        ),
        Err(Error::SizeLimit)
    );
    assert!(
        Integer::from_bitvec(
            &bits,
            &mut Budget {
                steps: 9,
                max_limbs: 8
            }
        )
        .is_ok()
    );
    assert_eq!(bits.words(), &[u32::MAX; 8]);
}
