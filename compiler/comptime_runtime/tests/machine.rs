use comptime_runtime::machine::{Error, MachineInteger as M};
use comptime_runtime::{Budget, Error as ResourceError};

#[test]
fn representations_are_checked_and_numeric_conversion_is_not_reinterpretation() {
    for width in 1..=128 {
        let high = 1u128 << (width - 1);
        let mask = if width == 128 {
            u128::MAX
        } else {
            (1u128 << width) - 1
        };
        let minimum = if width == 128 {
            i128::MIN
        } else {
            -(high as i128)
        };
        let maximum = (high - 1) as i128;
        assert_eq!(M::from_signed(minimum, width).unwrap().bits(), high);
        assert_eq!(
            M::from_bits(high, width, true).unwrap().signed_value(),
            Some(minimum)
        );
        assert_eq!(
            M::from_signed(maximum, width).unwrap().signed_value(),
            Some(maximum)
        );
        assert_eq!(
            M::from_bits(mask, width, true).unwrap().signed_value(),
            Some(-1)
        );
        assert_eq!(
            M::from_unsigned(mask, width).unwrap().unsigned_value(),
            Some(mask)
        );
        assert_eq!(
            M::from_signed(-1, width).unwrap().convert(width, false),
            Err(Error::OutOfRange)
        );
        assert_eq!(
            M::from_unsigned(high, width).unwrap().convert(width, true),
            Err(Error::OutOfRange)
        );
        if width < 128 {
            assert_eq!(M::from_unsigned(mask + 1, width), Err(Error::OutOfRange));
            assert_eq!(M::from_signed(maximum + 1, width), Err(Error::OutOfRange));
            assert_eq!(M::from_signed(minimum - 1, width), Err(Error::OutOfRange));
        }
    }
    for width in [0, 129, u16::MAX] {
        assert_eq!(M::from_bits(0, width, false), Err(Error::InvalidWidth));
    }
    assert_eq!(
        M::from_signed(-7, 8)
            .unwrap()
            .convert(128, true)
            .unwrap()
            .signed_value(),
        Some(-7)
    );
    assert_eq!(
        M::from_unsigned(255, 8)
            .unwrap()
            .convert(16, true)
            .unwrap()
            .signed_value(),
        Some(255)
    );
    assert_eq!(
        M::from_unsigned(255, 8).unwrap().convert(7, false),
        Err(Error::OutOfRange)
    );
    assert_eq!(
        M::from_signed(-1, 8).unwrap().to_string(),
        "(machine-integer (signed true) (width 8) (bits 0xff))"
    );
}

#[test]
fn all_byte_pairs_obey_checked_arithmetic_and_truncating_division() {
    let mut budget = Budget::default();
    for signed in [false, true] {
        let (low, high) = if signed { (-128i32, 127i32) } else { (0, 255) };
        let make = |x: i32| {
            if signed {
                M::from_signed(x as i128, 8)
            } else {
                M::from_unsigned(x as u128, 8)
            }
        };
        let expected = |x: i32| {
            if (low..=high).contains(&x) {
                make(x)
            } else {
                Err(Error::Overflow)
            }
        };
        for a in low..=high {
            let left = make(a).unwrap();
            assert_eq!(left.negate(&mut budget), expected(-a));
            for b in low..=high {
                let right = make(b).unwrap();
                assert_eq!(left.add(right, &mut budget), expected(a + b));
                assert_eq!(left.subtract(right, &mut budget), expected(a - b));
                assert_eq!(left.multiply(right, &mut budget), expected(a * b));
                let division = if b == 0 {
                    Err(Error::DivisionByZero)
                } else {
                    expected(a / b).and_then(|q| expected(a % b).map(|r| (q, r)))
                };
                assert_eq!(
                    left.div_rem_trunc(right, &mut budget),
                    division,
                    "{a}/{b} signed={signed}"
                );
            }
        }
    }
}

#[test]
fn full_width_extrema_never_overflow_the_host_or_wrap() {
    let mut budget = Budget::default();
    let minimum = M::from_signed(i128::MIN, 128).unwrap();
    let maximum = M::from_signed(i128::MAX, 128).unwrap();
    let minus_one = M::from_signed(-1, 128).unwrap();
    let one = M::from_signed(1, 128).unwrap();
    assert_eq!(minimum.negate(&mut budget), Err(Error::Overflow));
    assert_eq!(
        minimum.div_rem_trunc(minus_one, &mut budget),
        Err(Error::Overflow)
    );
    assert_eq!(
        minimum.multiply(minus_one, &mut budget),
        Err(Error::Overflow)
    );
    assert_eq!(maximum.add(one, &mut budget), Err(Error::Overflow));
    assert_eq!(minimum.subtract(one, &mut budget), Err(Error::Overflow));
    assert_eq!(minimum.add(maximum, &mut budget), Ok(minus_one));
    let max = M::from_unsigned(u128::MAX, 128).unwrap();
    let one = M::from_unsigned(1, 128).unwrap();
    let zero = M::from_unsigned(0, 128).unwrap();
    assert_eq!(max.add(one, &mut budget), Err(Error::Overflow));
    assert_eq!(
        max.multiply(M::from_unsigned(2, 128).unwrap(), &mut budget),
        Err(Error::Overflow)
    );
    assert_eq!(zero.subtract(one, &mut budget), Err(Error::Overflow));
    assert_eq!(max.div_rem_trunc(one, &mut budget), Ok((max, zero)));
}

#[test]
fn failures_consume_budget_without_changing_values_and_formats_are_not_coerced() {
    let left = M::from_unsigned(255, 8).unwrap();
    let right = M::from_unsigned(1, 8).unwrap();
    let mut budget = Budget {
        steps: 3,
        max_limbs: 0,
    };
    assert_eq!(left.add(right, &mut budget), Err(Error::Overflow));
    assert_eq!(
        left.add(M::from_unsigned(1, 16).unwrap(), &mut budget),
        Err(Error::RepresentationMismatch)
    );
    assert_eq!(
        left.add(M::from_signed(1, 8).unwrap(), &mut budget),
        Err(Error::RepresentationMismatch)
    );
    assert_eq!(
        left.subtract(right, &mut budget),
        Err(Error::Resource(ResourceError::BudgetExhausted))
    );
    assert_eq!(left.bits(), 255);
    assert_eq!(
        left.subtract(right, &mut Budget::default()).unwrap().bits(),
        254
    );
}
