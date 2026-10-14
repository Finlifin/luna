use comptime_runtime::{Budget, Error, Integer};
fn parse(text: &str) -> Integer {
    Integer::parse_literal(text, &mut Budget::default()).unwrap()
}

#[test]
fn radices_and_separators_preserve_one_canonical_value() {
    for spelling in [
        "255",
        "000255",
        "2__55",
        "0xFF",
        "0xff",
        "0b1111__1111",
        "0o377",
    ] {
        assert_eq!(parse(spelling), Integer::from_u128(255));
    }
    for spelling in ["0", "000", "0x00", "0o0", "0b0"] {
        assert_eq!(parse(spelling), Integer::zero());
        assert!(!parse(spelling).negated().is_negative());
    }
    for spelling in [
        "", "_1", "1_", "+1", "-1", "0x", "0Xff", "0x_FF", "0xFf", "0b2", "0o8", "1.0", "12u32",
        "１", "1 2",
    ] {
        assert_eq!(
            Integer::parse_literal(spelling, &mut Budget::default()),
            Err(Error::InvalidLiteral),
            "{spelling}"
        );
    }
}

#[test]
fn values_beyond_host_width_are_exact_across_radices() {
    let decimal =
        parse("115792089237316195423570985008687907853269984665640564039457584007913129639936");
    let hex = parse(&format!("0x1{}", "0".repeat(64)));
    let binary = parse(&format!("0b1{}", "0".repeat(256)));
    assert_eq!(decimal, hex);
    assert_eq!(hex, binary);
    assert_eq!(hex.bit_len(), 257);
    assert_eq!(hex.to_u128(), Err(Error::OutOfRange));
    assert_eq!(
        hex.to_string(),
        format!("(integer (sign positive) (hex \"1{}\"))", "0".repeat(64))
    );
    assert_eq!(
        hex.negated().to_string(),
        format!("(integer (sign negative) (hex \"1{}\"))", "0".repeat(64))
    );
}

#[test]
fn signed_arithmetic_matches_independent_host_results_over_a_dense_domain() {
    for a in -75i128..=75 {
        for b in -75i128..=75 {
            let x = Integer::from_i128(a);
            let y = Integer::from_i128(b);
            let mut budget = Budget::default();
            assert_eq!(x.cmp(&y), a.cmp(&b));
            assert_eq!(x.add(&y, &mut budget).unwrap().to_i128().unwrap(), a + b);
            assert_eq!(
                x.subtract(&y, &mut budget).unwrap().to_i128().unwrap(),
                a - b
            );
            assert_eq!(
                x.multiply(&y, &mut budget).unwrap().to_i128().unwrap(),
                a * b
            );
            if b != 0 {
                let (q, r) = x.div_rem_trunc(&y, &mut budget).unwrap();
                assert_eq!(q.to_i128().unwrap(), a / b);
                assert_eq!(r.to_i128().unwrap(), a % b);
            }
        }
    }
}

#[test]
fn multi_limb_arithmetic_matches_host_reference_without_overflow() {
    let mut state = 0x1234_5678_abcd_ffffu64;
    for _ in 0..2048 {
        state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
        let a = i128::from(state as i64);
        state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
        let b = i128::from(state as i64);
        let x = Integer::from_i128(a);
        let y = Integer::from_i128(b);
        let mut budget = Budget::default();
        assert_eq!(x.add(&y, &mut budget).unwrap().to_i128().unwrap(), a + b);
        assert_eq!(
            x.subtract(&y, &mut budget).unwrap().to_i128().unwrap(),
            a - b
        );
        assert_eq!(
            x.multiply(&y, &mut budget).unwrap().to_i128().unwrap(),
            a * b
        );
        if b != 0 {
            let (q, r) = x.div_rem_trunc(&y, &mut budget).unwrap();
            assert_eq!(q.to_i128().unwrap(), a / b);
            assert_eq!(r.to_i128().unwrap(), a % b);
        }
    }
}

#[test]
fn large_carries_borrows_and_division_reconstruct_the_dividend() {
    let a = parse(&format!("0x{}", "f".repeat(96)));
    let b = parse("0x10000000000000001");
    let one = Integer::from_u128(1);
    let mut budget = Budget::default();
    let power = a.add(&one, &mut budget).unwrap();
    assert_eq!(power, parse(&format!("0x1{}", "0".repeat(96))));
    assert_eq!(power.subtract(&one, &mut budget).unwrap(), a);
    assert_eq!(
        power.subtract(&power, &mut budget).unwrap(),
        Integer::zero()
    );
    for (x, y) in [
        (a.clone(), b.clone()),
        (a.negated(), b.clone()),
        (a.clone(), b.negated()),
        (a.negated(), b.negated()),
    ] {
        let (q, r) = x.div_rem_trunc(&y, &mut budget).unwrap();
        assert_eq!(
            q.multiply(&y, &mut budget)
                .unwrap()
                .add(&r, &mut budget)
                .unwrap(),
            x
        );
        assert!(r.is_zero() || r.is_negative() == x.is_negative());
        let abs = if r.is_negative() { r.negated() } else { r };
        assert!(abs < b);
    }
    let product = a.multiply(&b, &mut budget).unwrap();
    let (q, r) = product.div_rem_trunc(&b, &mut budget).unwrap();
    assert_eq!(q, a);
    assert_eq!(r, Integer::zero());
}

#[test]
fn fixed_width_conversions_check_both_signed_edges() {
    for width in 1..=127 {
        let signed_max = (1i128 << (width - 1)) - 1;
        let signed_min = -(1i128 << (width - 1));
        assert_eq!(
            Integer::from_i128(signed_min).to_signed(width),
            Ok(signed_min)
        );
        assert_eq!(
            Integer::from_i128(signed_max).to_signed(width),
            Ok(signed_max)
        );
        assert_eq!(
            Integer::from_i128(signed_min - 1).to_signed(width),
            Err(Error::OutOfRange)
        );
        assert_eq!(
            Integer::from_i128(signed_max + 1).to_signed(width),
            Err(Error::OutOfRange)
        );
        let unsigned_max = (1u128 << width) - 1;
        assert_eq!(
            Integer::from_u128(unsigned_max).to_unsigned(width),
            Ok(unsigned_max)
        );
        assert_eq!(
            Integer::from_u128(unsigned_max + 1).to_unsigned(width),
            Err(Error::OutOfRange)
        );
    }
    assert_eq!(Integer::from_i128(i128::MIN).to_i128(), Ok(i128::MIN));
    assert_eq!(Integer::from_u128(u128::MAX).to_u128(), Ok(u128::MAX));
    assert_eq!(
        Integer::from_i128(-1).to_unsigned(128),
        Err(Error::OutOfRange)
    );
    assert_eq!(Integer::zero().to_unsigned(0), Err(Error::OutOfRange));
    assert_eq!(Integer::zero().to_signed(129), Err(Error::OutOfRange));
    let (q, r) = Integer::from_i128(i128::MIN)
        .div_rem_trunc(&Integer::from_i128(-1), &mut Budget::default())
        .unwrap();
    assert_eq!(q.to_u128(), Ok(1u128 << 127));
    assert_eq!(q.to_i128(), Err(Error::OutOfRange));
    assert_eq!(r, Integer::zero());
}

#[test]
fn failure_consumes_budget_without_mutating_values_and_retry_is_explicit() {
    let a = parse("0xffffffffffffffffffffffffffffffff");
    let saved = a.clone();
    let mut empty = Budget {
        steps: 0,
        ..Budget::default()
    };
    assert_eq!(a.add(&a, &mut empty), Err(Error::BudgetExhausted));
    assert_eq!(a.multiply(&a, &mut empty), Err(Error::BudgetExhausted));
    assert_eq!(a.div_rem_trunc(&a, &mut empty), Err(Error::BudgetExhausted));
    assert_eq!(
        Integer::parse_literal("10", &mut empty),
        Err(Error::BudgetExhausted)
    );
    let mut interrupted = Budget {
        steps: 2,
        ..Budget::default()
    };
    assert_eq!(
        a.add(&a.negated(), &mut interrupted),
        Err(Error::BudgetExhausted)
    );
    assert_eq!(interrupted.steps, 0);
    let mut small = Budget {
        max_limbs: 4,
        ..Budget::default()
    };
    assert_eq!(
        a.add(&Integer::from_u128(1), &mut small),
        Err(Error::SizeLimit)
    );
    assert_eq!(a.multiply(&a, &mut small), Err(Error::SizeLimit));
    assert_eq!(
        Integer::parse_literal("0x100000000000000000000000000000000", &mut small),
        Err(Error::SizeLimit)
    );
    assert_eq!(a, saved);
    assert_eq!(
        a.div_rem_trunc(&Integer::zero(), &mut Budget::default()),
        Err(Error::DivisionByZero)
    );
    assert_eq!(
        a.div_rem_trunc(&a, &mut Budget::default()).unwrap(),
        (Integer::from_u128(1), Integer::zero())
    );
}

#[test]
fn wide_arithmetic_matches_independent_arbitrary_precision_oracle() {
    fn signed(text: &str) -> Integer {
        match text.strip_prefix('-') {
            Some(magnitude) => parse(magnitude).negated(),
            None => parse(text),
        }
    }
    for (line, row) in include_str!("fixtures/integers.tsv").lines().enumerate() {
        if row.starts_with('#') {
            continue;
        }
        let values: Vec<_> = row.split('\t').map(signed).collect();
        assert_eq!(values.len(), 7);
        let mut budget = Budget::default();
        let a = &values[0];
        let b = &values[1];
        assert_eq!(a.add(b, &mut budget).unwrap(), values[2], "sum at {line}");
        assert_eq!(
            a.subtract(b, &mut budget).unwrap(),
            values[3],
            "difference at {line}"
        );
        assert_eq!(
            a.multiply(b, &mut budget).unwrap(),
            values[4],
            "product at {line}"
        );
        let (q, r) = a.div_rem_trunc(b, &mut budget).unwrap();
        assert_eq!(q, values[5], "quotient at {line}");
        assert_eq!(r, values[6], "remainder at {line}");
    }
}
