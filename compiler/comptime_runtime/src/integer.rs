//! Canonical signed, arbitrary-precision integers. No host-width truncation.
use std::cmp::Ordering;
use std::fmt;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    InvalidLiteral,
    BudgetExhausted,
    SizeLimit,
    DivisionByZero,
    OutOfRange,
}
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::InvalidLiteral => "invalid integer literal",
            Self::BudgetExhausted => "integer operation budget exhausted",
            Self::SizeLimit => "integer representation size limit exceeded",
            Self::DivisionByZero => "integer division by zero",
            Self::OutOfRange => "integer does not fit the requested fixed-width representation",
        })
    }
}
impl std::error::Error for Error {}

/// Shared fuel is consumed on failures too; inputs are never mutated.
/// max_limbs bounds every individual magnitude buffer, including scratch.
#[derive(Clone, Debug)]
pub struct Budget {
    pub steps: usize,
    pub max_limbs: usize,
}
impl Default for Budget {
    fn default() -> Self {
        Self {
            steps: 1_000_000,
            max_limbs: 4096,
        }
    }
}
impl Budget {
    /// Charge caller-side value work against the same operation budget.
    pub fn consume(&mut self, steps: usize) -> Result<(), Error> {
        self.charge(steps)
    }
    pub(crate) fn charge(&mut self, steps: usize) -> Result<(), Error> {
        match self.steps.checked_sub(steps) {
            Some(remaining) => {
                self.steps = remaining;
                Ok(())
            }
            None => {
                self.steps = 0;
                Err(Error::BudgetExhausted)
            }
        }
    }
    fn size(&self, limbs: usize) -> Result<(), Error> {
        if limbs > self.max_limbs {
            Err(Error::SizeLimit)
        } else {
            Ok(())
        }
    }
}

/// Little-endian base-2^32 magnitude, with no high zero limbs. Zero is positive.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Integer {
    negative: bool,
    limbs: Vec<u32>,
}
impl Integer {
    fn canonical(negative: bool, mut limbs: Vec<u32>) -> Self {
        while limbs.last() == Some(&0) {
            limbs.pop();
        }
        Self {
            negative: negative && !limbs.is_empty(),
            limbs,
        }
    }
    pub fn zero() -> Self {
        Self {
            negative: false,
            limbs: vec![],
        }
    }
    pub fn is_zero(&self) -> bool {
        self.limbs.is_empty()
    }
    /// Canonical source spelling for transporting an already evaluated value.
    /// Display remains an S-expression and must not be reparsed as a literal.
    pub fn to_hex_literal(&self, budget: &mut Budget) -> Result<String, Error> {
        use std::fmt::Write;
        budget.size(self.limbs.len())?;
        budget.charge(self.limbs.len().saturating_mul(8).saturating_add(3))?;
        let mut text = if self.negative {
            String::from("-0x")
        } else {
            String::from("0x")
        };
        if let Some(last) = self.limbs.last() {
            write!(&mut text, "{last:x}").unwrap();
            for limb in self.limbs[..self.limbs.len() - 1].iter().rev() {
                write!(&mut text, "{limb:08x}").unwrap();
            }
        } else {
            text.push('0');
        }
        Ok(text)
    }
    /// Budgeted canonical decimal transport spelling, independent of Display.
    pub fn to_decimal_literal(&self, budget: &mut Budget) -> Result<String, Error> {
        use std::fmt::Write;
        budget.size(self.limbs.len())?;
        budget.charge(self.limbs.len().saturating_add(1))?;
        let mut magnitude = self.limbs.clone();
        let mut chunks = Vec::new();
        while !magnitude.is_empty() {
            budget.charge(magnitude.len().saturating_add(9))?;
            let mut remainder = 0u64;
            for limb in magnitude.iter_mut().rev() {
                let wide = (remainder << 32) | u64::from(*limb);
                *limb = (wide / 1_000_000_000) as u32;
                remainder = wide % 1_000_000_000;
            }
            chunks.push(remainder as u32);
            while magnitude.last() == Some(&0) {
                magnitude.pop();
            }
        }
        let mut text = String::new();
        if self.negative {
            text.push('-');
        }
        if let Some(last) = chunks.pop() {
            write!(&mut text, "{last}").unwrap();
            for chunk in chunks.into_iter().rev() {
                write!(&mut text, "{chunk:09}").unwrap();
            }
        } else {
            text.push('0');
        }
        Ok(text)
    }

    pub fn is_negative(&self) -> bool {
        self.negative
    }
    pub fn bit_len(&self) -> usize {
        self.limbs.last().map_or(0, |last| {
            (self.limbs.len() - 1) * 32 + (32 - last.leading_zeros()) as usize
        })
    }
    pub fn negated(&self) -> Self {
        let mut result = self.clone();
        if !result.is_zero() {
            result.negative = !result.negative;
        }
        result
    }
    /// Negate with the same size/work bounds as other public arithmetic.
    pub fn negate(&self, budget: &mut Budget) -> Result<Self, Error> {
        budget.size(self.limbs.len())?;
        budget.consume(self.limbs.len().saturating_add(1))?;
        Ok(self.negated())
    }
    pub fn from_u128(mut value: u128) -> Self {
        let mut limbs = Vec::new();
        while value != 0 {
            limbs.push(value as u32);
            value >>= 32;
        }
        Self::canonical(false, limbs)
    }
    pub fn from_i128(value: i128) -> Self {
        let magnitude = Self::from_u128(value.unsigned_abs());
        Self::canonical(value < 0, magnitude.limbs)
    }
    fn magnitude_cmp(&self, other: &Self) -> Ordering {
        self.limbs
            .len()
            .cmp(&other.limbs.len())
            .then_with(|| self.limbs.iter().rev().cmp(other.limbs.iter().rev()))
    }
    fn start(&self, other: &Self, budget: &mut Budget) -> Result<(), Error> {
        budget.size(self.limbs.len().max(other.limbs.len()))?;
        budget.charge(1)
    }
    /// Unsuffixed non-negative lexical integer. Unary signs are separate syntax.
    pub fn parse_literal(text: &str, budget: &mut Budget) -> Result<Self, Error> {
        budget.charge(1)?;
        let (radix, digits) = if let Some(s) = text.strip_prefix("0x") {
            (16, s)
        } else if let Some(s) = text.strip_prefix("0o") {
            (8, s)
        } else if let Some(s) = text.strip_prefix("0b") {
            (2, s)
        } else {
            (10, text)
        };
        if digits.is_empty() || digits.starts_with('_') || digits.ends_with('_') {
            return Err(Error::InvalidLiteral);
        }
        let mut value = Self::zero();
        let mut hex_upper = None;
        for byte in digits.bytes() {
            budget.charge(1)?;
            if byte == b'_' {
                continue;
            }
            let digit = match byte {
                b'0'..=b'9' => byte - b'0',
                b'a'..=b'f' => byte - b'a' + 10,
                b'A'..=b'F' => byte - b'A' + 10,
                _ => return Err(Error::InvalidLiteral),
            };
            if u32::from(digit) >= radix {
                return Err(Error::InvalidLiteral);
            }
            if byte.is_ascii_alphabetic() {
                let upper = byte.is_ascii_uppercase();
                if hex_upper.is_some_and(|old| old != upper) {
                    return Err(Error::InvalidLiteral);
                }
                hex_upper = Some(upper);
            }
            let mut carry = u64::from(digit);
            for limb in &mut value.limbs {
                budget.charge(1)?;
                let next = u64::from(*limb) * u64::from(radix) + carry;
                *limb = next as u32;
                carry = next >> 32;
            }
            if carry != 0 {
                budget.size(value.limbs.len() + 1)?;
                value.limbs.push(carry as u32);
            }
        }
        Ok(value)
    }
    pub fn add(&self, other: &Self, budget: &mut Budget) -> Result<Self, Error> {
        self.start(other, budget)?;
        if self.negative != other.negative {
            budget.charge(self.limbs.len().max(other.limbs.len()))?;
            return match self.magnitude_cmp(other) {
                Ordering::Less => other.subtract_magnitude(self, other.negative, budget),
                _ => self.subtract_magnitude(other, self.negative, budget),
            };
        }
        let count = self.limbs.len().max(other.limbs.len());
        let mut limbs = Vec::with_capacity(count);
        let mut carry = 0u64;
        for index in 0..count {
            budget.charge(1)?;
            let next = u64::from(*self.limbs.get(index).unwrap_or(&0))
                + u64::from(*other.limbs.get(index).unwrap_or(&0))
                + carry;
            limbs.push(next as u32);
            carry = next >> 32;
        }
        if carry != 0 {
            budget.size(count + 1)?;
            limbs.push(carry as u32);
        }
        Ok(Self::canonical(self.negative, limbs))
    }
    // The caller establishes |self| >= |other|.
    fn subtract_magnitude(
        &self,
        other: &Self,
        negative: bool,
        budget: &mut Budget,
    ) -> Result<Self, Error> {
        let mut limbs = Vec::with_capacity(self.limbs.len());
        let mut borrow = 0u64;
        for (index, limb) in self.limbs.iter().enumerate() {
            budget.charge(1)?;
            let rhs = u64::from(*other.limbs.get(index).unwrap_or(&0)) + borrow;
            limbs.push(u64::from(*limb).wrapping_sub(rhs) as u32);
            borrow = u64::from(u64::from(*limb) < rhs);
        }
        debug_assert_eq!(borrow, 0);
        Ok(Self::canonical(negative, limbs))
    }
    pub fn subtract(&self, other: &Self, budget: &mut Budget) -> Result<Self, Error> {
        self.start(other, budget)?;
        budget.charge(other.limbs.len())?;
        self.add(&other.negated(), budget)
    }
    pub fn multiply(&self, other: &Self, budget: &mut Budget) -> Result<Self, Error> {
        self.start(other, budget)?;
        if self.is_zero() || other.is_zero() {
            return Ok(Self::zero());
        }
        let size = self
            .limbs
            .len()
            .checked_add(other.limbs.len())
            .ok_or(Error::SizeLimit)?;
        budget.size(size)?;
        let mut limbs = vec![0u32; size];
        for (i, a) in self.limbs.iter().enumerate() {
            let mut carry = 0u64;
            for (j, b) in other.limbs.iter().enumerate() {
                budget.charge(1)?;
                let value = u64::from(*a) * u64::from(*b) + u64::from(limbs[i + j]) + carry;
                limbs[i + j] = value as u32;
                carry = value >> 32;
            }
            limbs[i + other.limbs.len()] = carry as u32;
        }
        Ok(Self::canonical(self.negative != other.negative, limbs))
    }
    /// Explicit truncation-toward-zero kernel; does not decide a language
    /// division protocol. Remainder has the dividend's sign and |r| < |divisor|.
    pub fn div_rem_trunc(
        &self,
        divisor: &Self,
        budget: &mut Budget,
    ) -> Result<(Self, Self), Error> {
        self.start(divisor, budget)?;
        if divisor.is_zero() {
            return Err(Error::DivisionByZero);
        }
        if let [divisor_word] = divisor.limbs.as_slice() {
            // Base-2^32 long division needs one u64 division per input limb.
            // The carry is smaller than the divisor, so (carry << 32) | limb
            // fits u64, including a maximal u32 divisor. Charge traversal and
            // canonicalization before allocating the quotient.
            budget.charge(self.limbs.len().saturating_mul(2).saturating_add(1))?;
            let mut quotient = vec![0; self.limbs.len()];
            let divisor_word = u64::from(*divisor_word);
            let mut carry = 0u64;
            for (limb, output) in self.limbs.iter().zip(&mut quotient).rev() {
                let dividend = (carry << 32) | u64::from(*limb);
                *output = (dividend / divisor_word) as u32;
                carry = dividend % divisor_word;
            }
            let remainder = if carry == 0 {
                vec![]
            } else {
                vec![carry as u32]
            };
            return Ok((
                Self::canonical(self.negative != divisor.negative, quotient),
                Self::canonical(self.negative, remainder),
            ));
        }
        let mut quotient = vec![0u32; self.limbs.len()];
        let mut remainder = Self::zero();
        for bit in (0..self.bit_len()).rev() {
            budget.charge(1)?;
            let mut carry = (self.limbs[bit / 32] >> (bit % 32)) & 1;
            for limb in &mut remainder.limbs {
                budget.charge(1)?;
                let next = *limb >> 31;
                *limb = (*limb << 1) | carry;
                carry = next;
            }
            if carry != 0 {
                budget.size(remainder.limbs.len() + 1)?;
                remainder.limbs.push(carry);
            }
            budget.charge(remainder.limbs.len().max(divisor.limbs.len()))?;
            if remainder.magnitude_cmp(divisor) != Ordering::Less {
                remainder = remainder.subtract_magnitude(divisor, false, budget)?;
                quotient[bit / 32] |= 1 << (bit % 32);
            }
        }
        Ok((
            Self::canonical(self.negative != divisor.negative, quotient),
            Self::canonical(self.negative, remainder.limbs),
        ))
    }
    /// Checked nonnegative literal conversion. Leading zeroes do not select
    /// width, and a narrower target never authorizes truncation.
    pub fn to_bitvec(
        &self,
        width: usize,
        budget: &mut Budget,
    ) -> Result<crate::bitvec::BitVec, crate::bitvec::Error> {
        budget.charge(1)?;
        if self.negative || self.bit_len() > width {
            return Err(crate::bitvec::Error::InvalidRepresentation);
        }
        let count = width / 32 + usize::from(width % 32 != 0);
        budget.size(count)?;
        budget.charge(count)?;
        let mut words = vec![0; count];
        words[..self.limbs.len()].copy_from_slice(&self.limbs);
        crate::bitvec::BitVec::from_words(width, &words, budget)
    }
    pub fn to_u128(&self) -> Result<u128, Error> {
        if self.negative || self.limbs.len() > 4 {
            return Err(Error::OutOfRange);
        }
        Ok(self
            .limbs
            .iter()
            .rev()
            .fold(0u128, |value, limb| (value << 32) | u128::from(*limb)))
    }
    pub fn to_i128(&self) -> Result<i128, Error> {
        if self.limbs.len() > 4 {
            return Err(Error::OutOfRange);
        }
        let magnitude = self
            .limbs
            .iter()
            .rev()
            .fold(0u128, |value, limb| (value << 32) | u128::from(*limb));
        if self.negative {
            if magnitude == 1u128 << 127 {
                Ok(i128::MIN)
            } else {
                i128::try_from(magnitude)
                    .map(|n| -n)
                    .map_err(|_| Error::OutOfRange)
            }
        } else {
            i128::try_from(magnitude).map_err(|_| Error::OutOfRange)
        }
    }
    /// Exact conversion for 1..=128-bit unsigned proxies. No wrapping.
    pub fn to_unsigned(&self, width: u32) -> Result<u128, Error> {
        if width == 0 || width > 128 || self.bit_len() > width as usize {
            return Err(Error::OutOfRange);
        }
        self.to_u128()
    }
    /// Exact conversion for 1..=128-bit signed proxies. No wrapping.
    pub fn to_signed(&self, width: u32) -> Result<i128, Error> {
        if width == 0 || width > 128 {
            return Err(Error::OutOfRange);
        }
        let value = self.to_i128()?;
        if width < 128 && (value < -(1i128 << (width - 1)) || value >= (1i128 << (width - 1))) {
            return Err(Error::OutOfRange);
        }
        Ok(value)
    }
}
impl Ord for Integer {
    fn cmp(&self, other: &Self) -> Ordering {
        match (self.negative, other.negative) {
            (true, false) => Ordering::Less,
            (false, true) => Ordering::Greater,
            (false, false) => self.magnitude_cmp(other),
            (true, true) => self.magnitude_cmp(other).reverse(),
        }
    }
}
impl PartialOrd for Integer {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl fmt::Display for Integer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(integer (sign {}) (hex \"",
            if self.negative {
                "negative"
            } else {
                "positive"
            }
        )?;
        if let Some(last) = self.limbs.last() {
            write!(f, "{last:x}")?;
            for limb in self.limbs[..self.limbs.len() - 1].iter().rev() {
                write!(f, "{limb:08x}")?;
            }
        } else {
            f.write_str("0")?;
        }
        f.write_str("\"))")
    }
}
