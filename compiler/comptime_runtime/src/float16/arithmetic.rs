//! Bounded exact rational arithmetic, rounded once into the selected format.
use super::Float16Format;
use crate::{Budget, Error, Integer};
use std::{cmp::Ordering, fmt};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Float16Op {
    Add,
    Subtract,
    Multiply,
    Divide,
}
impl fmt::Display for Float16Op {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Add => "(float16-op add)",
            Self::Subtract => "(float16-op subtract)",
            Self::Multiply => "(float16-op multiply)",
            Self::Divide => "(float16-op divide)",
        })
    }
}

impl Float16Format {
    /// Execute IEEE nearest-even arithmetic without host floating intermediates.
    /// Unlike literal formation, arithmetic overflow produces signed infinity.
    /// Arithmetic NaNs are canonical; unary negation only flips the sign bit.
    pub fn arithmetic(
        self,
        op: Float16Op,
        left: u16,
        right: u16,
        budget: &mut Budget,
    ) -> Result<u16, Error> {
        budget.consume(1)?;
        let infinity = self.infinity();
        let nan = infinity | (1 << (self.fraction_bits() - 1));
        let a = left & 0x7fff;
        let b = right & 0x7fff;
        if a > infinity || b > infinity {
            return Ok(nan);
        }
        let right = if op == Float16Op::Subtract {
            right ^ 0x8000
        } else {
            right
        };
        let sign = (left ^ right) & 0x8000;
        let (ma, ea) = self.parts(a);
        let (mb, eb) = self.parts(b);
        match op {
            Float16Op::Add | Float16Op::Subtract => {
                if a == infinity || b == infinity {
                    return Ok(if a == infinity && b == infinity && sign != 0 {
                        nan
                    } else if a == infinity {
                        left
                    } else {
                        right
                    });
                }
                let exponent = ea.min(eb);
                let mut x = scale(
                    &Integer::from_u128(ma.into()),
                    (ea - exponent) as u32,
                    budget,
                )?;
                let mut y = scale(
                    &Integer::from_u128(mb.into()),
                    (eb - exponent) as u32,
                    budget,
                )?;
                if left & 0x8000 != 0 {
                    x = x.negate(budget)?;
                }
                if right & 0x8000 != 0 {
                    y = y.negate(budget)?;
                }
                let sum = x.add(&y, budget)?;
                if sum.is_zero() {
                    return Ok(left & right & 0x8000);
                }
                let negative = sum.is_negative();
                let magnitude = if negative { sum.negate(budget)? } else { sum };
                self.round_ratio(
                    &magnitude,
                    1,
                    exponent,
                    if negative { 0x8000 } else { 0 },
                    budget,
                )
            }
            Float16Op::Multiply => {
                if (a == infinity && b == 0) || (b == infinity && a == 0) {
                    return Ok(nan);
                }
                if a == infinity || b == infinity {
                    return Ok(sign | infinity);
                }
                self.round_ratio(
                    &Integer::from_u128(u128::from(ma) * u128::from(mb)),
                    1,
                    ea + eb,
                    sign,
                    budget,
                )
            }
            Float16Op::Divide => {
                if (a == 0 && b == 0) || (a == infinity && b == infinity) {
                    return Ok(nan);
                }
                if a == infinity || b == 0 {
                    return Ok(sign | infinity);
                }
                if b == infinity {
                    return Ok(sign);
                }
                self.round_ratio(&Integer::from_u128(ma.into()), mb, ea - eb, sign, budget)
            }
        }
    }

    pub(super) fn round_ratio(
        self,
        numerator: &Integer,
        denominator: u32,
        exponent: i32,
        sign: u16,
        budget: &mut Budget,
    ) -> Result<u16, Error> {
        budget.consume(1)?;
        if numerator.is_zero() {
            return Ok(sign);
        }
        let mut lower = 0;
        let mut upper = self.infinity();
        while lower < upper {
            budget.consume(1)?;
            let middle = lower + (upper - lower) / 2;
            let (mantissa, power) = self.parts(middle);
            // Compare n/d * 2^exponent against the exact upper midpoint.
            let common = exponent.min(power - 1);
            let left = scale(numerator, (exponent - common) as u32, budget)?;
            let right = scale(
                &Integer::from_u128(u128::from(denominator) * u128::from(2 * mantissa + 1)),
                (power - 1 - common) as u32,
                budget,
            )?;
            budget.consume(left.bit_len().max(right.bit_len()).div_ceil(32))?;
            match left.cmp(&right) {
                Ordering::Less => upper = middle,
                Ordering::Equal if middle & 1 == 0 => upper = middle,
                _ => lower = middle + 1,
            }
        }
        Ok(sign | lower)
    }
}

pub(super) fn scale(
    value: &Integer,
    mut shift: u32,
    budget: &mut Budget,
) -> Result<Integer, Error> {
    let chunk = shift.min(127);
    let mut value = value.multiply(&Integer::from_u128(1u128 << chunk), budget)?;
    shift -= chunk;
    while shift != 0 {
        let chunk = shift.min(127);
        value = value.multiply(&Integer::from_u128(1u128 << chunk), budget)?;
        shift -= chunk;
    }
    Ok(value)
}
