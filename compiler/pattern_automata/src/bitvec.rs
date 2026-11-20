//! Arbitrary-width `BitVec<N>` values, intervals, segment alignment, and mask partitioning.

use crate::decision::{Binding, Place};
use crate::pattern::{
    BitSegmentPattern, BitVecPattern, CompileError, IntLit, IntType, Pattern, RangeBound, TypeDesc,
};
use crate::sexpr;
use std::collections::BTreeSet;
use std::fmt;

/// Error produced when constructing or parsing a [`BitValue`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BitVecError {
    /// Value exceeds the target bit width.
    Overflow { width: u32, detail: String },
    /// Negative or out-of-range value for signed/unsigned conversion.
    InvalidWidth { width: u32, detail: String },
    /// Invalid radix or malformed literal digits.
    InvalidLiteral { radix: u32, text: String },
}

impl fmt::Display for BitVecError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Overflow { width, detail } => {
                write!(f, "(bitvec-error overflow (width {width}) ")?;
                sexpr::write_quoted_str(f, detail)?;
                f.write_str(")")
            }
            Self::InvalidWidth { width, detail } => {
                write!(f, "(bitvec-error invalid-width (width {width}) ")?;
                sexpr::write_quoted_str(f, detail)?;
                f.write_str(")")
            }
            Self::InvalidLiteral { radix, text } => {
                write!(f, "(bitvec-error invalid-literal (radix {radix}) ")?;
                sexpr::write_quoted_str(f, text)?;
                f.write_str(")")
            }
        }
    }
}

impl std::error::Error for BitVecError {}

/// An arbitrary-width bit vector (`width >= 0`), stored LSB-first in `u64` words.
///
/// Bit `0` is the least significant bit and bit `width - 1` is the most significant bit,
/// matching Flurry's `BitVec<N>` representation and `bits[lo..hi]` indexing.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct BitValue {
    width: u32,
    words: Vec<u64>,
}

impl PartialOrd for BitValue {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for BitValue {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.width.cmp(&other.width).then_with(|| {
            for idx in (0..self.words.len()).rev() {
                let ord = self.words[idx].cmp(&other.words[idx]);
                if ord != std::cmp::Ordering::Equal {
                    return ord;
                }
            }
            std::cmp::Ordering::Equal
        })
    }
}

impl BitValue {
    /// Construct a zeroed bit vector of `width` bits.
    pub fn zero(width: u32) -> Self {
        let word_count = word_count_for(width);
        Self {
            width,
            words: vec![0u64; word_count],
        }
    }

    /// Construct an empty `BitVec<0>`.
    pub fn empty() -> Self {
        Self::zero(0)
    }

    /// Construct a bit vector of `width` bits with every bit set to `1`.
    pub fn all_ones(width: u32) -> Self {
        let mut out = Self::zero(width);
        for word in &mut out.words {
            *word = u64::MAX;
        }
        out.mask_top_word();
        out
    }

    /// Construct a 1-bit `BitValue` from a `bool`.
    pub fn from_bool(value: bool) -> Self {
        let mut out = Self::zero(1);
        if value {
            out.words[0] = 1;
        }
        out
    }

    /// Construct a `BitValue` of `width` bits from an unsigned `u64`.
    pub fn from_u64(width: u32, value: u64) -> Result<Self, BitVecError> {
        Self::from_u128(width, value as u128)
    }

    /// Construct a `BitValue` of `width` bits from an unsigned `u128`.
    pub fn from_u128(width: u32, value: u128) -> Result<Self, BitVecError> {
        if width == 0 {
            if value != 0 {
                return Err(BitVecError::Overflow {
                    width,
                    detail: format!("non-zero value {value} for width 0"),
                });
            }
            return Ok(Self::empty());
        }
        if width < 128 && value >= (1u128 << width) {
            return Err(BitVecError::Overflow {
                width,
                detail: format!("value {value} exceeds {width} unsigned bits"),
            });
        }
        let mut out = Self::zero(width);
        out.words[0] = value as u64;
        if out.words.len() > 1 {
            out.words[1] = (value >> 64) as u64;
        }
        out.mask_top_word();
        Ok(out)
    }

    /// Construct a `BitValue` of `width` bits from a signed `i128` (two's complement).
    pub fn from_i128(width: u32, value: i128) -> Result<Self, BitVecError> {
        if width == 0 || width > 128 {
            return Err(BitVecError::InvalidWidth {
                width,
                detail: "signed i128 conversion requires 1..=128 bits".to_string(),
            });
        }
        if width < 128 {
            let min = -(1i128 << (width - 1));
            let max = (1i128 << (width - 1)) - 1;
            if value < min || value > max {
                return Err(BitVecError::Overflow {
                    width,
                    detail: format!(
                        "value {value} exceeds signed {width}-bit range [{min}, {max}]"
                    ),
                });
            }
        }
        let raw = value as u128;
        let masked = if width == 128 {
            raw
        } else {
            raw & ((1u128 << width) - 1)
        };
        Self::from_u128(width, masked)
    }

    /// Construct a `BitValue` from little-endian `u64` words, validating no bits >= `width` are set.
    pub fn from_words(width: u32, mut words: Vec<u64>) -> Result<Self, BitVecError> {
        let needed = word_count_for(width);
        if words.len() > needed {
            for &extra in &words[needed..] {
                if extra != 0 {
                    return Err(BitVecError::Overflow {
                        width,
                        detail: "excess non-zero words beyond bit width".to_string(),
                    });
                }
            }
            words.truncate(needed);
        }
        if needed > 0 {
            while words.len() < needed {
                words.push(0);
            }
            let top_bits = width % 64;
            if top_bits != 0 {
                let top_mask = (1u64 << top_bits) - 1;
                if (words[needed - 1] & !top_mask) != 0 {
                    return Err(BitVecError::Overflow {
                        width,
                        detail: "upper bits set beyond bit width".to_string(),
                    });
                }
            }
        }
        Ok(Self { width, words })
    }

    /// Parse a Flurry fixed-bit literal segment under radix `2`, `8`, or `16`.
    ///
    /// Underscores `_` are ignored; each digit contributes 1, 3, or 4 bits MSB-first.
    pub fn from_radix_str(radix: u32, text: &str) -> Result<Self, BitVecError> {
        let bits_per_digit = match radix {
            2 => 1u32,
            8 => 3u32,
            16 => 4u32,
            _ => {
                return Err(BitVecError::InvalidLiteral {
                    radix,
                    text: text.to_string(),
                });
            }
        };
        let digits: Vec<u8> = text
            .chars()
            .filter(|&c| c != '_')
            .map(|c| {
                c.to_digit(radix)
                    .map(|d| d as u8)
                    .ok_or_else(|| BitVecError::InvalidLiteral {
                        radix,
                        text: text.to_string(),
                    })
            })
            .collect::<Result<Vec<_>, _>>()?;
        if digits.is_empty() {
            return Err(BitVecError::InvalidLiteral {
                radix,
                text: text.to_string(),
            });
        }
        let total_width = (digits.len() as u32)
            .checked_mul(bits_per_digit)
            .ok_or_else(|| BitVecError::InvalidLiteral {
                radix,
                text: text.to_string(),
            })?;
        let mut out = Self::zero(total_width);
        let mut cursor = total_width;
        for digit in digits {
            cursor -= bits_per_digit;
            for b in 0..bits_per_digit {
                if ((digit >> b) & 1) != 0 {
                    out.set_bit(cursor + b, true);
                }
            }
        }
        Ok(out)
    }

    /// Total number of bits.
    pub fn width(&self) -> u32 {
        self.width
    }

    /// Underlying little-endian `u64` words.
    pub fn words(&self) -> &[u64] {
        &self.words
    }

    /// Return `true` if all bits are `0` (including `width == 0`).
    pub fn is_zero(&self) -> bool {
        self.words.iter().all(|&w| w == 0)
    }

    /// Read bit `index` (`0` = LSB).
    pub fn bit(&self, index: u32) -> bool {
        assert!(
            index < self.width,
            "bit index {index} out of bounds for width {}",
            self.width
        );
        let word_idx = (index / 64) as usize;
        let bit_idx = index % 64;
        ((self.words[word_idx] >> bit_idx) & 1) != 0
    }

    /// Set bit `index` (`0` = LSB).
    pub fn set_bit(&mut self, index: u32, value: bool) {
        assert!(
            index < self.width,
            "bit index {index} out of bounds for width {}",
            self.width
        );
        let word_idx = (index / 64) as usize;
        let bit_idx = index % 64;
        if value {
            self.words[word_idx] |= 1u64 << bit_idx;
        } else {
            self.words[word_idx] &= !(1u64 << bit_idx);
        }
    }

    /// Extract half-open bit slice `[lo, hi)` (`0 <= lo <= hi <= width`).
    pub fn slice(&self, lo: u32, hi: u32) -> Self {
        assert!(
            lo <= hi && hi <= self.width,
            "invalid slice [{lo}..{hi}) for width {}",
            self.width
        );
        let slice_width = hi - lo;
        if slice_width == 0 {
            return Self::empty();
        }
        let mut out = Self::zero(slice_width);
        let word_shift = (lo / 64) as usize;
        let bit_shift = lo % 64;
        for i in 0..out.words.len() {
            let low_part = self.words.get(i + word_shift).copied().unwrap_or(0) >> bit_shift;
            let high_part = if bit_shift == 0 {
                0
            } else {
                self.words.get(i + word_shift + 1).copied().unwrap_or(0) << (64 - bit_shift)
            };
            out.words[i] = low_part | high_part;
        }
        out.mask_top_word();
        out
    }

    /// Overwrite bits `[lo, lo + src.width())` with `src`.
    pub fn splice_from(&mut self, lo: u32, src: &Self) {
        assert!(
            lo + src.width <= self.width,
            "splice [{lo}..{}) out of bounds for width {}",
            lo + src.width,
            self.width
        );
        for offset in 0..src.width {
            self.set_bit(lo + offset, src.bit(offset));
        }
    }

    /// Concatenate `self` (MSBs) with `low` (LSBs), matching Flurry's `self ++ low`.
    pub fn concat(&self, low: &Self) -> Self {
        let total = self.width + low.width;
        let mut out = Self::zero(total);
        out.splice_from(0, low);
        out.splice_from(low.width, self);
        out
    }

    /// Add `1`, returning `None` if `self` is already all ones (or width 0).
    pub fn checked_add_one(&self) -> Option<Self> {
        if self.width == 0 {
            return None;
        }
        let mut out = self.clone();
        let mut carry = true;
        for word in &mut out.words {
            if carry {
                let (next, wrapped) = word.overflowing_add(1);
                *word = next;
                carry = wrapped;
            } else {
                break;
            }
        }
        if carry {
            return None;
        }
        let top_bits = self.width % 64;
        if top_bits != 0 {
            let max_top = (1u64 << top_bits) - 1;
            let last = out.words.len() - 1;
            if out.words[last] > max_top {
                return None;
            }
        }
        Some(out)
    }

    /// Subtract `1`, returning `None` if `self` is already zero.
    pub fn checked_sub_one(&self) -> Option<Self> {
        if self.is_zero() {
            return None;
        }
        let mut out = self.clone();
        let mut borrow = true;
        for word in &mut out.words {
            if borrow {
                let (next, wrapped) = word.overflowing_sub(1);
                *word = next;
                borrow = wrapped;
            } else {
                break;
            }
        }
        debug_assert!(!borrow);
        Some(out)
    }

    /// Convert to unsigned `u128` when `width <= 128`.
    pub fn to_u128(&self) -> Option<u128> {
        if self.width > 128 {
            return None;
        }
        let w0 = self.words.first().copied().unwrap_or(0) as u128;
        let w1 = self.words.get(1).copied().unwrap_or(0) as u128;
        Some(w0 | (w1 << 64))
    }

    /// Convert to signed two's-complement `i128` when `1 <= width <= 128`.
    pub fn to_signed_i128(&self) -> Option<i128> {
        if self.width == 0 || self.width > 128 {
            return None;
        }
        let raw = self.to_u128()?;
        let shift = 128 - self.width;
        Some(((raw << shift) as i128) >> shift)
    }

    /// Render as `0b...` binary string MSB-first (`0b` when width is 0).
    pub fn to_bin_string(&self) -> String {
        let mut s = String::with_capacity(2 + self.width as usize);
        s.push_str("0b");
        for i in (0..self.width).rev() {
            s.push(if self.bit(i) { '1' } else { '0' });
        }
        s
    }

    fn mask_top_word(&mut self) {
        let top_bits = self.width % 64;
        if top_bits != 0 && !self.words.is_empty() {
            let last = self.words.len() - 1;
            self.words[last] &= (1u64 << top_bits) - 1;
        }
    }
}

impl fmt::Display for BitValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "(bitvec {} \"{}\")", self.width, self.to_bin_string())
    }
}

fn word_count_for(width: u32) -> usize {
    (width as usize).div_ceil(64)
}

/// Inclusive range `[lo, hi]` of equal-width [`BitValue`]s.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BitInterval {
    pub lo: BitValue,
    pub hi: BitValue,
}

impl BitInterval {
    /// Create a validated inclusive bit interval `[lo, hi]`.
    pub fn new(lo: BitValue, hi: BitValue) -> Self {
        assert_eq!(lo.width(), hi.width(), "BitInterval widths must match");
        assert!(lo <= hi, "BitInterval requires lo <= hi");
        Self { lo, hi }
    }

    /// Singleton interval containing exactly `value`.
    pub fn point(value: BitValue) -> Self {
        Self {
            lo: value.clone(),
            hi: value,
        }
    }

    /// Full interval `[0, 2^width - 1]` covering all `width`-bit values.
    pub fn full(width: u32) -> Self {
        Self {
            lo: BitValue::zero(width),
            hi: BitValue::all_ones(width),
        }
    }

    /// Bit width of values in this interval.
    pub fn width(&self) -> u32 {
        self.lo.width()
    }

    /// Check whether `value` belongs to `[lo, hi]`.
    pub fn contains(&self, value: &BitValue) -> bool {
        self.lo <= *value && *value <= self.hi
    }

    /// Return `true` if this interval covers the entire `width`-bit domain.
    pub fn is_full(&self) -> bool {
        self.lo.is_zero() && self.hi == BitValue::all_ones(self.width())
    }
}

impl fmt::Display for BitInterval {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.lo == self.hi {
            write!(f, "(bit {})", self.lo)
        } else {
            write!(f, "(bit-range {} {})", self.lo, self.hi)
        }
    }
}

/// Partition the entire `width`-bit domain `[0, 2^width - 1]` along all interval endpoints
/// in `intervals`, returning sorted, disjoint, gapless elementary [`BitInterval`]s.
pub fn partition_bit_intervals(width: u32, intervals: &[BitInterval]) -> Vec<BitInterval> {
    let min_val = BitValue::zero(width);
    let max_val = BitValue::all_ones(width);
    if width == 0 {
        return vec![BitInterval::full(0)];
    }

    let mut starts = BTreeSet::new();
    starts.insert(min_val);
    for iv in intervals {
        debug_assert_eq!(iv.width(), width);
        starts.insert(iv.lo.clone());
        if iv.hi < max_val
            && let Some(after) = iv.hi.checked_add_one()
        {
            starts.insert(after);
        }
    }

    let ordered: Vec<BitValue> = starts.into_iter().collect();
    let mut out = Vec::with_capacity(ordered.len());
    for (idx, start) in ordered.iter().enumerate() {
        let end = if idx + 1 < ordered.len() {
            ordered[idx + 1]
                .checked_sub_one()
                .expect("strictly increasing starts guarantee sub_one succeeds")
        } else {
            max_val.clone()
        };
        out.push(BitInterval::new(start.clone(), end));
    }
    out
}

/// Split `interval` of width `W` at bit boundary `split_bit` (`0 < split_bit < W`) into
/// disjoint `(hi_interval, lo_interval)` pairs covering `[k, W)` and `[0, k)`.
///
/// Any contiguous bit interval splits across a bit boundary into at most 3 `(hi, lo)` rectangles:
/// an optional left partial slice, an optional full-low middle band, and an optional right partial slice.
pub fn split_bit_interval(
    interval: &BitInterval,
    split_bit: u32,
) -> Vec<(BitInterval, BitInterval)> {
    let total = interval.width();
    assert!(
        split_bit > 0 && split_bit < total,
        "split_bit {split_bit} must lie strictly inside (0, {total})"
    );
    let lo_width = split_bit;
    let ah = interval.lo.slice(split_bit, total);
    let al = interval.lo.slice(0, split_bit);
    let bh = interval.hi.slice(split_bit, total);
    let bl = interval.hi.slice(0, split_bit);

    if ah == bh {
        return vec![(BitInterval::point(ah), BitInterval::new(al, bl))];
    }

    let zero_lo = BitValue::zero(lo_width);
    let max_lo = BitValue::all_ones(lo_width);
    let mut out = Vec::with_capacity(3);

    let mid_start = if al > zero_lo {
        let next = ah
            .checked_add_one()
            .expect("ah < bh guarantees ah + 1 cannot overflow");
        out.push((BitInterval::point(ah), BitInterval::new(al, max_lo.clone())));
        next
    } else {
        ah
    };

    let right_partial = if bl < max_lo {
        let prev = bh
            .checked_sub_one()
            .expect("ah < bh guarantees bh - 1 cannot underflow");
        let pair = (BitInterval::point(bh), BitInterval::new(zero_lo, bl));
        (prev, Some(pair))
    } else {
        (bh, None)
    };

    if mid_start <= right_partial.0 {
        out.push((
            BitInterval::new(mid_start, right_partial.0),
            BitInterval::full(lo_width),
        ));
    }
    if let Some(right) = right_partial.1 {
        out.push(right);
    }

    out
}

/// Convert a [`BitValue`] of width `w` to a runtime [`crate::eval::Value`] of type `ty`.
pub fn slice_to_value(slice: &BitValue, ty: &TypeDesc) -> Option<crate::eval::Value> {
    match ty {
        TypeDesc::BitVec(w) if *w == slice.width() => {
            Some(crate::eval::Value::BitVec(slice.clone()))
        }
        TypeDesc::Bool if slice.width() == 1 => Some(crate::eval::Value::Bool(!slice.is_zero())),
        TypeDesc::Int(IntType::UnsignedBits(w)) if u32::from(*w) == slice.width() => {
            Some(crate::eval::Value::Int(IntLit::Unsigned(slice.to_u128()?)))
        }
        TypeDesc::Int(IntType::SignedBits(w)) if u32::from(*w) == slice.width() => Some(
            crate::eval::Value::Int(IntLit::Signed(slice.to_signed_i128()?)),
        ),
        _ => None,
    }
}

/// A single aligned elementary bit-interval column `[lo, hi)` produced by
/// [`align_bitvec_column`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct AlignedBitInterval {
    pub lo: u32,
    pub hi: u32,
    pub ty: TypeDesc,
}

/// Pattern entry on one aligned elementary interval in [`AlignedBitMatrix`].
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum AlignedBitCell {
    Wild,
    /// Union of inclusive [`BitInterval`]s on `[lo, hi)`.
    BitRanges(Vec<BitInterval>),
    /// Unsplit opaque typed subpattern on `[lo, hi)` (e.g. Enum / custom pattern).
    Opaque {
        ty: TypeDesc,
        pattern: Pattern,
    },
}

/// One aligned row produced by decomposing a `BitVec<N>` pattern (or `Or` branch).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct AlignedBitRow {
    pub source_row_idx: usize,
    pub cells: Vec<AlignedBitCell>,
    pub bindings: Vec<Binding>,
}

/// Result of aligning a column of `BitVec<N>` patterns across all active matrix rows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct AlignedBitMatrix {
    pub intervals: Vec<AlignedBitInterval>,
    pub rows: Vec<AlignedBitRow>,
}

/// Input row passed to [`align_bitvec_column`].
#[derive(Clone, Debug)]
pub(crate) enum BitColumnRow {
    Wild,
    BitVec(BitVecPattern),
}

/// Constraint on one contiguous segment `[lo, hi)` extracted from a `BitVecPattern`.
#[derive(Clone, Debug)]
enum SegConstraint {
    BitRanges(Vec<BitInterval>),
    Opaque { ty: TypeDesc, pattern: Pattern },
}

/// One alternative branch for a single row's `BitVecPattern` after flattening nested `BitVec`s
/// and `Or`s within typed segments.
#[derive(Clone, Debug)]
struct RowBitBranch {
    constraints: Vec<(u32, u32, SegConstraint)>,
    bindings: Vec<Binding>,
}

/// Align heterogeneous, overlapping `BitVec<N>` segment splits across rows into common
/// disjoint bit intervals `[lo, hi)` ordered MSB-to-LSB.
pub(crate) fn align_bitvec_column(
    width: u32,
    base_place: &Place,
    rows: &[BitColumnRow],
) -> Result<AlignedBitMatrix, CompileError> {
    if width == 0 {
        return align_zero_width_column(base_place, rows);
    }

    let mut per_row_branches: Vec<Vec<RowBitBranch>> = Vec::with_capacity(rows.len());
    for row in rows {
        match row {
            BitColumnRow::Wild => {
                per_row_branches.push(vec![RowBitBranch {
                    constraints: Vec::new(),
                    bindings: Vec::new(),
                }]);
            }
            BitColumnRow::BitVec(bv_pat) => {
                let branches = decompose_bitvec_pattern(width, 0, base_place, bv_pat)?;
                per_row_branches.push(branches);
            }
        }
    }

    // Collect all boundary points from constrained segments across all rows.
    let mut boundaries = BTreeSet::new();
    boundaries.insert(0);
    boundaries.insert(width);
    let mut opaque_spans: Vec<(u32, u32)> = Vec::new();
    for branches in &per_row_branches {
        for branch in branches {
            for &(lo, hi, ref constraint) in &branch.constraints {
                boundaries.insert(lo);
                boundaries.insert(hi);
                if matches!(constraint, SegConstraint::Opaque { .. }) {
                    opaque_spans.push((lo, hi));
                }
            }
        }
    }

    // Opaque subpatterns (like Enum inside a bit slice) cannot be factored across bit splits,
    // so remove any interior boundary strictly inside an opaque span.
    if !opaque_spans.is_empty() {
        let merged = merge_overlapping_spans(opaque_spans);
        boundaries.retain(|&b| !merged.iter().any(|&(lo, hi)| b > lo && b < hi));
    }

    let sorted_bounds: Vec<u32> = boundaries.into_iter().collect();
    // Build disjoint intervals MSB-to-LSB (descending bit offset).
    let mut raw_intervals: Vec<(u32, u32)> = Vec::new();
    for pair in sorted_bounds.windows(2).rev() {
        raw_intervals.push((pair[0], pair[1]));
    }

    // Project every branch onto the aligned intervals.
    let mut active_intervals_mask = vec![false; raw_intervals.len()];
    let mut aligned_rows: Vec<AlignedBitRow> = Vec::new();

    for (source_row_idx, branches) in per_row_branches.into_iter().enumerate() {
        for branch in branches {
            let expanded =
                project_branch_onto_intervals(&raw_intervals, &branch, &mut active_intervals_mask)?;
            for (cells, bindings) in expanded {
                aligned_rows.push(AlignedBitRow {
                    source_row_idx,
                    cells,
                    bindings,
                });
            }
        }
    }

    // Drop elementary intervals that are `Wild` in every surviving row.
    let kept_indices: Vec<usize> = active_intervals_mask
        .iter()
        .enumerate()
        .filter_map(|(i, &active)| active.then_some(i))
        .collect();

    let intervals: Vec<AlignedBitInterval> = kept_indices
        .iter()
        .map(|&i| {
            let (lo, hi) = raw_intervals[i];
            AlignedBitInterval {
                lo,
                hi,
                ty: TypeDesc::BitVec(hi - lo),
            }
        })
        .collect();

    for row in &mut aligned_rows {
        let filtered: Vec<AlignedBitCell> =
            kept_indices.iter().map(|&i| row.cells[i].clone()).collect();
        row.cells = filtered;
    }

    Ok(AlignedBitMatrix {
        intervals,
        rows: aligned_rows,
    })
}

fn align_zero_width_column(
    base_place: &Place,
    rows: &[BitColumnRow],
) -> Result<AlignedBitMatrix, CompileError> {
    let mut aligned_rows = Vec::new();
    for (source_row_idx, row) in rows.iter().enumerate() {
        match row {
            BitColumnRow::Wild => {
                aligned_rows.push(AlignedBitRow {
                    source_row_idx,
                    cells: Vec::new(),
                    bindings: Vec::new(),
                });
            }
            BitColumnRow::BitVec(bv_pat) => {
                let branches = decompose_bitvec_pattern(0, 0, base_place, bv_pat)?;
                for branch in branches {
                    aligned_rows.push(AlignedBitRow {
                        source_row_idx,
                        cells: Vec::new(),
                        bindings: branch.bindings,
                    });
                }
            }
        }
    }
    Ok(AlignedBitMatrix {
        intervals: Vec::new(),
        rows: aligned_rows,
    })
}

fn merge_overlapping_spans(mut spans: Vec<(u32, u32)>) -> Vec<(u32, u32)> {
    spans.sort_unstable();
    let mut merged: Vec<(u32, u32)> = Vec::new();
    for (lo, hi) in spans {
        if let Some(last) = merged.last_mut()
            && lo < last.1
        {
            last.1 = last.1.max(hi);
        } else {
            merged.push((lo, hi));
        }
    }
    merged
}

fn decompose_bitvec_pattern(
    expected_width: u32,
    base_offset: u32,
    base_place: &Place,
    bv_pat: &BitVecPattern,
) -> Result<Vec<RowBitBranch>, CompileError> {
    let actual_width = bv_pat.segments.iter().try_fold(0u32, |acc, seg| {
        acc.checked_add(seg.width())
            .ok_or(CompileError::InvalidBitVecWidth {
                expected: expected_width,
                found: u32::MAX,
            })
    })?;
    if actual_width != expected_width {
        return Err(CompileError::InvalidBitVecWidth {
            expected: expected_width,
            found: actual_width,
        });
    }
    if let Some(declared) = bv_pat.total_width
        && declared != expected_width
    {
        return Err(CompileError::InvalidBitVecWidth {
            expected: expected_width,
            found: declared,
        });
    }

    let mut per_segment_alts: Vec<Vec<RowBitBranch>> = Vec::with_capacity(bv_pat.segments.len());
    let mut cursor = base_offset + expected_width;

    for seg in &bv_pat.segments {
        let w = seg.width();
        let lo = cursor - w;
        let hi = cursor;
        cursor = lo;

        match seg {
            BitSegmentPattern::Fixed {
                width: seg_w,
                value,
            } => {
                if value.width() != *seg_w {
                    return Err(CompileError::InvalidBitVecWidth {
                        expected: *seg_w,
                        found: value.width(),
                    });
                }
                if *seg_w == 0 {
                    per_segment_alts.push(vec![RowBitBranch {
                        constraints: Vec::new(),
                        bindings: Vec::new(),
                    }]);
                } else {
                    per_segment_alts.push(vec![RowBitBranch {
                        constraints: vec![(
                            lo,
                            hi,
                            SegConstraint::BitRanges(vec![BitInterval::point(value.clone())]),
                        )],
                        bindings: Vec::new(),
                    }]);
                }
            }
            BitSegmentPattern::Typed {
                width: seg_w,
                ty,
                pattern,
            } => {
                validate_slice_type(*seg_w, ty)?;
                let alts = decompose_typed_segment(lo, hi, ty, base_place, pattern)?;
                per_segment_alts.push(alts);
            }
        }
    }

    // Combine segment alternatives across segments in this single row.
    // Note: top-level non-branching segments have length 1, so only segments containing
    // internal `Or` patterns fan out, and impossible segments (`is_empty`) prune immediately.
    let mut acc = vec![RowBitBranch {
        constraints: Vec::new(),
        bindings: Vec::new(),
    }];
    for seg_alts in per_segment_alts {
        if seg_alts.is_empty() {
            return Ok(Vec::new());
        }
        let mut next = Vec::with_capacity(acc.len() * seg_alts.len());
        for prefix in &acc {
            for seg in &seg_alts {
                let mut combined = prefix.clone();
                combined.constraints.extend(seg.constraints.iter().cloned());
                combined.bindings.extend(seg.bindings.iter().cloned());
                next.push(combined);
            }
        }
        acc = next;
    }

    Ok(acc)
}

fn validate_slice_type(width: u32, ty: &TypeDesc) -> Result<(), CompileError> {
    let matches = match ty {
        TypeDesc::BitVec(w) => *w == width,
        TypeDesc::Bool => width == 1,
        TypeDesc::Int(IntType::UnsignedBits(w)) | TypeDesc::Int(IntType::SignedBits(w)) => {
            u32::from(*w) == width
        }
        _ => false,
    };
    if !matches {
        return Err(CompileError::TypeMismatch {
            expected: format!("bit-slice type of width {width}"),
            found: ty.to_string(),
        });
    }
    Ok(())
}

fn decompose_typed_segment(
    lo: u32,
    hi: u32,
    ty: &TypeDesc,
    base_place: &Place,
    pattern: &Pattern,
) -> Result<Vec<RowBitBranch>, CompileError> {
    let width = hi - lo;
    match pattern {
        Pattern::Wild => Ok(vec![RowBitBranch {
            constraints: Vec::new(),
            bindings: Vec::new(),
        }]),
        Pattern::Bind { name, subpattern } => {
            let mut alts = decompose_typed_segment(lo, hi, ty, base_place, subpattern)?;
            let slice_place = base_place.bit_slice(lo, hi, ty.clone());
            for alt in &mut alts {
                alt.bindings.push(Binding {
                    name: name.clone(),
                    place: slice_place.clone(),
                });
            }
            Ok(alts)
        }
        Pattern::Or(choices) => {
            let mut all = Vec::new();
            for choice in choices {
                all.extend(decompose_typed_segment(lo, hi, ty, base_place, choice)?);
            }
            Ok(all)
        }
        Pattern::BitVec(inner) => {
            if !matches!(ty, TypeDesc::BitVec(w) if *w == width) {
                return Err(CompileError::TypeMismatch {
                    expected: ty.to_string(),
                    found: format!("BitVec<{width}>"),
                });
            }
            decompose_bitvec_pattern(width, lo, base_place, inner)
        }
        Pattern::Bool(b) => {
            if !matches!(ty, TypeDesc::Bool) {
                return Err(CompileError::TypeMismatch {
                    expected: ty.to_string(),
                    found: "bool".to_string(),
                });
            }
            Ok(vec![RowBitBranch {
                constraints: vec![(
                    lo,
                    hi,
                    SegConstraint::BitRanges(vec![BitInterval::point(BitValue::from_bool(*b))]),
                )],
                bindings: Vec::new(),
            }])
        }
        Pattern::Int(lit) => {
            let intervals = int_lit_to_bit_intervals(width, ty, *lit)?;
            if intervals.is_empty() {
                return Ok(Vec::new());
            }
            Ok(vec![RowBitBranch {
                constraints: if width == 0 {
                    Vec::new()
                } else {
                    vec![(lo, hi, SegConstraint::BitRanges(intervals))]
                },
                bindings: Vec::new(),
            }])
        }
        Pattern::IntRange { start, end } => {
            let intervals = int_range_to_bit_intervals(width, ty, *start, *end)?;
            if intervals.is_empty() {
                return Ok(Vec::new());
            }
            Ok(vec![RowBitBranch {
                constraints: if width == 0 {
                    Vec::new()
                } else {
                    vec![(lo, hi, SegConstraint::BitRanges(intervals))]
                },
                bindings: Vec::new(),
            }])
        }
        other => {
            if width == 0 {
                return Err(CompileError::TypeMismatch {
                    expected: ty.to_string(),
                    found: other.to_string(),
                });
            }
            Ok(vec![RowBitBranch {
                constraints: vec![(
                    lo,
                    hi,
                    SegConstraint::Opaque {
                        ty: ty.clone(),
                        pattern: other.clone(),
                    },
                )],
                bindings: Vec::new(),
            }])
        }
    }
}

fn int_lit_to_bit_intervals(
    width: u32,
    ty: &TypeDesc,
    lit: IntLit,
) -> Result<Vec<BitInterval>, CompileError> {
    let bound = RangeBound::Inclusive(lit);
    int_range_to_bit_intervals(width, ty, bound, bound)
}

fn int_range_to_bit_intervals(
    width: u32,
    ty: &TypeDesc,
    start: RangeBound<IntLit>,
    end: RangeBound<IntLit>,
) -> Result<Vec<BitInterval>, CompileError> {
    let int_ty = match ty {
        TypeDesc::Int(it) => *it,
        other => {
            return Err(CompileError::TypeMismatch {
                expected: other.to_string(),
                found: "int".to_string(),
            });
        }
    };
    let domain = crate::pattern::IntInterval::domain_for(int_ty);
    let Some(clamped) = crate::pattern::IntInterval::from_bounds(start, end, domain) else {
        return Ok(Vec::new());
    };
    let lo_i = clamped
        .lo_finite()
        .expect("bounded bit slice has finite lo");
    let hi_i = clamped
        .hi_finite()
        .expect("bounded bit slice has finite hi");

    match int_ty {
        IntType::UnsignedBits(_) => {
            let u_lo = lo_i.to_u128().expect("unsigned range >= 0");
            let u_hi = hi_i.to_u128().expect("unsigned range fits u128");
            let b_lo = BitValue::from_u128(width, u_lo).expect("clamped to domain");
            let b_hi = BitValue::from_u128(width, u_hi).expect("clamped to domain");
            Ok(vec![BitInterval::new(b_lo, b_hi)])
        }
        IntType::SignedBits(_) => {
            let s_lo = lo_i.to_i128().expect("signed range fits i128");
            let s_hi = hi_i.to_i128().expect("signed range fits i128");
            let mut out = Vec::new();
            // Two's complement separates negative and non-negative segments cleanly:
            // [0..=max_pos] stays low, [min_neg..=-1] sits in high half [2^(w-1)..=2^w - 1].
            if s_hi >= 0 {
                let pos_lo = s_lo.max(0);
                let b_lo = BitValue::from_i128(width, pos_lo).expect("clamped positive");
                let b_hi = BitValue::from_i128(width, s_hi).expect("clamped positive");
                out.push(BitInterval::new(b_lo, b_hi));
            }
            if s_lo < 0 {
                let neg_hi = s_hi.min(-1);
                let b_lo = BitValue::from_i128(width, s_lo).expect("clamped negative");
                let b_hi = BitValue::from_i128(width, neg_hi).expect("clamped negative");
                out.push(BitInterval::new(b_lo, b_hi));
            }
            out.sort();
            Ok(out)
        }
        IntType::OpenSigned | IntType::OpenUnsigned => unreachable!("validated bounded bit slice"),
    }
}

fn project_branch_onto_intervals(
    intervals: &[(u32, u32)],
    branch: &RowBitBranch,
    active_mask: &mut [bool],
) -> Result<Vec<(Vec<AlignedBitCell>, Vec<Binding>)>, CompileError> {
    let mut cell_options: Vec<Vec<(usize, AlignedBitCell)>> = vec![Vec::new(); intervals.len()];
    let mut current_rows: Vec<(Vec<AlignedBitCell>, Vec<Binding>)> = vec![(
        vec![AlignedBitCell::Wild; intervals.len()],
        branch.bindings.clone(),
    )];

    let _ = &mut cell_options;

    for &(seg_lo, seg_hi, ref constraint) in &branch.constraints {
        let covered: Vec<usize> = intervals
            .iter()
            .enumerate()
            .filter_map(|(idx, &(ilo, ihi))| (ilo >= seg_lo && ihi <= seg_hi).then_some(idx))
            .collect();
        debug_assert!(
            !covered.is_empty(),
            "every constraint must align to at least one elementary interval"
        );

        match constraint {
            SegConstraint::Opaque { ty, pattern } => {
                debug_assert_eq!(covered.len(), 1);
                let idx = covered[0];
                active_mask[idx] = true;
                for (cells, _) in &mut current_rows {
                    cells[idx] = AlignedBitCell::Opaque {
                        ty: ty.clone(),
                        pattern: pattern.clone(),
                    };
                }
            }
            SegConstraint::BitRanges(bit_intervals) => {
                if covered.len() == 1 {
                    let idx = covered[0];
                    let (ilo, ihi) = intervals[idx];
                    debug_assert_eq!((ilo, ihi), (seg_lo, seg_hi));
                    active_mask[idx] = true;
                    let merged = merge_bit_intervals(bit_intervals);
                    for (cells, _) in &mut current_rows {
                        cells[idx] = AlignedBitCell::BitRanges(merged.clone());
                    }
                } else {
                    // `covered` is ordered MSB-to-LSB (descending bit offset).
                    // Recursively split each `BitInterval` across the covered elementary boundaries.
                    let mut seg_combos: Vec<Vec<BitInterval>> = Vec::new();
                    for biv in bit_intervals {
                        let splits = split_across_intervals(biv, seg_lo, &covered, intervals);
                        seg_combos.extend(splits);
                    }
                    for &idx in &covered {
                        active_mask[idx] = true;
                    }
                    let mut next_rows = Vec::with_capacity(current_rows.len() * seg_combos.len());
                    for (base_cells, base_bindings) in &current_rows {
                        for combo in &seg_combos {
                            let mut cloned = base_cells.clone();
                            for (pos, &col_idx) in covered.iter().enumerate() {
                                if !combo[pos].is_full() {
                                    cloned[col_idx] =
                                        AlignedBitCell::BitRanges(vec![combo[pos].clone()]);
                                }
                            }
                            next_rows.push((cloned, base_bindings.clone()));
                        }
                    }
                    current_rows = next_rows;
                }
            }
        }
    }

    Ok(current_rows)
}

fn split_across_intervals(
    biv: &BitInterval,
    seg_lo: u32,
    covered_msb_to_lsb: &[usize],
    intervals: &[(u32, u32)],
) -> Vec<Vec<BitInterval>> {
    if covered_msb_to_lsb.len() == 1 {
        return vec![vec![biv.clone()]];
    }
    let (_, first_hi) = intervals[covered_msb_to_lsb[0]];
    let (first_lo, _) = intervals[covered_msb_to_lsb[0]];
    debug_assert_eq!(first_hi - seg_lo, biv.width());
    let local_split = first_lo - seg_lo;
    let pairs = split_bit_interval(biv, local_split);
    let mut out = Vec::new();
    for (hi_iv, lo_iv) in pairs {
        let tails = split_across_intervals(&lo_iv, seg_lo, &covered_msb_to_lsb[1..], intervals);
        for mut tail in tails {
            let mut full = Vec::with_capacity(1 + tail.len());
            full.push(hi_iv.clone());
            full.append(&mut tail);
            out.push(full);
        }
    }
    out
}

/// Normalize and coalesce overlapping or adjacent [`BitInterval`]s of equal width.
pub fn merge_bit_intervals(intervals: &[BitInterval]) -> Vec<BitInterval> {
    if intervals.is_empty() {
        return Vec::new();
    }
    let mut sorted = intervals.to_vec();
    sorted.sort();
    let max_val = BitValue::all_ones(sorted[0].width());
    let mut out: Vec<BitInterval> = Vec::with_capacity(sorted.len());
    for iv in sorted {
        if let Some(last) = out.last_mut() {
            let adjacent =
                last.hi < max_val && last.hi.checked_add_one().is_some_and(|next| next >= iv.lo);
            if iv.lo <= last.hi || adjacent {
                if iv.hi > last.hi {
                    last.hi = iv.hi;
                }
                continue;
            }
        }
        out.push(iv);
    }
    out
}

/// Find the smallest [`BitValue`] of width `width` NOT contained in any of `covered`,
/// returning `None` if `covered` covers `[0, 2^width - 1]`.
pub fn first_uncovered_bit_value(width: u32, covered: &[BitInterval]) -> Option<BitValue> {
    let merged = merge_bit_intervals(covered);
    let zero = BitValue::zero(width);
    let max = BitValue::all_ones(width);
    let mut cursor = zero;
    for iv in merged {
        if cursor < iv.lo {
            return Some(cursor);
        }
        if iv.hi == max {
            return None;
        }
        let next = iv.hi.checked_add_one()?;
        if next > cursor {
            cursor = next;
        }
    }
    Some(cursor)
}
