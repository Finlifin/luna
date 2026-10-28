use comptime_runtime::{
    Budget, Error,
    float16::{Float16Format as F, Float16Op as Op},
};

fn eval(f: F, op: Op, a: u16, b: u16) -> u16 {
    f.arithmetic(op, a, b, &mut Budget::default()).unwrap()
}

#[test]
fn special_values_signed_zeros_and_canonical_nans() {
    for (f, one, infinity, nan) in [
        (F::Binary16, 0x3c00, 0x7c00, 0x7e00),
        (F::BFloat16, 0x3f80, 0x7f80, 0x7fc0),
    ] {
        for (op, a, b, expected) in [
            (Op::Add, 0x8000, 0x8000, 0x8000),
            (Op::Add, 0, 0x8000, 0),
            (Op::Subtract, 0x8000, 0, 0x8000),
            (Op::Subtract, one, one, 0),
            (Op::Multiply, 0x8000, one, 0x8000),
            (Op::Divide, 0, one | 0x8000, 0x8000),
            (Op::Add, infinity, infinity | 0x8000, nan),
            (Op::Subtract, infinity, infinity, nan),
            (Op::Multiply, infinity, 0, nan),
            (Op::Divide, 0, 0, nan),
            (Op::Divide, infinity, infinity, nan),
            (Op::Divide, one, 0x8000, infinity | 0x8000),
            (Op::Divide, one, infinity | 0x8000, 0x8000),
            (Op::Multiply, infinity, one | 0x8000, infinity | 0x8000),
            (Op::Add, infinity | 0x8000, one, infinity | 0x8000),
        ] {
            assert_eq!(eval(f, op, a, b), expected, "{f} {op} {a:04x} {b:04x}");
        }
        for op in [Op::Add, Op::Subtract, Op::Multiply, Op::Divide] {
            for payload in [infinity | 1, nan, nan | 0x8001] {
                assert_eq!(eval(f, op, payload, one), nan);
                assert_eq!(eval(f, op, one, payload), nan);
            }
        }
    }
}

#[test]
fn midpoint_subnormal_overflow_and_division_round_once() {
    for (f, one, two, three, third, infinity, half_ulp) in [
        (F::Binary16, 0x3c00, 0x4000, 0x4200, 0x3555, 0x7c00, 0x1000),
        (F::BFloat16, 0x3f80, 0x4000, 0x4040, 0x3eab, 0x7f80, 0x3b80),
    ] {
        assert_eq!(eval(f, Op::Divide, one, three), third);
        assert_eq!(eval(f, Op::Add, one, half_ulp), one);
        assert_eq!(eval(f, Op::Add, one + 1, half_ulp), one + 2);
        assert_eq!(eval(f, Op::Divide, 1, two), 0);
        assert_eq!(eval(f, Op::Divide, 0x8001, two), 0x8000);
        assert_eq!(eval(f, Op::Divide, 3, two), 2);
        assert_eq!(eval(f, Op::Add, infinity - 1, infinity - 1), infinity);
        assert_eq!(
            eval(f, Op::Multiply, infinity - 1, two | 0x8000),
            infinity | 0x8000
        );
    }
}

fn decode(bits: u16, fraction: u32, bias: i32, infinity: u16) -> f64 {
    let magnitude = bits & 0x7fff;
    let exponent = i32::from(magnitude >> fraction);
    let mantissa = u32::from(magnitude) & ((1 << fraction) - 1);
    let value = if magnitude >= infinity {
        if magnitude == infinity {
            f64::INFINITY
        } else {
            f64::NAN
        }
    } else if exponent == 0 {
        f64::from(mantissa) * 2.0f64.powi(1 - bias - fraction as i32)
    } else {
        f64::from((1 << fraction) | mantissa) * 2.0f64.powi(exponent - bias - fraction as i32)
    };
    if bits & 0x8000 == 0 { value } else { -value }
}

// Independent exponent quantization oracle; no rational midpoint search.
fn quantize(value: f64, fraction: u32, bias: i32, infinity: u16) -> u16 {
    if value.is_nan() {
        return infinity | (1 << (fraction - 1));
    }
    let sign = if value.is_sign_negative() { 0x8000 } else { 0 };
    let value = value.abs();
    if value.is_infinite() {
        return sign | infinity;
    }
    if value < 2.0f64.powi(1 - bias) {
        return sign | (value / 2.0f64.powi(1 - bias - fraction as i32)).round_ties_even() as u16;
    }
    let mut exponent = ((value.to_bits() >> 52) & 0x7ff) as i32 - 1023;
    let mut mantissa = (value / 2.0f64.powi(exponent - fraction as i32)).round_ties_even() as u32;
    if mantissa == 1 << (fraction + 1) {
        exponent += 1;
        mantissa >>= 1;
    }
    if exponent + bias >= i32::from(infinity >> fraction) {
        return sign | infinity;
    }
    sign | (((exponent + bias) as u16) << fraction) | (mantissa - (1 << fraction)) as u16
}

#[test]
fn arithmetic_agrees_with_independent_wide_precision_quantization() {
    for (f, fraction, bias, infinity) in
        [(F::Binary16, 10, 15, 0x7c00), (F::BFloat16, 7, 127, 0x7f80)]
    {
        let mut random = 0x6a09e667u32;
        for _ in 0..8192 {
            random = random.wrapping_mul(1664525).wrapping_add(1013904223);
            let a = (random >> 16) as u16;
            random = random.wrapping_mul(1664525).wrapping_add(1013904223);
            let b = (random >> 16) as u16;
            let x = decode(a, fraction, bias, infinity);
            let y = decode(b, fraction, bias, infinity);
            for (op, result) in [
                (Op::Add, x + y),
                (Op::Subtract, x - y),
                (Op::Multiply, x * y),
                (Op::Divide, x / y),
            ] {
                assert_eq!(
                    eval(f, op, a, b),
                    quantize(result, fraction, bias, infinity),
                    "{f} {op} {a:04x} {b:04x}"
                );
            }
        }
    }
}

#[test]
fn arithmetic_budget_and_storage_failures_leave_later_operations_usable() {
    for (f, one) in [(F::Binary16, 0x3c00), (F::BFloat16, 0x3f80)] {
        assert_eq!(
            f.arithmetic(
                Op::Multiply,
                one,
                one,
                &mut Budget {
                    steps: 0,
                    ..Budget::default()
                }
            ),
            Err(Error::BudgetExhausted)
        );
        assert!(
            f.arithmetic(
                Op::Add,
                one,
                1,
                &mut Budget {
                    max_limbs: 0,
                    ..Budget::default()
                }
            )
            .is_err()
        );
        assert_eq!(eval(f, Op::Divide, one, one), one);
    }
}
