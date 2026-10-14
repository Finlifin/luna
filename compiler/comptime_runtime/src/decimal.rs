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

    /// IEEE nearest, ties-to-even conversion of this decimal literal. An
    /// overflowing finite literal is left to the caller's conversion policy.
    pub fn to_f32_bits(&self, budget: &mut Budget) -> Result<u32, Error> {
        budget.charge(self.spelling.len().saturating_add(1))?;
        let value = self
            .spelling
            .parse::<f32>()
            .map_err(|_| Error::InvalidLiteral)?;
        if !value.is_finite() {
            return Err(Error::OutOfRange);
        }
        Ok(value.to_bits() | if self.negative { 1 << 31 } else { 0 })
    }
    pub fn to_f64_bits(&self, budget: &mut Budget) -> Result<u64, Error> {
        budget.charge(self.spelling.len().saturating_add(1))?;
        let value = self
            .spelling
            .parse::<f64>()
            .map_err(|_| Error::InvalidLiteral)?;
        if !value.is_finite() {
            return Err(Error::OutOfRange);
        }
        Ok(value.to_bits() | if self.negative { 1 << 63 } else { 0 })
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
