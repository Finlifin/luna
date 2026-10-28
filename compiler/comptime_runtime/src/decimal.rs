//! Exact lexical decimal components, before choosing a binary runtime proxy.
//! This is not a general implementation of the language's comptime Float.
use crate::{Budget, Error, Integer};
use std::fmt;

#[derive(Clone, Debug)]
pub struct DecimalLiteral {
    coefficient: Integer,
    exponent: Integer,
    negative: bool,
    // Validated spelling is retained for direct conversion to the selected
    // IEEE format. Never route an f32 conversion through a rounded f64.
    spelling: String,
}

fn digits(text: &str) -> bool {
    !text.is_empty()
        && !text.starts_with('_')
        && !text.ends_with('_')
        && text
            .bytes()
            .all(|byte| byte.is_ascii_digit() || byte == b'_')
}

impl DecimalLiteral {
    pub fn parse(text: &str, budget: &mut Budget) -> Result<Self, Error> {
        budget.charge(text.len().saturating_add(1))?;
        let (mantissa, exponent) = text.split_once(['e', 'E']).unwrap_or((text, "0"));
        let (whole, fraction) = mantissa.split_once('.').ok_or(Error::InvalidLiteral)?;
        if !digits(whole) || !digits(fraction) {
            return Err(Error::InvalidLiteral);
        }
        let (negative_exponent, exponent) = if let Some(exponent) = exponent.strip_prefix('-') {
            (true, exponent)
        } else {
            (false, exponent.strip_prefix('+').unwrap_or(exponent))
        };
        if !digits(exponent) {
            return Err(Error::InvalidLiteral);
        }
        let spelling: String = text.chars().filter(|c| *c != '_').collect();
        let coefficient: String = whole
            .bytes()
            .chain(fraction.bytes())
            .filter(|b| *b != b'_')
            .map(char::from)
            .collect();
        let mut decimal_exponent = Integer::parse_literal(exponent, budget)?;
        if negative_exponent {
            decimal_exponent = decimal_exponent.negated();
        }
        decimal_exponent = decimal_exponent.subtract(
            &Integer::from_u128(fraction.bytes().filter(|b| *b != b'_').count() as u128),
            budget,
        )?;
        let coefficient = Integer::parse_literal(&coefficient, budget)?;
        Ok(Self {
            coefficient,
            exponent: decimal_exponent,
            negative: false,
            spelling,
        })
    }
    pub fn coefficient(&self) -> &Integer {
        &self.coefficient
    }
    pub fn exponent(&self) -> &Integer {
        &self.exponent
    }
    pub fn negated(mut self) -> Self {
        self.negative = !self.negative;
        self
    }

    /// Canonical finite decimal identity without binary rounding. Signed zero
    /// remains distinct for later IEEE conversion; `compare` is numeric.
    pub fn exact_value(&self, budget: &mut Budget) -> Result<DecimalValue, Error> {
        budget.charge(self.spelling.len().saturating_add(1))?;
        let mantissa = self
            .spelling
            .split(['e', 'E'])
            .next()
            .ok_or(Error::InvalidLiteral)?;
        let digits = mantissa
            .bytes()
            .filter(|b| *b != b'.')
            .map(char::from)
            .collect::<String>();
        let nonzero = digits.trim_start_matches('0');
        if nonzero.is_empty() {
            return Ok(DecimalValue {
                digits: "0".into(),
                exponent: Integer::zero(),
                negative: self.negative,
            });
        }
        let canonical = nonzero.trim_end_matches('0');
        // Apply the same representation-size check as exact integer parsing.
        Integer::parse_literal(canonical, budget)?;
        let removed = nonzero.len() - canonical.len();
        let exponent = self
            .exponent
            .add(&Integer::from_u128(removed as u128), budget)?;
        Ok(DecimalValue {
            digits: canonical.into(),
            exponent,
            negative: self.negative,
        })
    }

    pub fn to_f16_bits(&self, budget: &mut Budget) -> Result<u16, Error> {
        self.exact_value(budget)?.to_f16_bits(budget)
    }
    pub fn to_bf16_bits(&self, budget: &mut Budget) -> Result<u16, Error> {
        self.exact_value(budget)?.to_bf16_bits(budget)
    }

    /// Share target rounding with computed exact values.
    pub fn to_f32_bits(&self, budget: &mut Budget) -> Result<u32, Error> {
        self.exact_value(budget)?.to_f32_bits(budget)
    }
    pub fn to_f64_bits(&self, budget: &mut Budget) -> Result<u64, Error> {
        self.exact_value(budget)?.to_f64_bits(budget)
    }
}
impl fmt::Display for DecimalLiteral {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(decimal-literal (negative {}) (coefficient {}) (exponent {}))",
            self.negative, self.coefficient, self.exponent
        )
    }
}

/// A finite base-ten value. Private fields enforce a nonzero leading/trailing
/// significand digit (except zero) and a zero exponent for zero. Identity keeps
/// the sign of zero, because two numerically equal values may round differently.
/// This is a literal-value building block, not the full language Float algebra.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct DecimalValue {
    digits: Box<str>,
    exponent: Integer,
    negative: bool,
}
impl DecimalValue {
    /// Read an internal literal spelling. Source lexing still represents minus
    /// as an operator; materialized constants may carry their sign here.
    pub fn parse_literal(text: &str, budget: &mut Budget) -> Result<Self, Error> {
        let (negative, magnitude) = text.strip_prefix('-').map_or((false, text), |s| (true, s));
        let literal = DecimalLiteral::parse(magnitude, budget)?;
        if negative { literal.negated() } else { literal }.exact_value(budget)
    }

    /// Lossless scientific spelling; never expand a power of ten or round
    /// through a host float. Work is bounded by stored payload and fuel.
    pub fn to_literal(&self, budget: &mut Budget) -> Result<String, Error> {
        budget.charge(
            self.digits
                .len()
                .saturating_add(self.exponent.bit_len().div_ceil(32))
                .saturating_add(8),
        )?;
        Integer::parse_literal(&self.digits, budget)?;
        if self.exponent.bit_len().div_ceil(32) > budget.max_limbs {
            return Err(Error::SizeLimit);
        }
        let exponent = self.exponent.to_decimal_literal(budget)?;
        let mut spelling = String::new();
        if self.negative {
            spelling.push('-');
        }
        spelling.push_str(&self.digits);
        spelling.push_str(".0e");
        spelling.push_str(&exponent);
        Ok(spelling)
    }

    pub fn digits(&self) -> &str {
        &self.digits
    }
    pub fn exponent(&self) -> &Integer {
        &self.exponent
    }
    pub fn is_negative(&self) -> bool {
        self.negative
    }
    pub fn is_zero(&self) -> bool {
        self.digits.as_ref() == "0"
    }

    pub fn to_f16_bits(&self, budget: &mut Budget) -> Result<u16, Error> {
        crate::float16::Float16Format::Binary16.round(self, budget)
    }
    pub fn to_bf16_bits(&self, budget: &mut Budget) -> Result<u16, Error> {
        crate::float16::Float16Format::BFloat16.round(self, budget)
    }

    /// IEEE nearest, ties-to-even conversion directly to the target format.
    /// Finite overflow is reported to the caller; underflow keeps zero's sign.
    pub fn to_f32_bits(&self, budget: &mut Budget) -> Result<u32, Error> {
        let spelling = self.to_literal(budget)?;
        budget.charge(spelling.len().saturating_add(1))?;
        let value = spelling.parse::<f32>().map_err(|_| Error::InvalidLiteral)?;
        if !value.is_finite() {
            return Err(Error::OutOfRange);
        }
        Ok(value.to_bits())
    }

    pub fn to_f64_bits(&self, budget: &mut Budget) -> Result<u64, Error> {
        let spelling = self.to_literal(budget)?;
        budget.charge(spelling.len().saturating_add(1))?;
        let value = spelling.parse::<f64>().map_err(|_| Error::InvalidLiteral)?;
        if !value.is_finite() {
            return Err(Error::OutOfRange);
        }
        Ok(value.to_bits())
    }

    fn coefficient(&self, budget: &mut Budget) -> Result<Integer, Error> {
        budget.charge(self.exponent.bit_len().div_ceil(32).saturating_add(1))?;
        if self.exponent.bit_len().div_ceil(32) > budget.max_limbs {
            return Err(Error::SizeLimit);
        }
        let value = Integer::parse_literal(&self.digits, budget)?;
        if self.negative {
            value.negate(budget)
        } else {
            Ok(value)
        }
    }

    fn from_coefficient(
        value: Integer,
        exponent: Integer,
        negative_zero: bool,
        budget: &mut Budget,
    ) -> Result<Self, Error> {
        if value.is_zero() {
            budget.charge(1)?;
            return Ok(Self {
                digits: "0".into(),
                exponent: Integer::zero(),
                negative: negative_zero,
            });
        }
        let spelling = value.to_decimal_literal(budget)?;
        let magnitude = spelling.strip_prefix('-').unwrap_or(&spelling);
        budget.charge(magnitude.len())?;
        let canonical = magnitude.trim_end_matches('0');
        let exponent = exponent.add(
            &Integer::from_u128((magnitude.len() - canonical.len()) as u128),
            budget,
        )?;
        Ok(Self {
            digits: canonical.into(),
            exponent,
            negative: value.is_negative(),
        })
    }

    /// Exact finite multiplication. Zero signs multiply by exclusive-or.
    pub fn multiply(&self, other: &Self, budget: &mut Budget) -> Result<Self, Error> {
        let left = self.coefficient(budget)?;
        let right = other.coefficient(budget)?;
        let product = left.multiply(&right, budget)?;
        let exponent = if product.is_zero() {
            Integer::zero()
        } else {
            self.exponent.add(&other.exponent, budget)?
        };
        Self::from_coefficient(product, exponent, self.negative ^ other.negative, budget)
    }

    fn aligned_coefficient(
        &self,
        exponent: &Integer,
        budget: &mut Budget,
    ) -> Result<Integer, Error> {
        let shift = self
            .exponent
            .subtract(exponent, budget)?
            .to_u128()
            .map_err(|_| Error::SizeLimit)?;
        let shift = usize::try_from(shift).map_err(|_| Error::SizeLimit)?;
        let width = self
            .digits
            .len()
            .checked_add(shift)
            .ok_or(Error::SizeLimit)?;
        // Ten decimal digits per limb is an allocation upper bound; parsing
        // below enforces the exact binary limb limit.
        if width > budget.max_limbs.saturating_mul(10) {
            return Err(Error::SizeLimit);
        }
        budget.charge(width)?;
        let mut spelling = String::with_capacity(width);
        spelling.push_str(&self.digits);
        spelling.extend(std::iter::repeat_n('0', shift));
        let value = Integer::parse_literal(&spelling, budget)?;
        if self.negative {
            value.negate(budget)
        } else {
            Ok(value)
        }
    }

    /// Exact finite addition. Cancellation yields +0; two negative zeros
    /// retain -0. Huge alignment gaps fail before allocating their expansion.
    pub fn add(&self, other: &Self, budget: &mut Budget) -> Result<Self, Error> {
        self.coefficient(budget)?;
        other.coefficient(budget)?;
        if self.is_zero() && other.is_zero() {
            return Self::from_coefficient(
                Integer::zero(),
                Integer::zero(),
                self.negative && other.negative,
                budget,
            );
        }
        if self.is_zero() {
            budget.charge(other.digits.len())?;
            return Ok(other.clone());
        }
        if other.is_zero() {
            budget.charge(self.digits.len())?;
            return Ok(self.clone());
        }
        let exponent = std::cmp::min(&self.exponent, &other.exponent);
        let left = self.aligned_coefficient(exponent, budget)?;
        let right = other.aligned_coefficient(exponent, budget)?;
        let sum = left.add(&right, budget)?;
        Self::from_coefficient(sum, exponent.clone(), false, budget)
    }

    pub fn subtract(&self, other: &Self, budget: &mut Budget) -> Result<Self, Error> {
        self.add(&other.negate(budget)?, budget)
    }

    /// Negate without decimal rounding; both zero signs remain representable.
    pub fn negate(&self, budget: &mut Budget) -> Result<Self, Error> {
        budget.charge(
            self.digits
                .len()
                .saturating_add(self.exponent.bit_len().div_ceil(32))
                .saturating_add(1),
        )?;
        if self.exponent.bit_len().div_ceil(32) > budget.max_limbs {
            return Err(Error::SizeLimit);
        }
        Integer::parse_literal(&self.digits, budget)?;
        let mut result = self.clone();
        result.negative = !result.negative;
        Ok(result)
    }

    /// Compare exact finite decimals without constructing a power of ten.
    /// Work depends on stored significands/exponents, not exponent magnitude.
    pub fn compare(&self, other: &Self, budget: &mut Budget) -> Result<std::cmp::Ordering, Error> {
        use std::cmp::Ordering;
        budget.charge(
            self.digits
                .len()
                .saturating_add(other.digits.len())
                .saturating_add(1),
        )?;
        if self.is_zero() && other.is_zero() {
            return Ok(Ordering::Equal);
        }
        let left_negative = self.negative && !self.is_zero();
        let right_negative = other.negative && !other.is_zero();
        if left_negative != right_negative {
            return Ok(if left_negative {
                Ordering::Less
            } else {
                Ordering::Greater
            });
        }
        let magnitude = if self.is_zero() {
            Ordering::Less
        } else if other.is_zero() {
            Ordering::Greater
        } else {
            let left_order = self
                .exponent
                .add(&Integer::from_u128(self.digits.len() as u128), budget)?;
            let right_order = other
                .exponent
                .add(&Integer::from_u128(other.digits.len() as u128), budget)?;
            budget.charge(
                left_order
                    .bit_len()
                    .div_ceil(32)
                    .saturating_add(right_order.bit_len().div_ceil(32))
                    .saturating_add(1),
            )?;
            match left_order.cmp(&right_order) {
                Ordering::Equal => {
                    let width = self.digits.len().max(other.digits.len());
                    self.digits
                        .bytes()
                        .chain(std::iter::repeat(b'0'))
                        .take(width)
                        .cmp(
                            other
                                .digits
                                .bytes()
                                .chain(std::iter::repeat(b'0'))
                                .take(width),
                        )
                }
                different => different,
            }
        };
        Ok(if left_negative {
            magnitude.reverse()
        } else {
            magnitude
        })
    }
}
impl fmt::Display for DecimalValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(decimal-value (negative {}) (digits {:?}) (exponent {}))",
            self.negative, self.digits, self.exponent
        )
    }
}
