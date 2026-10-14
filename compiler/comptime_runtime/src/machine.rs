//! Checked fixed-width integer values. No wrapping, host pointer-width choice,
//! source operator selection or permission to bypass program verification.
use crate::Budget;
use std::fmt;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    InvalidWidth,
    OutOfRange,
    RepresentationMismatch,
    Overflow,
    DivisionByZero,
    Resource(crate::Error),
}
impl From<crate::Error> for Error {
    fn from(error: crate::Error) -> Self {
        Self::Resource(error)
    }
}
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidWidth => f.write_str("machine integer width must be in 1..=128"),
            Self::OutOfRange => f.write_str("machine integer input is out of range"),
            Self::RepresentationMismatch => {
                f.write_str("machine integer operand widths and signedness must agree")
            }
            Self::Overflow => f.write_str("machine integer arithmetic overflow"),
            Self::DivisionByZero => f.write_str("machine integer division by zero"),
            Self::Resource(error) => error.fmt(f),
        }
    }
}
impl std::error::Error for Error {}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct MachineInteger {
    bits: u128,
    width: u16,
    signed: bool,
}
impl MachineInteger {
    fn mask(width: u16) -> Result<u128, Error> {
        match width {
            1..=127 => Ok((1u128 << width) - 1),
            128 => Ok(u128::MAX),
            _ => Err(Error::InvalidWidth),
        }
    }
    pub fn from_bits(bits: u128, width: u16, signed: bool) -> Result<Self, Error> {
        let mask = Self::mask(width)?;
        if bits & !mask != 0 {
            return Err(Error::OutOfRange);
        }
        Ok(Self {
            bits,
            width,
            signed,
        })
    }
    pub fn from_unsigned(value: u128, width: u16) -> Result<Self, Error> {
        Self::from_bits(value, width, false)
    }
    pub fn from_signed(value: i128, width: u16) -> Result<Self, Error> {
        let mask = Self::mask(width)?;
        if width < 128 {
            let high = 1i128 << (width - 1);
            if value < -high || value >= high {
                return Err(Error::OutOfRange);
            }
        }
        Ok(Self {
            bits: (value as u128) & mask,
            width,
            signed: true,
        })
    }
    pub fn width(self) -> u16 {
        self.width
    }
    pub fn is_signed(self) -> bool {
        self.signed
    }
    pub fn bits(self) -> u128 {
        self.bits
    }
    pub fn signed_value(self) -> Option<i128> {
        if !self.signed {
            return None;
        }
        let extension = if self.bits & (1u128 << (self.width - 1)) != 0 {
            !Self::mask(self.width).expect("validated width")
        } else {
            0
        };
        Some((self.bits | extension) as i128)
    }
    pub fn unsigned_value(self) -> Option<u128> {
        (!self.signed).then_some(self.bits)
    }

    /// Numeric conversion, distinct from interpreting an explicit bit pattern.
    pub fn convert(self, width: u16, signed: bool) -> Result<Self, Error> {
        Self::mask(width)?;
        match (self.signed_value(), signed) {
            (Some(value), true) => Self::from_signed(value, width),
            (Some(value), false) => {
                Self::from_unsigned(u128::try_from(value).map_err(|_| Error::OutOfRange)?, width)
            }
            (None, true) => Self::from_signed(
                i128::try_from(self.bits).map_err(|_| Error::OutOfRange)?,
                width,
            ),
            (None, false) => Self::from_unsigned(self.bits, width),
        }
    }
    fn pair(self, other: Self, budget: &mut Budget) -> Result<(), Error> {
        // Constant-size, allocation-free work. Charge failures as well.
        budget.consume(1)?;
        if self.width != other.width || self.signed != other.signed {
            return Err(Error::RepresentationMismatch);
        }
        Ok(())
    }
    fn signed_result(self, value: Option<i128>) -> Result<Self, Error> {
        Self::from_signed(value.ok_or(Error::Overflow)?, self.width).map_err(|_| Error::Overflow)
    }
    fn unsigned_result(self, value: Option<u128>) -> Result<Self, Error> {
        Self::from_unsigned(value.ok_or(Error::Overflow)?, self.width).map_err(|_| Error::Overflow)
    }
    pub fn add(self, other: Self, budget: &mut Budget) -> Result<Self, Error> {
        self.pair(other, budget)?;
        match (self.signed_value(), other.signed_value()) {
            (Some(a), Some(b)) => self.signed_result(a.checked_add(b)),
            _ => self.unsigned_result(self.bits.checked_add(other.bits)),
        }
    }
    pub fn subtract(self, other: Self, budget: &mut Budget) -> Result<Self, Error> {
        self.pair(other, budget)?;
        match (self.signed_value(), other.signed_value()) {
            (Some(a), Some(b)) => self.signed_result(a.checked_sub(b)),
            _ => self.unsigned_result(self.bits.checked_sub(other.bits)),
        }
    }
    pub fn multiply(self, other: Self, budget: &mut Budget) -> Result<Self, Error> {
        self.pair(other, budget)?;
        match (self.signed_value(), other.signed_value()) {
            (Some(a), Some(b)) => self.signed_result(a.checked_mul(b)),
            _ => self.unsigned_result(self.bits.checked_mul(other.bits)),
        }
    }
    pub fn negate(self, budget: &mut Budget) -> Result<Self, Error> {
        budget.consume(1)?;
        match self.signed_value() {
            Some(value) => self.signed_result(value.checked_neg()),
            None => self.unsigned_result(0u128.checked_sub(self.bits)),
        }
    }
    /// Explicit internal truncation convention; this does not choose the
    /// rounding rule of a Flurry source operator. Both results must fit.
    pub fn div_rem_trunc(self, other: Self, budget: &mut Budget) -> Result<(Self, Self), Error> {
        self.pair(other, budget)?;
        if other.bits == 0 {
            return Err(Error::DivisionByZero);
        }
        match (self.signed_value(), other.signed_value()) {
            (Some(a), Some(b)) => {
                let quotient = self.signed_result(a.checked_div(b))?;
                let remainder = self.signed_result(a.checked_rem(b))?;
                Ok((quotient, remainder))
            }
            _ => Ok((
                self.unsigned_result(self.bits.checked_div(other.bits))?,
                self.unsigned_result(self.bits.checked_rem(other.bits))?,
            )),
        }
    }
}
impl fmt::Display for MachineInteger {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(machine-integer (signed {}) (width {}) (bits 0x{:x}))",
            self.signed, self.width, self.bits
        )
    }
}
