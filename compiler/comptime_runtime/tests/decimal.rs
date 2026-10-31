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

#[test]
fn exact_decimal_identity_normalizes_spelling_without_losing_signed_zero() {
    use std::collections::HashSet;
    let values = ["1.0", "10.00e-1", "0.010e2", "0001.000_0"]
        .map(|s| parse(s).exact_value(&mut Budget::default()).unwrap());
    assert_eq!(values.iter().cloned().collect::<HashSet<_>>().len(), 1);
    assert_eq!(values[0].digits(), "1");
    assert_eq!(values[0].exponent(), &Integer::zero());
    let zero = parse("0.000e9999999999999999999999999999999")
        .exact_value(&mut Budget::default())
        .unwrap();
    let negative_zero = parse("0.0")
        .negated()
        .exact_value(&mut Budget::default())
        .unwrap();
    assert_ne!(zero, negative_zero);
    assert_eq!(
        zero.compare(&negative_zero, &mut Budget::default())
            .unwrap(),
        std::cmp::Ordering::Equal
    );
    assert_eq!(zero.exponent(), &Integer::zero());
    assert!(negative_zero.is_negative());
    assert_eq!(
        values[0].to_string(),
        "(decimal-value (negative false) (digits \"1\") (exponent (integer (sign positive) (hex \"0\"))))"
    );
}

#[test]
fn exact_decimal_comparison_retains_values_that_share_binary_rounding() {
    use std::cmp::Ordering;
    for (left, right) in [
        (
            "1.0000000000000000000000000000001",
            "1.0000000000000000000000000000002",
        ),
        (
            "9.99e99999999999999999999999999999999",
            "1.0e100000000000000000000000000000000",
        ),
        ("1.001", "1.01"),
        ("0.0", "1.0e-99999999999999999999999999999"),
    ] {
        let a = parse(left).exact_value(&mut Budget::default()).unwrap();
        let b = parse(right).exact_value(&mut Budget::default()).unwrap();
        assert_eq!(
            a.compare(&b, &mut Budget::default()).unwrap(),
            Ordering::Less,
            "{left} {right}"
        );
        assert_eq!(
            b.compare(&a, &mut Budget::default()).unwrap(),
            Ordering::Greater
        );
        let a = parse(left)
            .negated()
            .exact_value(&mut Budget::default())
            .unwrap();
        let b = parse(right)
            .negated()
            .exact_value(&mut Budget::default())
            .unwrap();
        assert_eq!(
            a.compare(&b, &mut Budget::default()).unwrap(),
            Ordering::Greater
        );
    }
}

#[test]
fn exact_decimal_failures_consume_budget_without_changing_inputs() {
    let input = parse("12345.000");
    assert_eq!(
        input.exact_value(&mut Budget {
            steps: 0,
            max_limbs: 10
        }),
        Err(Error::BudgetExhausted)
    );
    let value = input.exact_value(&mut Budget::default()).unwrap();
    let before = value.clone();
    assert_eq!(
        value.compare(
            &before,
            &mut Budget {
                steps: 0,
                max_limbs: 10
            }
        ),
        Err(Error::BudgetExhausted)
    );
    assert_eq!(value, before);
    assert_eq!(
        value.compare(&before, &mut Budget::default()).unwrap(),
        std::cmp::Ordering::Equal
    );
    assert_eq!(
        parse("12345678901234567890.0").exact_value(&mut Budget {
            steps: 100000,
            max_limbs: 1
        }),
        Err(Error::SizeLimit)
    );
}

#[test]
fn exact_decimal_order_matches_scaled_integer_oracle() {
    let mut cases = Vec::new();
    for coefficient in [-999i128, -101, -10, -1, 0, 1, 10, 101, 999] {
        for exponent in -3i32..=3 {
            let spelling = format!("{}.0e{exponent}", coefficient.abs());
            let literal = parse(&spelling);
            let literal = if coefficient < 0 {
                literal.negated()
            } else {
                literal
            };
            cases.push((
                literal.exact_value(&mut Budget::default()).unwrap(),
                coefficient * 10i128.pow((exponent + 3) as u32),
            ));
        }
    }
    for (left, scaled_left) in &cases {
        for (right, scaled_right) in &cases {
            assert_eq!(
                left.compare(right, &mut Budget::default()).unwrap(),
                scaled_left.cmp(scaled_right),
                "{left} vs {right}"
            );
        }
    }
}

#[test]
fn exact_decimal_spelling_round_trips_without_expanding_the_exponent() {
    use comptime_runtime::decimal::DecimalValue;
    for text in [
        "0.0",
        "-0.0",
        "123456789012345678901234567890.123456789",
        "-0.00000000000000000000000000123",
        "1.0e9999999999999999999999999999999999999999",
        "-1.0e-9999999999999999999999999999999999999999",
    ] {
        let value = DecimalValue::parse_literal(text, &mut Budget::default()).unwrap();
        let spelling = value.to_literal(&mut Budget::default()).unwrap();
        assert!(spelling.len() < text.len() + 10, "{spelling}");
        assert_eq!(
            value,
            DecimalValue::parse_literal(&spelling, &mut Budget::default()).unwrap()
        );
        let mut empty = Budget {
            steps: 0,
            max_limbs: 4096,
        };
        assert_eq!(value.to_literal(&mut empty), Err(Error::BudgetExhausted));
        assert!(value.to_literal(&mut Budget::default()).is_ok());
    }
    let value =
        DecimalValue::parse_literal("123456789012345678901234567890.0", &mut Budget::default())
            .unwrap();
    assert_eq!(
        value.to_literal(&mut Budget {
            steps: 10000,
            max_limbs: 1
        }),
        Err(Error::SizeLimit)
    );
}

#[test]
fn exact_decimal_negation_is_budgeted_and_preserves_magnitude() {
    use comptime_runtime::decimal::DecimalValue;
    for text in [
        "0.0",
        "-0.0",
        "123456789012345678901234567890.12345",
        "-1.0e9999999999999999999999999999999999999999",
    ] {
        let original = DecimalValue::parse_literal(text, &mut Budget::default()).unwrap();
        let negative = original.negate(&mut Budget::default()).unwrap();
        assert_ne!(original.is_negative(), negative.is_negative());
        assert_eq!(original.digits(), negative.digits());
        assert_eq!(original.exponent(), negative.exponent());
        assert_eq!(negative.negate(&mut Budget::default()).unwrap(), original);
        assert_eq!(
            original.negate(&mut Budget {
                steps: 0,
                max_limbs: 4096
            }),
            Err(Error::BudgetExhausted)
        );
        assert!(original.negate(&mut Budget::default()).is_ok());
    }
    for text in [
        "123456789012345678901234567890.0",
        "1.0e9999999999999999999999999999",
    ] {
        let value = DecimalValue::parse_literal(text, &mut Budget::default()).unwrap();
        assert_eq!(
            value.negate(&mut Budget {
                steps: 10000,
                max_limbs: 1
            }),
            Err(Error::SizeLimit)
        );
    }
}

#[test]
fn exact_decimal_arithmetic_matches_scaled_integer_oracle() {
    use comptime_runtime::decimal::DecimalValue;
    let value = |n, exponent| {
        DecimalValue::parse_literal(&format!("{n}.0e{exponent}"), &mut Budget::default()).unwrap()
    };
    for a in -12i128..=12 {
        for b in -12i128..=12 {
            for ea in [-2i32, 0, 2] {
                for eb in [-2i32, 0, 2] {
                    let left = value(a, ea);
                    let right = value(b, eb);
                    let scaled_a = a * 10i128.pow((ea + 2) as u32);
                    let scaled_b = b * 10i128.pow((eb + 2) as u32);
                    for (actual, expected) in [
                        (
                            left.add(&right, &mut Budget::default()).unwrap(),
                            value(scaled_a + scaled_b, -2),
                        ),
                        (
                            left.subtract(&right, &mut Budget::default()).unwrap(),
                            value(scaled_a - scaled_b, -2),
                        ),
                        (
                            left.multiply(&right, &mut Budget::default()).unwrap(),
                            value(scaled_a * scaled_b, -4),
                        ),
                    ] {
                        assert_eq!(
                            actual.compare(&expected, &mut Budget::default()).unwrap(),
                            std::cmp::Ordering::Equal,
                            "{a}e{ea} {b}e{eb}: {actual} vs {expected}"
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn exact_decimal_arithmetic_preserves_zero_signs_and_bounds_alignment() {
    use comptime_runtime::decimal::DecimalValue;
    let value = |text| DecimalValue::parse_literal(text, &mut Budget::default()).unwrap();
    let positive = value("0.0");
    let negative = value("-0.0");
    assert_eq!(
        negative.add(&negative, &mut Budget::default()).unwrap(),
        negative
    );
    assert_eq!(
        positive.add(&negative, &mut Budget::default()).unwrap(),
        positive
    );
    assert_eq!(
        negative
            .multiply(&value("-2.0"), &mut Budget::default())
            .unwrap(),
        positive
    );
    assert_eq!(
        negative
            .multiply(&value("2.0"), &mut Budget::default())
            .unwrap(),
        negative
    );
    let huge = value("1.0e9999999999999999999999999999999999999999");
    assert_eq!(
        huge.subtract(&huge, &mut Budget::default()).unwrap(),
        positive
    );
    assert_eq!(huge.add(&positive, &mut Budget::default()).unwrap(), huge);
    assert_eq!(
        huge.add(&value("1.0"), &mut Budget::default()),
        Err(Error::SizeLimit)
    );
    assert_eq!(
        huge.multiply(&negative, &mut Budget::default()).unwrap(),
        negative
    );
    for operation in [
        DecimalValue::add,
        DecimalValue::subtract,
        DecimalValue::multiply,
    ] {
        assert_eq!(
            operation(
                &value("1.25"),
                &value("2.5"),
                &mut Budget {
                    steps: 0,
                    max_limbs: 4096
                }
            ),
            Err(Error::BudgetExhausted)
        );
        assert!(operation(&value("1.25"), &value("2.5"), &mut Budget::default()).is_ok());
    }
    let a = value("1.0000000000000000000000000000001");
    let b = value("1.0");
    assert_eq!(
        a.subtract(&b, &mut Budget::default()).unwrap(),
        value("0.0000000000000000000000000000001")
    );
}

#[test]
fn exact_decimal_arithmetic_rejects_output_growth_without_mutating_operands() {
    use comptime_runtime::decimal::DecimalValue;
    let value = |text| DecimalValue::parse_literal(text, &mut Budget::default()).unwrap();
    let left = value("4294967295.0");
    let right = value("1.0");
    assert_eq!(
        left.add(
            &right,
            &mut Budget {
                steps: 10000,
                max_limbs: 1
            }
        ),
        Err(Error::SizeLimit)
    );
    assert_eq!(
        left.add(&right, &mut Budget::default()).unwrap(),
        value("4294967296.0")
    );
    let factor = value("65536.0");
    assert_eq!(
        factor.multiply(
            &factor,
            &mut Budget {
                steps: 10000,
                max_limbs: 1
            }
        ),
        Err(Error::SizeLimit)
    );
    assert_eq!(
        factor.multiply(&factor, &mut Budget::default()).unwrap(),
        value("4294967296.0")
    );
}

#[test]
fn computed_exact_values_round_directly_to_the_selected_binary_format() {
    use comptime_runtime::decimal::DecimalValue;
    let value = |text: &str| DecimalValue::parse_literal(text, &mut Budget::default()).unwrap();
    let half = value("1.000000059604644775390625");
    let epsilon = value("0.0000000000000000000000000000001");
    let above = half.add(&epsilon, &mut Budget::default()).unwrap();
    let below = half.subtract(&epsilon, &mut Budget::default()).unwrap();
    assert_eq!(
        half.to_f32_bits(&mut Budget::default()).unwrap(),
        0x3f800000
    );
    assert_eq!(
        below.to_f32_bits(&mut Budget::default()).unwrap(),
        0x3f800000
    );
    assert_eq!(
        above.to_f32_bits(&mut Budget::default()).unwrap(),
        0x3f800001
    );
    assert_eq!(
        (f64::from_bits(above.to_f64_bits(&mut Budget::default()).unwrap()) as f32).to_bits(),
        0x3f800000
    );
    let negative = above.negate(&mut Budget::default()).unwrap();
    assert_eq!(
        negative.to_f32_bits(&mut Budget::default()).unwrap(),
        0xbf800001
    );
    let sum = value("0.1")
        .add(&value("0.2"), &mut Budget::default())
        .unwrap();
    assert_eq!(
        sum.to_f32_bits(&mut Budget::default()).unwrap(),
        0.3f32.to_bits()
    );
    assert_eq!(
        sum.to_f64_bits(&mut Budget::default()).unwrap(),
        0.3f64.to_bits()
    );
    for (text, bits32, bits64) in [
        ("0.0", 0, 0),
        ("-0.0", 1u32 << 31, 1u64 << 63),
        ("1.0e-9999999999999999999999999999999999999999", 0, 0),
        (
            "-1.0e-9999999999999999999999999999999999999999",
            1 << 31,
            1 << 63,
        ),
    ] {
        let exact = value(text);
        assert_eq!(exact.to_f32_bits(&mut Budget::default()).unwrap(), bits32);
        assert_eq!(exact.to_f64_bits(&mut Budget::default()).unwrap(), bits64);
    }
    for text in [
        "1.0e9999999999999999999999999999999999999999",
        "-1.0e9999999999999999999999999999999999999999",
    ] {
        assert_eq!(
            value(text).to_f32_bits(&mut Budget::default()),
            Err(Error::OutOfRange)
        );
        assert_eq!(
            value(text).to_f64_bits(&mut Budget::default()),
            Err(Error::OutOfRange)
        );
    }
    assert_eq!(
        above.to_f32_bits(&mut Budget {
            steps: 0,
            max_limbs: 4096
        }),
        Err(Error::BudgetExhausted)
    );
    assert_eq!(
        above.to_f64_bits(&mut Budget {
            steps: 100000,
            max_limbs: 1
        }),
        Err(Error::SizeLimit)
    );
    assert_eq!(
        above.to_f32_bits(&mut Budget::default()).unwrap(),
        0x3f800001
    );
}

#[test]
fn computed_binary_midpoints_follow_ties_to_even_including_subnormal_edges() {
    use comptime_runtime::decimal::DecimalValue;
    let value = |text: &str| DecimalValue::parse_literal(text, &mut Budget::default()).unwrap();
    let half = value("1.00000000000000011102230246251565404236316680908203125");
    let epsilon = value("1.0e-60");
    assert_eq!(
        half.to_f64_bits(&mut Budget::default()).unwrap(),
        0x3ff0000000000000
    );
    assert_eq!(
        half.add(&epsilon, &mut Budget::default())
            .unwrap()
            .to_f64_bits(&mut Budget::default())
            .unwrap(),
        0x3ff0000000000001
    );
    assert_eq!(
        half.subtract(&epsilon, &mut Budget::default())
            .unwrap()
            .to_f64_bits(&mut Budget::default())
            .unwrap(),
        0x3ff0000000000000
    );
    // Independently construct 2^-n as 5^n * 10^-n: midpoint between zero
    // and the smallest subnormal. No binary float calculates the oracle.
    for (power, epsilon) in [(150, "1.0e-200"), (1075, "1.0e-1200")] {
        let mut coefficient = Integer::from_u128(1);
        let mut budget = Budget::default();
        for _ in 0..power {
            coefficient = coefficient
                .multiply(&Integer::from_u128(5), &mut budget)
                .unwrap();
        }
        let decimal = coefficient.to_decimal_literal(&mut budget).unwrap();
        let midpoint = value(&format!("{decimal}.0e-{power}"));
        let above = midpoint
            .add(&value(epsilon), &mut Budget::default())
            .unwrap();
        let below = midpoint
            .subtract(&value(epsilon), &mut Budget::default())
            .unwrap();
        for (number, bits) in [(midpoint, 0), (above, 1), (below, 0)] {
            if power == 150 {
                assert_eq!(number.to_f32_bits(&mut Budget::default()).unwrap(), bits);
                assert_eq!(
                    number
                        .negate(&mut Budget::default())
                        .unwrap()
                        .to_f32_bits(&mut Budget::default())
                        .unwrap(),
                    bits | (1 << 31)
                );
            } else {
                assert_eq!(
                    number.to_f64_bits(&mut Budget::default()).unwrap(),
                    u64::from(bits)
                );
                assert_eq!(
                    number
                        .negate(&mut Budget::default())
                        .unwrap()
                        .to_f64_bits(&mut Budget::default())
                        .unwrap(),
                    u64::from(bits) | (1 << 63)
                );
            }
        }
    }
}

#[test]
fn representation_metadata_rechecks_limits_and_never_changes_identity() {
    use comptime_runtime::decimal::DecimalValue;
    use std::hash::{Hash, Hasher};
    fn hash(value: &DecimalValue) -> u64 {
        let mut hash = std::collections::hash_map::DefaultHasher::new();
        value.hash(&mut hash);
        hash.finish()
    }
    // Multiplication creates a significand with a removable trailing zero.
    let left = DecimalValue::parse_literal("2.0", &mut Budget::default()).unwrap();
    let right = DecimalValue::parse_literal("21474836485.0", &mut Budget::default()).unwrap();
    let value = left.multiply(&right, &mut Budget::default()).unwrap();
    let original = value.clone();
    let debug = format!("{value:?}");
    let identity = hash(&value);
    let mut exhausted = Budget {
        steps: 1,
        max_limbs: 2,
    };
    assert_eq!(
        value.check_representation(&mut exhausted),
        Err(Error::BudgetExhausted)
    );
    assert_eq!(exhausted.steps, 0);
    assert_eq!(
        value.check_representation(&mut Budget {
            steps: 1000,
            max_limbs: 1
        }),
        Err(Error::SizeLimit)
    );
    value
        .check_representation(&mut Budget {
            steps: 1000,
            max_limbs: 2,
        })
        .unwrap();
    value
        .check_representation(&mut Budget {
            steps: 1,
            max_limbs: 2,
        })
        .unwrap();
    assert_eq!(
        value.check_representation(&mut Budget {
            steps: 1,
            max_limbs: 1
        }),
        Err(Error::SizeLimit)
    );
    assert_eq!(value, original);
    assert_eq!(hash(&value), identity);
    assert_eq!(format!("{value:?}"), debug);
    assert_eq!(
        value.to_literal(&mut Budget::default()).unwrap(),
        "4294967297.0e1"
    );
}

#[test]
fn known_decimal_export_uses_linear_copy_fuel_and_checks_both_size_limits() {
    use comptime_runtime::decimal::DecimalValue;
    let digits = "7".repeat(2048);
    let text = format!("{digits}.0");
    let value = DecimalValue::parse_literal(&text, &mut Budget::default()).unwrap();
    let mut linear = Budget {
        steps: digits.len() * 2 + 64,
        max_limbs: 4096,
    };
    assert_eq!(
        value.to_literal(&mut linear).unwrap(),
        format!("{digits}.0e0")
    );
    assert_eq!(
        value.to_literal(&mut Budget {
            steps: 10000,
            max_limbs: 1
        }),
        Err(Error::SizeLimit)
    );
    let huge_exponent =
        DecimalValue::parse_literal("1.0e4294967296", &mut Budget::default()).unwrap();
    assert_eq!(
        huge_exponent.check_representation(&mut Budget {
            steps: 1,
            max_limbs: 1
        }),
        Err(Error::SizeLimit)
    );
    huge_exponent
        .check_representation(&mut Budget {
            steps: 1,
            max_limbs: 2,
        })
        .unwrap();
    let zero = DecimalValue::parse_literal("-0.0", &mut Budget::default()).unwrap();
    zero.check_representation(&mut Budget {
        steps: 1,
        max_limbs: 0,
    })
    .unwrap();
}
