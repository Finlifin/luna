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
        self.scalar_math_values(binding, &values, expr.source)
            .map(Flow::Value)
    }

    pub(super) fn scalar_math_values(
        &mut self,
        binding: &nir::intrinsics::Checked,
        values: &[Value],
        source: SourceRange,
    ) -> Result<Value, Error> {
        for value in values {
            Self::concrete_value(value, source)?;
        }
        if binding.operation() == Op::FusedMultiplyAdd {
            use comptime_runtime::float16::Float16Format as F;
            let half = match values {
                [Value::Float16(a), Value::Float16(b), Value::Float16(c)] => {
                    Some((F::Binary16, *a, *b, *c))
                }
                [Value::BFloat16(a), Value::BFloat16(b), Value::BFloat16(c)] => {
                    Some((F::BFloat16, *a, *b, *c))
                }
                _ => None,
            };
            if let Some((format, a, b, c)) = half {
                let bits = self.integer_operation(Some(source), |budget| {
                    format.fused_multiply_add(a, b, c, budget)
                })?;
                return Ok(if format == F::Binary16 {
                    Value::Float16(bits)
                } else {
                    Value::BFloat16(bits)
                });
            }
        }
        let result = scalar(
            binding.operation(),
            values,
            binding.width_for_target(self.target),
        );
        result.ok_or_else(|| self.failure(ErrorKind::Type, Some(source)))
    }
}

fn scale_ieee_pow2(bits: u64, power: i32, fraction: u32, exp_bits: u32) -> u64 {
    let total_bits = 1 + exp_bits + fraction;
    let sign_bit = 1u64 << (total_bits - 1);
    let exp_mask = ((1u64 << exp_bits) - 1) << fraction;
    let mant_mask = (1u64 << fraction) - 1;
    let sign = bits & sign_bit;
    let magnitude = bits & (sign_bit - 1);
    if magnitude >= exp_mask {
        return if magnitude == exp_mask {
            bits
        } else {
            bits | (1u64 << (fraction - 1))
        };
    }
    if magnitude == 0 {
        return bits;
    }
    let bias = ((1u64 << (exp_bits - 1)) - 1) as i64;
    let raw_exp = magnitude >> fraction;
    let (mantissa, exponent) = if raw_exp == 0 {
        (magnitude, 1 - bias - i64::from(fraction))
    } else {
        (
            (1u64 << fraction) | (magnitude & mant_mask),
            raw_exp as i64 - bias - i64::from(fraction),
        )
    };
    let exponent = exponent + i64::from(power);
    let leading = 63 - mantissa.leading_zeros();
    let normalized = exponent + i64::from(leading);
    if normalized > bias {
        return sign | exp_mask;
    }
    if normalized >= 1 - bias {
        let significand = mantissa << (fraction - leading);
        return sign | (((normalized + bias) as u64) << fraction) | (significand & mant_mask);
    }
    let shift = 1 - bias - i64::from(fraction) - exponent;
    if shift <= 0 {
        return sign | (mantissa << (-shift));
    }
    if shift > i64::from(leading) + 1 {
        return sign;
    }
    let quotient = mantissa >> shift;
    let remainder = mantissa & ((1u64 << shift) - 1);
    let halfway = 1u64 << (shift - 1);
    let increment = remainder > halfway || (remainder == halfway && quotient & 1 != 0);
    sign | (quotient + u64::from(increment))
}

fn scale_f32(bits: u32, power: i32) -> u32 {
    scale_ieee_pow2(u64::from(bits), power, 23, 8) as u32
}

fn scale_f64(bits: u64, power: i32) -> u64 {
    scale_ieee_pow2(bits, power, 52, 11)
}

macro_rules! float_math {
    ($kind:ident, $float:ty, $values:ident, $op:ident, $scale:ident) => {{
        let read = |index| match $values.get(index)? {
            Value::$kind(bits) => Some(<$float>::from_bits(*bits)),
            _ => None,
        };
        let a_bits = match $values.first()? {
            Value::$kind(bits) => *bits,
            _ => return None,
        };
        let a = <$float>::from_bits(a_bits);
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
        if $op == Op::ScaleByPowerOfTwo {
            let Some(Value::SignedInteger(n)) = $values.get(1) else {
                return None;
            };
            return Some(Value::$kind($scale(a_bits, i32::try_from(*n).ok()?)));
        }
        let value: $float = match $op {
            Op::FusedMultiplyAdd => a.mul_add(read(1)?, read(2)?),
            Op::SquareRoot => a.sqrt(),
            Op::CopySign => a.copysign(read(1)?),
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
            Op::Power => a.powf(read(1)?),
            Op::IntegerPower => {
                let Some(Value::SignedInteger(n)) = $values.get(1) else {
                    return None;
                };
                a.powi(i32::try_from(*n).ok()?)
            }
            Op::Minimum | Op::Maximum | Op::MinimumNumber | Op::MaximumNumber => {
                let b = read(1)?;
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
    if values.len() != op.operand_count() {
        return None;
    }
    match values.first()? {
        Value::Float16(_) => half_scalar(op, values, false),
        Value::BFloat16(_) => half_scalar(op, values, true),
        Value::Float32(_) => float_math!(Float32, f32, values, op, scale_f32),
        Value::Float64(_) => float_math!(Float64, f64, values, op, scale_f64),
        Value::SignedInteger(_) | Value::UnsignedInteger(_) => integer(op, values, width?),
        _ => None,
    }
}

fn integer(op: Op, values: &[Value], width: u32) -> Option<Value> {
    if !(1..=128).contains(&width) {
        return None;
    }
    let mask = u128::MAX >> (128 - width);
    let lo = i128::MIN >> (128 - width);
    let hi = i128::MAX >> (128 - width);
    let signed = matches!(values.first()?, Value::SignedInteger(_));
    let read = |index| match values.get(index)? {
        Value::SignedInteger(value) if signed && *value >= lo && *value <= hi => {
            Some(*value as u128 & mask)
        }
        Value::UnsignedInteger(value) if !signed && *value <= mask => Some(*value),
        _ => None,
    };
    let a = read(0)?;
    let b = if values.len() > 1 { read(1)? } else { 0 };
    let extend = |value: u128| ((value << (128 - width)) as i128) >> (128 - width);
    let (sa, sb) = (extend(a), extend(b));
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
    } else if op == Op::ScaleByPowerOfTwo {
        let Value::SignedInteger(power) = values.get(1)? else {
            return None;
        };
        format.scale_pow2(a, i32::try_from(*power).ok()?)
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

#[cfg(test)]
mod tests {
    use super::*;
    use nir::intrinsics::{Operation as Op, Registry, Tag};
    use nir::program::{Enum, Variant, VariantShape};

    #[test]
    fn scalar_math_rejects_wrong_arity_types_and_signed_out_of_range() {
        let src = SourceRange { start: 1, end: 2 };
        let fam_id = DeclarationId {
            package: 9,
            local: 1,
        };
        let intrinsic_enum = Enum {
            is_intrinsic: true,
            parameter_types: vec![],
            capabilities: vec![],
            requirements: vec![],
            type_parameters: vec![],
            declaration: fam_id,
            name: "Math".into(),
            variants: vec![
                Variant {
                    name: "fma".into(),
                    shape: VariantShape::Unit,
                    fields: vec![],
                },
                Variant {
                    name: "copysign".into(),
                    shape: VariantShape::Unit,
                    fields: vec![],
                },
                Variant {
                    name: "iabs".into(),
                    shape: VariantShape::Unit,
                    fields: vec![],
                },
            ],
            groups: vec![],
            encoding: None,
            symbol_encoding: None,
        };
        let mut pkg = Package::default();
        pkg.enums.push(intrinsic_enum);
        let reg = Registry::new(
            &pkg,
            &[
                (
                    Tag {
                        family: fam_id,
                        variant: 0,
                    },
                    Op::FusedMultiplyAdd,
                ),
                (
                    Tag {
                        family: fam_id,
                        variant: 1,
                    },
                    Op::CopySign,
                ),
                (
                    Tag {
                        family: fam_id,
                        variant: 2,
                    },
                    Op::WrappingAbsoluteValue,
                ),
            ],
        )
        .unwrap();
        let fma = reg
            .check(
                Tag {
                    family: fam_id,
                    variant: 0,
                },
                &[
                    Term::Primitive(Primitive::F32),
                    Term::Primitive(Primitive::F32),
                    Term::Primitive(Primitive::F32),
                ],
                &Term::Primitive(Primitive::F32),
            )
            .unwrap()
            .for_comptime();
        let copysign = reg
            .check(
                Tag {
                    family: fam_id,
                    variant: 1,
                },
                &[
                    Term::Primitive(Primitive::F32),
                    Term::Primitive(Primitive::F32),
                ],
                &Term::Primitive(Primitive::F32),
            )
            .unwrap()
            .for_comptime();
        let iabs = reg
            .check(
                Tag {
                    family: fam_id,
                    variant: 2,
                },
                &[Term::Primitive(Primitive::I8)],
                &Term::Primitive(Primitive::I8),
            )
            .unwrap()
            .for_comptime();

        let mut vm = Interpreter::for_comptime(&pkg, Budget::default()).unwrap();
        let one = Value::Float32(1.0f32.to_bits());
        assert_eq!(
            vm.scalar_math_values(&fma, &[one.clone(), one.clone()], src)
                .unwrap_err()
                .kind,
            ErrorKind::Type
        );
        assert_eq!(
            vm.scalar_math_values(&fma, &[one.clone(), one.clone(), Value::Bool(false)], src)
                .unwrap_err()
                .kind,
            ErrorKind::Type
        );
        assert_eq!(
            vm.scalar_math_values(&copysign, &[one.clone(), Value::Float64(0)], src)
                .unwrap_err()
                .kind,
            ErrorKind::Type
        );
        assert_eq!(
            vm.scalar_math_values(
                &copysign,
                &[
                    one,
                    Value::AbstractValue {
                        declaration: DeclarationId {
                            package: 9,
                            local: 99,
                        },
                        ty: Term::Primitive(Primitive::F32),
                    }
                ],
                src,
            )
            .unwrap_err()
            .kind,
            ErrorKind::AbstractValueUnresolved
        );
        assert_eq!(
            vm.scalar_math_values(&iabs, &[Value::SignedInteger(200)], src)
                .unwrap_err()
                .kind,
            ErrorKind::Type
        );
    }
}
