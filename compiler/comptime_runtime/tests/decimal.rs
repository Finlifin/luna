use comptime_runtime::{Budget, Error, Integer, decimal::DecimalLiteral};
fn parse(text: &str) -> DecimalLiteral {
    DecimalLiteral::parse(text, &mut Budget::default()).unwrap()
}

#[test]
fn decimal_components_remain_exact_before_binary_conversion() {
    let literal = parse("12__34.56_70e-2");
    assert_eq!(literal.coefficient(), &Integer::from_u128(12345670));
    assert_eq!(literal.exponent(), &Integer::from_i128(-6));
    let huge = parse("1.0e9999999999999999999999999999999999999999");
    assert!(huge.exponent().to_i128().is_err());
    assert_eq!(
        huge.to_f64_bits(&mut Budget::default()),
        Err(Error::OutOfRange)
    );
}

#[test]
fn malformed_decimal_syntax_is_not_accepted_by_the_host_parser() {
    for text in [
        "", "1", "1.", ".5", "1e3", "-1.0", "+1.0", "1_.0", "1._0", "1.0_", "1.0e", "1.0e-",
        "1.0e_2", "1.0e2_", "NaN", "Infinity", "1.0e1e2", "1.0 f32",
    ] {
        assert_eq!(
            DecimalLiteral::parse(text, &mut Budget::default()).unwrap_err(),
            Error::InvalidLiteral,
            "{text}"
        );
    }
}

#[test]
fn direct_f32_conversion_avoids_double_rounding_at_halfway() {
    let halfway = parse("1.000000059604644775390625");
    assert_eq!(
        halfway.to_f32_bits(&mut Budget::default()).unwrap(),
        0x3f800000
    );
    let above = parse("1.0000000596046447753906250000001");
    assert_eq!(
        above.to_f32_bits(&mut Budget::default()).unwrap(),
        0x3f800001
    );
    assert_eq!(
        (f64::from_bits(above.to_f64_bits(&mut Budget::default()).unwrap()) as f32).to_bits(),
        0x3f800000
    );
    let odd_halfway = parse("1.000000178813934326171875");
    assert_eq!(
        odd_halfway.to_f32_bits(&mut Budget::default()).unwrap(),
        0x3f800002
    );
}

#[test]
fn binary_formats_preserve_sign_zero_subnormals_and_known_bits() {
    assert_eq!(
        parse("0.1").to_f32_bits(&mut Budget::default()).unwrap(),
        0x3dcccccd
    );
    assert_eq!(
        parse("0.1").to_f64_bits(&mut Budget::default()).unwrap(),
        0x3fb999999999999a
    );
    assert_eq!(
        parse("0.0")
            .negated()
            .to_f32_bits(&mut Budget::default())
            .unwrap(),
        0x80000000
    );
    assert_eq!(
        parse("0.0")
            .negated()
            .to_f64_bits(&mut Budget::default())
            .unwrap(),
        0x8000000000000000
    );
    assert_eq!(
        parse("1.401298464324817070923729583289916131280e-45")
            .to_f32_bits(&mut Budget::default())
            .unwrap(),
        1
    );
    assert_eq!(
        parse("4.940656458412465441765687928682213723650e-324")
            .to_f64_bits(&mut Budget::default())
            .unwrap(),
        1
    );
    assert_eq!(
        parse("1.0e-9999")
            .negated()
            .to_f64_bits(&mut Budget::default())
            .unwrap(),
        1 << 63
    );
    assert_eq!(
        parse("1.0e39").to_f32_bits(&mut Budget::default()),
        Err(Error::OutOfRange)
    );
    assert_eq!(
        parse("1.0e309").to_f64_bits(&mut Budget::default()),
        Err(Error::OutOfRange)
    );
}

#[test]
fn decimal_budget_bounds_input_and_exact_components() {
    let literal = parse("0.1");
    let mut exhausted = Budget {
        steps: 0,
        max_limbs: 2,
    };
    assert_eq!(
        literal.to_f32_bits(&mut exhausted),
        Err(Error::BudgetExhausted)
    );
    assert_eq!(
        literal.to_f64_bits(&mut exhausted),
        Err(Error::BudgetExhausted)
    );
    assert_eq!(
        literal.to_f32_bits(&mut Budget::default()).unwrap(),
        0x3dcccccd
    );
    assert_eq!(
        DecimalLiteral::parse(
            "1.0",
            &mut Budget {
                steps: 1,
                max_limbs: 2
            }
        )
        .unwrap_err(),
        Error::BudgetExhausted
    );
    assert_eq!(
        DecimalLiteral::parse(
            "12345678901234567890.0",
            &mut Budget {
                steps: 10000,
                max_limbs: 1
            }
        )
        .unwrap_err(),
        Error::SizeLimit
    );
    assert!(
        parse("1.0")
            .to_string()
            .starts_with("(decimal-literal (negative false)")
    );
}
