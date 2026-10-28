use comptime_runtime::float16::{Float16Format as F, IntegralRounding as R};

fn decode(bits: u16, fraction: u32, bias: i32) -> f64 {
    let magnitude = bits & 0x7fff;
    let exponent = i32::from(magnitude >> fraction);
    let significand = u32::from(magnitude) & ((1 << fraction) - 1);
    let value = if exponent == 0 {
        f64::from(significand) * 2.0f64.powi(1 - bias - fraction as i32)
    } else {
        f64::from(significand | (1 << fraction)) * 2.0f64.powi(exponent - bias - fraction as i32)
    };
    if bits & 0x8000 != 0 { -value } else { value }
}

#[test]
fn integral_rounding_covers_every_encoding_and_all_five_directions() {
    for (format, fraction, bias, infinity) in
        [(F::Binary16, 10, 15, 0x7c00), (F::BFloat16, 7, 127, 0x7f80)]
    {
        for bits in 0..=u16::MAX {
            for mode in [
                R::Floor,
                R::Ceiling,
                R::Truncate,
                R::NearestEven,
                R::NearestAway,
            ] {
                let rounded = format.round_integral(bits, mode);
                let magnitude = bits & 0x7fff;
                if magnitude >= infinity {
                    let expected = if magnitude == infinity {
                        bits
                    } else {
                        bits | (1 << (fraction - 1))
                    };
                    assert_eq!(rounded, expected, "{format} {mode} {bits:04x}");
                    continue;
                }
                let value = decode(bits, fraction, bias);
                let expected = match mode {
                    R::Floor => value.floor(),
                    R::Ceiling => value.ceil(),
                    R::Truncate => value.trunc(),
                    R::NearestEven => value.round_ties_even(),
                    R::NearestAway => value.round(),
                };
                assert!(rounded & 0x7fff < infinity);
                assert_eq!(
                    decode(rounded, fraction, bias),
                    expected,
                    "{format} {mode} {bits:04x}"
                );
                assert_eq!(
                    rounded & 0x8000 != 0,
                    expected.is_sign_negative(),
                    "{format} {mode} {bits:04x}"
                );
            }
        }
    }
}
