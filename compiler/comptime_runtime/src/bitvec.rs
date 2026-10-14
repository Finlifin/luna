//! Defined bit values, independent of storage layout, locations and ownership.
//! Index zero is the least significant bit. This kernel grants no access or
//! unsafe construction permission and does not perform synthesis.
use crate::Budget;
use std::fmt;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    WidthMismatch,
    OutOfBounds,
    Narrowing,
    InvalidRepresentation,
    Resource(crate::Error),
}
impl From<crate::Error> for Error {
    fn from(value: crate::Error) -> Self {
        Self::Resource(value)
    }
}
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::WidthMismatch => "bit-vector operands must have equal widths",
            Self::OutOfBounds => "bit-vector index or slice is out of bounds",
            Self::Narrowing => "bit-vector narrowing requires an explicit slice",
            Self::InvalidRepresentation => {
                "bit-vector representation contains missing or excess bits"
            }
            Self::Resource(crate::Error::BudgetExhausted) => {
                "bit-vector operation budget exhausted"
            }
            Self::Resource(_) => "bit-vector representation size limit exceeded",
        })
    }
}
impl std::error::Error for Error {}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct BitVec {
    width: usize,
    words: Vec<u32>,
}
fn count(width: usize) -> usize {
    width / 32 + usize::from(width % 32 != 0)
}
fn reserve(width: usize, budget: &mut Budget) -> Result<usize, Error> {
    let words = count(width);
    if words > budget.max_limbs {
        return Err(crate::Error::SizeLimit.into());
    }
    budget.charge(words.saturating_add(1))?;
    Ok(words)
}
impl BitVec {
    /// Exact little-endian words. Reject nonzero padding; never truncate input.
    pub fn from_words(width: usize, words: &[u32], budget: &mut Budget) -> Result<Self, Error> {
        if words.len() != count(width) {
            return Err(Error::InvalidRepresentation);
        }
        reserve(width, budget)?;
        if width % 32 != 0 && words.last().is_some_and(|w| *w >> (width % 32) != 0) {
            return Err(Error::InvalidRepresentation);
        }
        Ok(Self {
            width,
            words: words.to_vec(),
        })
    }
    pub fn width(&self) -> usize {
        self.width
    }
    pub fn words(&self) -> &[u32] {
        &self.words
    }
    fn zero(width: usize, budget: &mut Budget) -> Result<Self, Error> {
        let words = reserve(width, budget)?;
        Ok(Self {
            width,
            words: vec![0; words],
        })
    }
    fn mask_tail(&mut self) {
        if self.width % 32 != 0 {
            if let Some(last) = self.words.last_mut() {
                *last &= (1u32 << (self.width % 32)) - 1;
            }
        }
    }
    fn combine(
        &self,
        other: &Self,
        budget: &mut Budget,
        op: fn(u32, u32) -> u32,
    ) -> Result<Self, Error> {
        if self.width != other.width {
            return Err(Error::WidthMismatch);
        }
        reserve(self.width, budget)?;
        Ok(Self {
            width: self.width,
            words: self
                .words
                .iter()
                .zip(&other.words)
                .map(|(&a, &b)| op(a, b))
                .collect(),
        })
    }
    pub fn and(&self, other: &Self, budget: &mut Budget) -> Result<Self, Error> {
        self.combine(other, budget, |a, b| a & b)
    }
    pub fn or(&self, other: &Self, budget: &mut Budget) -> Result<Self, Error> {
        self.combine(other, budget, |a, b| a | b)
    }
    pub fn not(&self, budget: &mut Budget) -> Result<Self, Error> {
        reserve(self.width, budget)?;
        let mut result = Self {
            width: self.width,
            words: self.words.iter().map(|w| !w).collect(),
        };
        result.mask_tail();
        Ok(result)
    }
    pub fn bit(&self, index: usize, budget: &mut Budget) -> Result<Self, Error> {
        if index >= self.width {
            return Err(Error::OutOfBounds);
        }
        self.slice(index, index + 1, budget)
    }
    pub fn slice(&self, lo: usize, hi: usize, budget: &mut Budget) -> Result<Self, Error> {
        if lo > hi || hi > self.width {
            return Err(Error::OutOfBounds);
        }
        let mut result = Self::zero(hi - lo, budget)?;
        let offset = lo % 32;
        for (i, word) in result.words.iter_mut().enumerate() {
            let source = lo / 32 + i;
            *word = self.words[source] >> offset;
            if offset != 0 {
                *word |= self.words.get(source + 1).copied().unwrap_or(0) << (32 - offset);
            }
        }
        result.mask_tail();
        Ok(result)
    }
    /// `self ++ low`: left operand at the high end, right at the low end.
    pub fn concat(&self, low: &Self, budget: &mut Budget) -> Result<Self, Error> {
        let width = self
            .width
            .checked_add(low.width)
            .ok_or(crate::Error::SizeLimit)?;
        let mut result = Self::zero(width, budget)?;
        result.words[..low.words.len()].copy_from_slice(&low.words);
        let offset = low.width % 32;
        for (i, &word) in self.words.iter().enumerate() {
            let target = low.width / 32 + i;
            result.words[target] |= word << offset;
            if offset != 0 && target + 1 < result.words.len() {
                result.words[target + 1] |= word >> (32 - offset);
            }
        }
        Ok(result)
    }
    /// Replace a value slice after the caller has obtained the complete RHS.
    /// This returns a new value; it grants no permission to mutate a location.
    pub fn replaced(&self, start: usize, rhs: &Self, budget: &mut Budget) -> Result<Self, Error> {
        let end = start.checked_add(rhs.width).ok_or(Error::OutOfBounds)?;
        if end > self.width {
            return Err(Error::OutOfBounds);
        }
        reserve(self.width, budget)?;
        budget.charge(rhs.words.len().saturating_add(1))?;
        let mut result = self.clone();
        if start == end {
            return Ok(result);
        }
        // Clear exactly the selected interval, leaving both boundary words'
        // remaining bits intact. Avoid shifting by the machine word width.
        for index in start / 32..=(end - 1) / 32 {
            let lo = start.saturating_sub(index * 32).min(32);
            let hi = (end - index * 32).min(32);
            let low_mask = if lo == 0 { 0 } else { (1u32 << lo) - 1 };
            let high_mask = if hi == 32 { 0 } else { u32::MAX << hi };
            result.words[index] &= low_mask | high_mask;
        }
        let offset = start % 32;
        for (i, &word) in rhs.words.iter().enumerate() {
            let target = start / 32 + i;
            result.words[target] |= word << offset;
            if offset != 0 && target + 1 < result.words.len() {
                result.words[target + 1] |= word >> (32 - offset);
            }
        }
        Ok(result)
    }
    pub fn widen(&self, width: usize, budget: &mut Budget) -> Result<Self, Error> {
        if width < self.width {
            return Err(Error::Narrowing);
        }
        let mut result = Self::zero(width, budget)?;
        result.words[..self.words.len()].copy_from_slice(&self.words);
        Ok(result)
    }
}
impl fmt::Display for BitVec {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "(bitvec (width {}) (hex \"", self.width)?;
        if self.words.is_empty() {
            f.write_str("0")?;
        }
        for word in self.words.iter().rev() {
            write!(f, "{word:08x}")?;
        }
        f.write_str("\"))")
    }
}
