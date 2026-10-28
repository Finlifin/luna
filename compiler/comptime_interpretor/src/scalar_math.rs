//! Checked scalar operations for the comptime pipeline. Runtime tags are rejected.
use super::*;
use nir::intrinsics::Operation as Op;

impl Interpreter<'_> {
    pub(super) fn eval_scalar_math(
        &mut self,
        expr: &Expr,
        environment: &mut Environment,
    ) -> Result<Flow, Error> {
        let K::Intrinsic {
            binding: Some(binding),
            ..
        } = &expr.kind
        else {
            return Err(self.failure(ErrorKind::Unsupported, Some(expr.source)));
        };
        if !binding.is_comptime() {
            return Err(self.failure(ErrorKind::Unsupported, Some(expr.source)));
        }
        let binding = nir::intrinsics::checked(expr)
            .map_err(|_| self.failure(ErrorKind::Type, Some(expr.source)))?;
        let K::Intrinsic { inputs, .. } = &expr.kind else {
            unreachable!()
        };
        let mut values = Vec::with_capacity(inputs.len());
        for input in inputs {
            match self.eval(input, environment)? {
                Flow::Value(value) => values.push(value),
                flow => return Ok(flow),
            }
        }
        let result = scalar(
            binding.operation(),
            &values,
            binding.width_for_target(self.target),
        );
        result
            .map(Flow::Value)
            .ok_or_else(|| self.failure(ErrorKind::Type, Some(expr.source)))
    }
}

// Scale in bounded exact power-of-two steps; never first form an overflowing
// or underflowing 2^exponent. The last multiplication alone may be subnormal.
macro_rules! float_math {
    ($kind:ident, $float:ty, $values:ident, $op:ident, $large:expr, $small:expr, $step:expr) => {{
        let Value::$kind(bits) = $values.first()? else {
            return None;
        };
        let a = <$float>::from_bits(*bits);
        let b = match $values.get(1) {
            Some(Value::$kind(bits)) => <$float>::from_bits(*bits),
            _ => 0.0,
        };
        let c = match $values.get(2) {
            Some(Value::$kind(bits)) => <$float>::from_bits(*bits),
            _ => 0.0,
        };
        let predicate = match $op {
            Op::IsNan => Some(a.is_nan()),
            Op::IsInfinite => Some(a.is_infinite()),
            Op::IsFinite => Some(a.is_finite()),
            Op::IsNormal => Some(a.is_normal()),
            Op::IsSubnormal => Some(a.is_subnormal()),
            Op::SignBit => Some(a.is_sign_negative()),
            _ => None,
        };
        if let Some(value) = predicate {
            return Some(Value::Bool(value));
        }
        let value: $float = match $op {
            Op::FusedMultiplyAdd => a.mul_add(b, c),
            Op::SquareRoot => a.sqrt(),
            Op::CopySign => a.copysign(b),
            Op::Floor => a.floor(),
            Op::Ceiling => a.ceil(),
            Op::Truncate => a.trunc(),
            Op::RoundEven => a.round_ties_even(),
            Op::RoundAway => a.round(),
            Op::AbsoluteValue => a.abs(),
            Op::Sine => a.sin(),
            Op::Cosine => a.cos(),
            Op::Exponential => a.exp(),
            Op::Exponential2 => a.exp2(),
            Op::NaturalLog => a.ln(),
            Op::Log2 => a.log2(),
            Op::Log10 => a.log10(),
            Op::Power => a.powf(b),
            Op::IntegerPower => {
                let Some(Value::SignedInteger(n)) = $values.get(1) else {
                    return None;
                };
                a.powi(i32::try_from(*n).ok()?)
            }
            Op::ScaleByPowerOfTwo => {
                let Some(Value::SignedInteger(n)) = $values.get(1) else {
                    return None;
                };
                let mut n = i32::try_from(*n).ok()?;
                let mut value = a;
                // Two steps suffice to exhaust any finite input's exponent.
                for _ in 0..2 {
                    if n > $step {
                        value *= $large;
                        n -= $step;
                    }
                    if n < -$step {
                        value *= $small;
                        n += $step;
                    }
                }
                value * (2.0 as $float).powi(n.clamp(-$step, $step))
            }
            Op::Minimum | Op::Maximum | Op::MinimumNumber | Op::MaximumNumber => {
                let minimum = matches!($op, Op::Minimum | Op::MinimumNumber);
                let propagate = matches!($op, Op::Minimum | Op::Maximum);
                if a.is_nan() {
                    if propagate || b.is_nan() { a + 0.0 } else { b }
                } else if b.is_nan() {
                    if propagate { b + 0.0 } else { a }
                } else if a == b {
                    if minimum {
                        if a.is_sign_negative() { a } else { b }
                    } else if a.is_sign_positive() {
                        a
                    } else {
                        b
                    }
                } else if (a < b) == minimum {
                    a
                } else {
                    b
                }
            }
            _ => return None,
        };
        Some(Value::$kind(value.to_bits()))
    }};
}

fn scalar(op: Op, values: &[Value], width: Option<u32>) -> Option<Value> {
    match values.first()? {
        Value::Float16(_) => half_scalar(op, values, false),
        Value::BFloat16(_) => half_scalar(op, values, true),
        Value::Float32(_) => float_math!(
            Float32,
            f32,
            values,
            op,
            2.0f32.powi(102),
            2.0f32.powi(-102),
            102
        ),
        Value::Float64(_) => float_math!(
            Float64,
            f64,
            values,
            op,
            2.0f64.powi(969),
            2.0f64.powi(-969),
            969
        ),
        Value::SignedInteger(_) | Value::UnsignedInteger(_) => integer(op, values, width?),
        _ => None,
    }
}

fn integer(op: Op, values: &[Value], width: u32) -> Option<Value> {
    if !(1..=128).contains(&width) {
        return None;
    }
    let mask = u128::MAX >> (128 - width);
    let signed = matches!(values.first()?, Value::SignedInteger(_));
    let read = |index| match values.get(index)? {
        Value::SignedInteger(value) if signed => Some(*value as u128 & mask),
        Value::UnsignedInteger(value) if !signed && *value <= mask => Some(*value),
        _ => None,
    };
    let a = read(0)?;
    let b = if values.len() > 1 { read(1)? } else { 0 };
    let extend = |value: u128| ((value << (128 - width)) as i128) >> (128 - width);
    let (sa, sb) = (extend(a), extend(b));
    let lo = i128::MIN >> (128 - width);
    let hi = i128::MAX >> (128 - width);
    let bits = match op {
        Op::BitAnd => a & b,
        Op::BitOr => a | b,
        Op::BitXor => a ^ b,
        Op::BitReverse => a.reverse_bits() >> (128 - width),
        Op::ByteSwap if width % 16 == 0 => a.swap_bytes() >> (128 - width),
        Op::PopulationCount => a.count_ones() as u128,
        Op::LeadingZeros => (a.leading_zeros() - (128 - width)) as u128,
        Op::TrailingZeros => a.trailing_zeros().min(width) as u128,
        Op::FunnelShiftLeft | Op::FunnelShiftRight => {
            let amount = (read(2)? % width as u128) as u32;
            if amount == 0 {
                if op == Op::FunnelShiftLeft { a } else { b }
            } else if op == Op::FunnelShiftLeft {
                (a << amount) | (b >> (width - amount))
            } else {
                (b >> amount) | (a << (width - amount))
            }
        }
        Op::IntegerMinimum => {
            if signed {
                sa.min(sb) as u128
            } else {
                a.min(b)
            }
        }
        Op::IntegerMaximum => {
            if signed {
                sa.max(sb) as u128
            } else {
                a.max(b)
            }
        }
        Op::WrappingAbsoluteValue if signed => sa.wrapping_abs() as u128,
        Op::SaturatingAdd => {
            if signed {
                sa.saturating_add(sb).clamp(lo, hi) as u128
            } else {
                a.saturating_add(b).min(mask)
            }
        }
        Op::SaturatingSubtract => {
            if signed {
                sa.saturating_sub(sb).clamp(lo, hi) as u128
            } else {
                a.saturating_sub(b)
            }
        }
        _ => return None,
    } & mask;
    Some(if signed {
        Value::SignedInteger(extend(bits))
    } else {
        Value::UnsignedInteger(bits)
    })
}

// Classify the representation directly: widening changes subnormal classes.
fn half_predicate(op: Op, bits: u16, exponent_mask: u16) -> Option<Value> {
    let magnitude = bits & 0x7fff;
    let exponent = bits & exponent_mask;
    Some(Value::Bool(match op {
        Op::IsNan => magnitude > exponent_mask,
        Op::IsInfinite => magnitude == exponent_mask,
        Op::IsFinite => exponent != exponent_mask,
        Op::IsNormal => exponent != 0 && exponent != exponent_mask,
        Op::IsSubnormal => exponent == 0 && magnitude != 0,
        Op::SignBit => bits & 0x8000 != 0,
        _ => return None,
    }))
}

fn half_scalar(op: Op, values: &[Value], bfloat: bool) -> Option<Value> {
    use comptime_runtime::float16::Float16Format;
    let bits = |value: &Value| match (bfloat, value) {
        (false, Value::Float16(bits)) | (true, Value::BFloat16(bits)) => Some(*bits),
        _ => None,
    };
    let a = bits(values.first()?)?;
    let (format, infinity, quiet) = if bfloat {
        (Float16Format::BFloat16, 0x7f80, 0x0040)
    } else {
        (Float16Format::Binary16, 0x7c00, 0x0200)
    };
    if op.is_float_predicate() {
        return half_predicate(op, a, infinity);
    }
    use comptime_runtime::float16::IntegralRounding as R;
    let rounding = match op {
        Op::Floor => Some(R::Floor),
        Op::Ceiling => Some(R::Ceiling),
        Op::Truncate => Some(R::Truncate),
        Op::RoundEven => Some(R::NearestEven),
        Op::RoundAway => Some(R::NearestAway),
        _ => None,
    };
    let result = if let Some(mode) = rounding {
        format.round_integral(a, mode)
    } else if op == Op::SquareRoot {
        format.square_root(a)
    } else if op == Op::AbsoluteValue {
        a & 0x7fff
    } else {
        let b = bits(values.get(1)?)?;
        match op {
            Op::CopySign => (a & 0x7fff) | (b & 0x8000),
            Op::Minimum | Op::Maximum | Op::MinimumNumber | Op::MaximumNumber => {
                let minimum = matches!(op, Op::Minimum | Op::MinimumNumber);
                let propagate = matches!(op, Op::Minimum | Op::Maximum);
                let a_nan = a & 0x7fff > infinity;
                let b_nan = b & 0x7fff > infinity;
                if a_nan {
                    if propagate || b_nan { a | quiet } else { b }
                } else if b_nan {
                    if propagate { b | quiet } else { a }
                } else {
                    match format.compare(a, b)? {
                        std::cmp::Ordering::Equal => {
                            if minimum {
                                a | b
                            } else {
                                a & b
                            }
                        }
                        ordering => {
                            if (ordering == std::cmp::Ordering::Less) == minimum {
                                a
                            } else {
                                b
                            }
                        }
                    }
                }
            }
            _ => return None,
        }
    };
    Some(if bfloat {
        Value::BFloat16(result)
    } else {
        Value::Float16(result)
    })
}
