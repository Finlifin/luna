use comptime_runtime::{Budget, Error, decimal::DecimalValue, float16::Float16Format as F};

fn value(text: &str) -> DecimalValue {
    DecimalValue::parse_literal(text, &mut Budget::default()).unwrap()
}

#[test]
fn exact_midpoints_do_not_double_round_through_f32_or_f64() {
    for (format, even, odd, cases) in [
        (
            F::Binary16,
            0x3c00,
            0x3c01,
            [
                "1.00048828124999999999999999999999999999",
                "1.00048828125",
                "1.00048828125000000000000000000000000001",
            ],
        ),
        (
            F::BFloat16,
            0x3f80,
            0x3f81,
            [
                "1.00390624999999999999999999999999999999",
                "1.00390625",
                "1.00390625000000000000000000000000000001",
            ],
        ),
    ] {
        for (text, expected) in cases.into_iter().zip([even, even, odd]) {
            let v = value(text);
            assert_eq!(format.round(&v, &mut Budget::default()).unwrap(), expected);
            assert_eq!(
                format
                    .round(
                        &v.negate(&mut Budget::default()).unwrap(),
                        &mut Budget::default()
                    )
                    .unwrap(),
                expected | 0x8000
            );
        }
    }
    // A midpoint whose lower neighbor is odd rounds upward.
    assert_eq!(
        value("1.00146484375")
            .to_f16_bits(&mut Budget::default())
            .unwrap(),
        0x3c02
    );
    assert_eq!(
        value("1.01171875")
            .to_bf16_bits(&mut Budget::default())
            .unwrap(),
        0x3f82
    );
}

#[test]
fn binary16_subnormal_normal_and_overflow_boundaries() {
    for (text, expected) in [
        ("0.0", 0),
        ("1.0", 0x3c00),
        ("65504.0", 0x7bff),
        ("65519.9999999999999999999999999", 0x7bff),
        ("0.000000059604644775390625", 1),
        ("0.0000000298023223876953125", 0),
        ("0.00000002980232238769531250000001", 1),
        ("0.00006103515625", 0x0400),
        ("0.0000610053539276123046875", 0x0400),
    ] {
        assert_eq!(
            value(text).to_f16_bits(&mut Budget::default()).unwrap(),
            expected,
            "{text}"
        );
    }
    assert_eq!(
        value("65520.0").to_f16_bits(&mut Budget::default()),
        Err(Error::OutOfRange)
    );
}

#[test]
fn finite_encodings_decode_exactly_across_both_exponent_ranges() {
    for (format, fraction, bias, infinity) in
        [(F::Binary16, 10, 15, 0x7c00), (F::BFloat16, 7, 127, 0x7f80)]
    {
        // Every exponent boundary, neighboring mantissas and both signs.
        for exponent in 0..(infinity >> fraction) {
            for mantissa in [0, 1, (1 << fraction) - 2, (1 << fraction) - 1] {
                for sign in [0, 0x8000] {
                    let bits = sign | (exponent << fraction) | mantissa;
                    let exact = format.to_exact(bits, &mut Budget::default()).unwrap();
                    let expected = if exponent == 0 {
                        f64::from(mantissa) * 2f64.powi(1 - bias - fraction)
                    } else {
                        f64::from((1 << fraction) | mantissa)
                            * 2f64.powi(i32::from(exponent) - bias - fraction)
                    };
                    let expected = if sign == 0 { expected } else { -expected };
                    assert_eq!(
                        exact.to_f64_bits(&mut Budget::default()).unwrap(),
                        expected.to_bits(),
                        "{format} {bits:04x}"
                    );
                    assert_eq!(
                        format.round(&exact, &mut Budget::default()).unwrap(),
                        bits,
                        "{format} {bits:04x}"
                    );
                }
            }
        }
        for bits in [infinity, infinity | 1, infinity | 0x8000, 0xffff] {
            assert_eq!(
                format.to_exact(bits, &mut Budget::default()),
                Err(Error::OutOfRange)
            );
        }
    }
}

#[test]
fn huge_exponents_signed_underflow_and_budget_failure_recover() {
    for format in [F::Binary16, F::BFloat16] {
        assert_eq!(
            format.round(
                &value("1.0e9999999999999999999999999999999999999"),
                &mut Budget::default()
            ),
            Err(Error::OutOfRange)
        );
        let tiny = value("1.0e-9999999999999999999999999999999999999");
        assert_eq!(format.round(&tiny, &mut Budget::default()).unwrap(), 0);
        assert_eq!(
            format
                .round(
                    &tiny.negate(&mut Budget::default()).unwrap(),
                    &mut Budget::default()
                )
                .unwrap(),
            0x8000
        );
        let v = value("1.0");
        let snapshot = v.to_string();
        assert_eq!(
            format.round(
                &v,
                &mut Budget {
                    steps: 1,
                    max_limbs: 4096
                }
            ),
            Err(Error::BudgetExhausted)
        );
        assert_eq!(
            format.round(
                &v,
                &mut Budget {
                    steps: 1_000_000,
                    max_limbs: 0
                }
            ),
            Err(Error::SizeLimit)
        );
        assert_eq!(v.to_string(), snapshot);
        assert!(format.round(&v, &mut Budget::default()).is_ok());
    }
}

#[test]
fn bfloat16_subnormal_and_finite_overflow_midpoints() {
    assert_eq!(
        value("338953138925153547590470800371487866880.0")
            .to_bf16_bits(&mut Budget::default())
            .unwrap(),
        0x7f7f
    );
    assert_eq!(
        value("339617752923046005526922703901628039167.0")
            .to_bf16_bits(&mut Budget::default())
            .unwrap(),
        0x7f7f
    );
    assert_eq!(value("918354961579912115600575419704879435795832466228193376178712270530013483949005603790283203125.0e-133").to_bf16_bits(&mut Budget::default()).unwrap(), 0x0001);
    assert_eq!(value("4591774807899560578002877098524397178979162331140966880893561352650067419745028018951416015625.0e-134").to_bf16_bits(&mut Budget::default()).unwrap(), 0x0000);
    assert_eq!(value("45917748078995605780028770985243971789791623311409668808935613526500674197450280189514160156251.0e-135").to_bf16_bits(&mut Budget::default()).unwrap(), 0x0001);
    assert_eq!(value("11754943508222875079687365372222456778186655567720875215087517062784172594547271728515625.0e-126").to_bf16_bits(&mut Budget::default()).unwrap(), 0x0080);
    assert_eq!(value("1170902576014387947390733660123721280639686394440946554627858144925767192034982144832611083984375.0e-134").to_bf16_bits(&mut Budget::default()).unwrap(), 0x0080);
    assert_eq!(
        value("339617752923046005526922703901628039168.0").to_bf16_bits(&mut Budget::default()),
        Err(Error::OutOfRange)
    );
}

#[test]
fn comparisons_cover_every_encoding_against_an_exact_widening_oracle() {
    fn oracle(bits: u16, fraction: u32, exponent_bits: u32, bias: i32) -> f64 {
        let exponent = (u32::from(bits) >> fraction) & ((1 << exponent_bits) - 1);
        let mantissa = u32::from(bits) & ((1 << fraction) - 1);
        let value = if exponent == (1 << exponent_bits) - 1 {
            if mantissa == 0 {
                f64::INFINITY
            } else {
                f64::NAN
            }
        } else if exponent == 0 {
            f64::from(mantissa) * 2.0f64.powi(1 - bias - fraction as i32)
        } else {
            f64::from((1 << fraction) | mantissa)
                * 2.0f64.powi(exponent as i32 - bias - fraction as i32)
        };
        if bits & 0x8000 == 0 { value } else { -value }
    }
    for (format, fraction, exponent, bias, infinity) in [
        (F::Binary16, 10, 5, 15, 0x7c00),
        (F::BFloat16, 7, 8, 127, 0x7f80),
    ] {
        for bits in 0..=u16::MAX {
            for other in [
                bits,
                bits ^ 0x8000,
                bits.wrapping_add(1),
                bits.wrapping_mul(40503).wrapping_add(17),
                0,
                0x8000,
                infinity,
                infinity | 0x8000,
                infinity | 1,
                infinity | 0x8001,
            ] {
                assert_eq!(
                    format.compare(bits, other),
                    oracle(bits, fraction, exponent, bias)
                        .partial_cmp(&oracle(other, fraction, exponent, bias)),
                    "{format} {bits:04x} {other:04x}"
                );
            }
        }
    }
}
