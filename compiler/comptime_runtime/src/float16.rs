//! Exact decimal rounding for the two distinct 16-bit floating-point formats.
//! This is representation machinery, not permission to execute a source operator.
use crate::{Budget, Error, Integer, decimal::DecimalValue};
use std::{cmp::Ordering, fmt};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Float16Format {
    Binary16,
    BFloat16,
}

impl Float16Format {
    fn fraction_bits(self) -> u32 {
        match self {
            Self::Binary16 => 10,
            Self::BFloat16 => 7,
        }
    }

    fn bias(self) -> i32 {
        match self {
            Self::Binary16 => 15,
            Self::BFloat16 => 127,
        }
    }

    fn infinity(self) -> u16 {
        match self {
            Self::Binary16 => 0x7c00,
            Self::BFloat16 => 0x7f80,
        }
    }

    /// IEEE ordered comparison. NaNs (quiet or signaling) are unordered;
    /// signed zeros compare equal. This never rounds or canonicalizes payloads.
    pub fn compare(self, left: u16, right: u16) -> Option<Ordering> {
        let a = left & 0x7fff;
        let b = right & 0x7fff;
        if a > self.infinity() || b > self.infinity() {
            return None;
        }
        if a == 0 && b == 0 {
            return Some(Ordering::Equal);
        }
        let key = |bits: u16| {
            if bits & 0x8000 != 0 {
                !bits
            } else {
                bits ^ 0x8000
            }
        };
        Some(key(left).cmp(&key(right)))
    }

    // Finite positive encodings are ordered by their unsigned representation.
    fn parts(self, bits: u16) -> (u32, i32) {
        let fraction_bits = self.fraction_bits();
        let exponent = u32::from(bits) >> fraction_bits;
        let fraction = u32::from(bits) & ((1 << fraction_bits) - 1);
        if exponent == 0 {
            (fraction, 1 - self.bias() - fraction_bits as i32)
        } else {
            (
                (1 << fraction_bits) | fraction,
                exponent as i32 - self.bias() - fraction_bits as i32,
            )
        }
    }

    /// Round once, nearest with ties to even, directly from the exact decimal.
    /// Finite overflow is an error; underflow preserves the input zero sign.
    /// Binary search uses at most 15 exact midpoint comparisons. Large decimal
    /// exponents are compared symbolically, never expanded into powers of ten.
    pub fn round(self, value: &DecimalValue, budget: &mut Budget) -> Result<u16, Error> {
        budget.consume(1)?;
        let sign = if value.is_negative() { 0x8000 } else { 0 };
        let positive;
        let value = if value.is_negative() {
            positive = value.negate(budget)?;
            &positive
        } else {
            value
        };
        if value.is_zero() {
            // Validate representation limits even on the zero fast path.
            value.to_literal(budget)?;
            return Ok(sign);
        }
        let mut lower = 0;
        let mut upper = self.infinity();
        while lower < upper {
            budget.consume(1)?;
            let middle = lower + (upper - lower) / 2;
            let (significand, exponent) = self.parts(middle);
            let midpoint = dyadic(2 * significand + 1, exponent - 1, budget)?;
            match value.compare(&midpoint, budget)? {
                Ordering::Less => upper = middle,
                Ordering::Equal if middle & 1 == 0 => upper = middle,
                _ => lower = middle + 1,
            }
        }
        if lower == self.infinity() {
            Err(Error::OutOfRange)
        } else {
            Ok(sign | lower)
        }
    }

    /// Recover the exact finite value, including the sign of zero. Nonfinite
    /// encodings require explicit infinity/NaN handling by the caller.
    pub fn to_exact(self, bits: u16, budget: &mut Budget) -> Result<DecimalValue, Error> {
        budget.consume(1)?;
        let magnitude = bits & 0x7fff;
        if magnitude >= self.infinity() {
            return Err(Error::OutOfRange);
        }
        let (significand, exponent) = self.parts(magnitude);
        let value = dyadic(significand, exponent, budget)?;
        if bits & 0x8000 != 0 {
            value.negate(budget)
        } else {
            Ok(value)
        }
    }
}

// m * 2^-n = (m * 5^n) * 10^-n; no intermediate machine float rounding.
fn dyadic(significand: u32, exponent: i32, budget: &mut Budget) -> Result<DecimalValue, Error> {
    let mut coefficient = Integer::from_u128(significand.into());
    let factor = Integer::from_u128(if exponent < 0 { 5 } else { 2 });
    for _ in 0..exponent.unsigned_abs() {
        coefficient = coefficient.multiply(&factor, budget)?;
    }
    let digits = coefficient.to_decimal_literal(budget)?;
    budget.consume(digits.len().saturating_add(16))?;
    DecimalValue::parse_literal(&format!("{digits}.0e{}", exponent.min(0)), budget)
}

impl fmt::Display for Float16Format {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Binary16 => "(float16-format f16)",
            Self::BFloat16 => "(float16-format bf16)",
        })
    }
}
