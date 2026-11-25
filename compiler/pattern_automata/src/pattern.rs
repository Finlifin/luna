//! Crate-independent pattern matrix representation, domain metadata, and validation.

use crate::bitvec::BitValue;
use crate::sexpr;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

/// Integer literal supporting full signed 128-bit and unsigned 128-bit values.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum IntLit {
    Signed(i128),
    Unsigned(u128),
}

impl IntLit {
    /// Convert to exact signed 192-bit representation [`FiniteInt`].
    pub fn to_finite(self) -> FiniteInt {
        match self {
            Self::Signed(v) => FiniteInt::from_i128(v),
            Self::Unsigned(v) => FiniteInt::from_u128(v),
        }
    }
}

impl fmt::Display for IntLit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Signed(v) => write!(f, "(int {v})"),
            Self::Unsigned(v) => write!(f, "(uint {v})"),
        }
    }
}

/// Exact signed 192-bit two's complement integer `(high: i64, low: u128)`.
///
/// Unifies `i128` and `u128` endpoints plus `+/- 1` adjustments without overflow.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FiniteInt {
    pub high: i64,
    pub low: u128,
}

impl FiniteInt {
    /// Zero in 192-bit representation.
    pub const ZERO: Self = Self { high: 0, low: 0 };

    /// Construct from signed `i128`.
    pub const fn from_i128(value: i128) -> Self {
        Self {
            high: if value < 0 { -1 } else { 0 },
            low: value as u128,
        }
    }

    /// Construct from unsigned `u128`.
    pub const fn from_u128(value: u128) -> Self {
        Self {
            high: 0,
            low: value,
        }
    }

    /// Minimum signed value for `bits` (`1 <= bits <= 128`).
    pub const fn min_signed(bits: u8) -> Self {
        assert!(bits >= 1 && bits <= 128);
        if bits == 128 {
            Self::from_i128(i128::MIN)
        } else {
            Self::from_i128(-(1i128 << (bits - 1)))
        }
    }

    /// Maximum signed value for `bits` (`1 <= bits <= 128`).
    pub const fn max_signed(bits: u8) -> Self {
        assert!(bits >= 1 && bits <= 128);
        if bits == 128 {
            Self::from_i128(i128::MAX)
        } else {
            Self::from_i128((1i128 << (bits - 1)) - 1)
        }
    }

    /// Maximum unsigned value for `bits` (`1 <= bits <= 128`).
    pub const fn max_unsigned(bits: u8) -> Self {
        assert!(bits >= 1 && bits <= 128);
        if bits == 128 {
            Self::from_u128(u128::MAX)
        } else {
            Self::from_u128((1u128 << bits) - 1)
        }
    }

    /// Return `self + 1`.
    pub fn add_one(self) -> Self {
        let (low, carry) = self.low.overflowing_add(1);
        let high = self
            .high
            .checked_add(i64::from(carry))
            .expect("FiniteInt add_one overflowed 192 bits");
        Self { high, low }
    }

    /// Return `self - 1`.
    pub fn sub_one(self) -> Self {
        let (low, borrow) = self.low.overflowing_sub(1);
        let high = self
            .high
            .checked_sub(i64::from(borrow))
            .expect("FiniteInt sub_one underflowed 192 bits");
        Self { high, low }
    }

    /// Convert to `i128` if representable.
    pub fn to_i128(self) -> Option<i128> {
        if self.high == 0 && (self.low as i128) >= 0 {
            Some(self.low as i128)
        } else if self.high == -1 && (self.low as i128) < 0 {
            Some(self.low as i128)
        } else {
            None
        }
    }

    /// Convert to `u128` if representable.
    pub fn to_u128(self) -> Option<u128> {
        if self.high == 0 { Some(self.low) } else { None }
    }

    /// Convert back to [`IntLit`], preferring `Signed` when `prefer_signed` is set
    /// and the value fits in `i128`. Returns `None` if `self` exceeds the 128-bit range.
    pub fn to_int_lit(self, prefer_signed: bool) -> Option<IntLit> {
        if prefer_signed {
            if let Some(s) = self.to_i128() {
                return Some(IntLit::Signed(s));
            }
            if let Some(u) = self.to_u128() {
                return Some(IntLit::Unsigned(u));
            }
        } else {
            if let Some(u) = self.to_u128() {
                return Some(IntLit::Unsigned(u));
            }
            if let Some(s) = self.to_i128() {
                return Some(IntLit::Signed(s));
            }
        }
        None
    }
}

impl fmt::Display for FiniteInt {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(u) = self.to_u128() {
            return write!(f, "{u}");
        }
        if let Some(s) = self.to_i128() {
            return write!(f, "{s}");
        }
        let negative = self.high < 0;
        let (mag_high, mag_low) = if negative {
            let inv_low = !self.low;
            let inv_high = (!self.high) as u64;
            let (low, carry) = inv_low.overflowing_add(1);
            let high = inv_high.wrapping_add(u64::from(carry));
            (high, low)
        } else {
            (self.high as u64, self.low)
        };
        let mut limbs = [mag_low as u64, (mag_low >> 64) as u64, mag_high];
        let mut digits = Vec::new();
        while limbs != [0, 0, 0] {
            let mut rem = 0u128;
            for limb in limbs.iter_mut().rev() {
                let cur = (rem << 64) | (*limb as u128);
                *limb = (cur / 10) as u64;
                rem = cur % 10;
            }
            digits.push(char::from(b'0' + (rem as u8)));
        }
        if digits.is_empty() {
            digits.push('0');
        }
        if negative {
            f.write_str("-")?;
        }
        for ch in digits.into_iter().rev() {
            write!(f, "{ch}")?;
        }
        Ok(())
    }
}

/// Lower bound point for integer interval partitioning (`NegInf < Finite(_)`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum LowerPoint {
    NegInf,
    Finite(FiniteInt),
}

/// Upper bound point for integer interval partitioning (`Finite(_) < PosInf`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum UpperPoint {
    Finite(FiniteInt),
    PosInf,
}

/// Closed or half-unbounded discrete integer interval `[lo, hi]`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct IntInterval {
    pub lo: LowerPoint,
    pub hi: UpperPoint,
}

impl IntInterval {
    /// Full integer domain interval for [`IntType`].
    pub fn domain_for(ty: IntType) -> Self {
        match ty {
            IntType::SignedBits(bits) => Self {
                lo: LowerPoint::Finite(FiniteInt::min_signed(bits)),
                hi: UpperPoint::Finite(FiniteInt::max_signed(bits)),
            },
            IntType::UnsignedBits(bits) => Self {
                lo: LowerPoint::Finite(FiniteInt::ZERO),
                hi: UpperPoint::Finite(FiniteInt::max_unsigned(bits)),
            },
            IntType::OpenSigned => Self {
                lo: LowerPoint::NegInf,
                hi: UpperPoint::PosInf,
            },
            IntType::OpenUnsigned => Self {
                lo: LowerPoint::Finite(FiniteInt::ZERO),
                hi: UpperPoint::PosInf,
            },
        }
    }

    /// Normalize `(start, end)` bounds into `[lo, hi]` clamped to `domain`, returning `None`
    /// if the resulting interval is empty.
    pub fn from_bounds(
        start: RangeBound<IntLit>,
        end: RangeBound<IntLit>,
        domain: Self,
    ) -> Option<Self> {
        let lo = match start {
            RangeBound::Unbounded => LowerPoint::NegInf,
            RangeBound::Inclusive(v) => LowerPoint::Finite(v.to_finite()),
            RangeBound::Exclusive(v) => LowerPoint::Finite(v.to_finite().add_one()),
        };
        let hi = match end {
            RangeBound::Unbounded => UpperPoint::PosInf,
            RangeBound::Inclusive(v) => UpperPoint::Finite(v.to_finite()),
            RangeBound::Exclusive(v) => UpperPoint::Finite(v.to_finite().sub_one()),
        };
        let clamped_lo = lo.max(domain.lo);
        let clamped_hi = hi.min(domain.hi);
        if is_non_empty_interval(clamped_lo, clamped_hi) {
            Some(Self {
                lo: clamped_lo,
                hi: clamped_hi,
            })
        } else {
            None
        }
    }

    /// Return the finite lower bound if bounded below.
    pub fn lo_finite(&self) -> Option<FiniteInt> {
        match self.lo {
            LowerPoint::NegInf => None,
            LowerPoint::Finite(v) => Some(v),
        }
    }

    /// Return the finite upper bound if bounded above.
    pub fn hi_finite(&self) -> Option<FiniteInt> {
        match self.hi {
            UpperPoint::Finite(v) => Some(v),
            UpperPoint::PosInf => None,
        }
    }

    /// Check whether `lit` lies inside `[lo, hi]`.
    pub fn contains(&self, lit: IntLit) -> bool {
        self.contains_finite(lit.to_finite())
    }

    /// Check whether `val` lies inside `[lo, hi]`.
    pub fn contains_finite(&self, val: FiniteInt) -> bool {
        let ge_lo = match self.lo {
            LowerPoint::NegInf => true,
            LowerPoint::Finite(v) => val >= v,
        };
        let le_hi = match self.hi {
            UpperPoint::Finite(v) => val <= v,
            UpperPoint::PosInf => true,
        };
        ge_lo && le_hi
    }

    /// Choose a canonical representative value inside this non-empty interval.
    pub fn representative(&self, prefer_signed: bool) -> IntLit {
        if self.contains_finite(FiniteInt::ZERO) {
            return FiniteInt::ZERO
                .to_int_lit(prefer_signed)
                .unwrap_or(IntLit::Signed(0));
        }
        match (self.lo, self.hi) {
            (LowerPoint::Finite(lo), _) => {
                lo.to_int_lit(prefer_signed).unwrap_or(IntLit::Signed(0))
            }
            (LowerPoint::NegInf, UpperPoint::Finite(hi)) => {
                hi.to_int_lit(prefer_signed).unwrap_or(IntLit::Signed(-1))
            }
            (LowerPoint::NegInf, UpperPoint::PosInf) => FiniteInt::ZERO
                .to_int_lit(prefer_signed)
                .unwrap_or(IntLit::Signed(0)),
        }
    }

    /// Partition `domain` into disjoint, sorted, gapless elementary intervals along all endpoints
    /// appearing in `intervals`.
    pub fn partition(domain: Self, intervals: &[Self]) -> Vec<Self> {
        let mut starts = BTreeSet::new();
        starts.insert(domain.lo);
        for iv in intervals {
            let clamped_lo = iv.lo.max(domain.lo);
            let clamped_hi = iv.hi.min(domain.hi);
            if !is_non_empty_interval(clamped_lo, clamped_hi) {
                continue;
            }
            starts.insert(clamped_lo);
            if let UpperPoint::Finite(hi_val) = clamped_hi
                && UpperPoint::Finite(hi_val) < domain.hi
            {
                starts.insert(LowerPoint::Finite(hi_val.add_one()));
            }
        }

        let ordered: Vec<LowerPoint> = starts.into_iter().collect();
        let mut out = Vec::with_capacity(ordered.len());
        for (idx, &start) in ordered.iter().enumerate() {
            let end = if idx + 1 < ordered.len() {
                match ordered[idx + 1] {
                    LowerPoint::Finite(next_lo) => UpperPoint::Finite(next_lo.sub_one()),
                    LowerPoint::NegInf => unreachable!("NegInf cannot follow another LowerPoint"),
                }
            } else {
                domain.hi
            };
            if is_non_empty_interval(start, end) {
                out.push(Self { lo: start, hi: end });
            }
        }
        out
    }

    /// Merge overlapping or adjacent [`IntInterval`]s.
    pub fn merge(intervals: &[Self]) -> Vec<Self> {
        if intervals.is_empty() {
            return Vec::new();
        }
        let mut sorted = intervals.to_vec();
        sorted.sort();
        let mut out: Vec<Self> = Vec::with_capacity(sorted.len());
        for iv in sorted {
            if let Some(last) = out.last_mut() {
                let touches = match (last.hi, iv.lo) {
                    (UpperPoint::PosInf, _) | (_, LowerPoint::NegInf) => true,
                    (UpperPoint::Finite(hi), LowerPoint::Finite(lo)) => hi.add_one() >= lo,
                };
                if touches {
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

    /// Find the smallest integer inside `domain` NOT covered by `covered`.
    pub fn first_uncovered(domain: Self, covered: &[Self], prefer_signed: bool) -> Option<IntLit> {
        let merged = Self::merge(covered);
        let mut cursor = domain.lo;
        for iv in merged {
            if !is_non_empty_interval(cursor, domain.hi) {
                return None;
            }
            if iv.lo > cursor {
                return Some(
                    Self {
                        lo: cursor,
                        hi: domain.hi,
                    }
                    .representative(prefer_signed),
                );
            }
            match iv.hi {
                UpperPoint::PosInf => return None,
                UpperPoint::Finite(hi_val) => {
                    if UpperPoint::Finite(hi_val) >= domain.hi {
                        return None;
                    }
                    let next = LowerPoint::Finite(hi_val.add_one());
                    if next > cursor {
                        cursor = next;
                    }
                }
            }
        }
        if is_non_empty_interval(cursor, domain.hi) {
            Some(
                Self {
                    lo: cursor,
                    hi: domain.hi,
                }
                .representative(prefer_signed),
            )
        } else {
            None
        }
    }
}

fn is_non_empty_interval(lo: LowerPoint, hi: UpperPoint) -> bool {
    match (lo, hi) {
        (LowerPoint::NegInf, _) | (_, UpperPoint::PosInf) => true,
        (LowerPoint::Finite(a), UpperPoint::Finite(b)) => a <= b,
    }
}

impl fmt::Display for IntInterval {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match (self.lo, self.hi) {
            (LowerPoint::Finite(a), UpperPoint::Finite(b)) if a == b => {
                write!(f, "(int-point {a})")
            }
            (lo, hi) => {
                f.write_str("(int-range ")?;
                match lo {
                    LowerPoint::NegInf => f.write_str("-inf")?,
                    LowerPoint::Finite(a) => write!(f, "{a}")?,
                }
                f.write_str(" ")?;
                match hi {
                    UpperPoint::Finite(b) => write!(f, "{b}")?,
                    UpperPoint::PosInf => f.write_str("+inf")?,
                }
                f.write_str(")")
            }
        }
    }
}

/// Closed Unicode scalar interval `[lo, hi]` (always excluding surrogates `0xD800..=0xDFFF`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CharInterval {
    pub lo: char,
    pub hi: char,
}

impl CharInterval {
    /// The two valid Unicode scalar value bands.
    pub const fn unicode_domains() -> [Self; 2] {
        [
            Self {
                lo: '\u{0}',
                hi: '\u{D7FF}',
            },
            Self {
                lo: '\u{E000}',
                hi: '\u{10FFFF}',
            },
        ]
    }

    /// Normalize `(start, end)` character bounds into 0, 1, or 2 surrogate-free intervals.
    pub fn from_bounds(start: RangeBound<char>, end: RangeBound<char>) -> Vec<Self> {
        let raw_lo: u32 = match start {
            RangeBound::Unbounded => 0,
            RangeBound::Inclusive(c) => c as u32,
            RangeBound::Exclusive(c) => (c as u32).saturating_add(1),
        };
        let raw_hi: i64 = match end {
            RangeBound::Unbounded => 0x10FFFF,
            RangeBound::Inclusive(c) => c as i64,
            RangeBound::Exclusive(c) => (c as i64) - 1,
        };
        if (raw_lo as i64) > raw_hi {
            return Vec::new();
        }
        let mut out = Vec::with_capacity(2);
        for dom in Self::unicode_domains() {
            let d_lo = dom.lo as u32;
            let d_hi = dom.hi as u32;
            let lo = raw_lo.max(d_lo);
            let hi = (raw_hi as u32).min(d_hi);
            if lo <= hi
                && let (Some(c_lo), Some(c_hi)) = (char::from_u32(lo), char::from_u32(hi))
            {
                out.push(Self { lo: c_lo, hi: c_hi });
            }
        }
        out
    }

    /// Check whether `ch` lies inside `[lo, hi]`.
    pub fn contains(&self, ch: char) -> bool {
        self.lo <= ch && ch <= self.hi
    }

    /// Partition both Unicode scalar bands along all endpoints appearing in `intervals`.
    pub fn partition_all(intervals: &[Self]) -> Vec<Self> {
        let mut out = Vec::new();
        for dom in Self::unicode_domains() {
            let d_lo = dom.lo as u32;
            let d_hi = dom.hi as u32;
            let mut starts = BTreeSet::new();
            starts.insert(d_lo);
            for iv in intervals {
                let lo = (iv.lo as u32).max(d_lo);
                let hi = (iv.hi as u32).min(d_hi);
                if lo <= hi {
                    starts.insert(lo);
                    if hi < d_hi {
                        starts.insert(hi + 1);
                    }
                }
            }
            let ordered: Vec<u32> = starts.into_iter().collect();
            for (idx, &start) in ordered.iter().enumerate() {
                let end = if idx + 1 < ordered.len() {
                    ordered[idx + 1] - 1
                } else {
                    d_hi
                };
                out.push(Self {
                    lo: char::from_u32(start).expect("valid Unicode scalar in domain"),
                    hi: char::from_u32(end).expect("valid Unicode scalar in domain"),
                });
            }
        }
        out
    }

    /// Find the first valid Unicode scalar value not covered by `covered`.
    pub fn first_uncovered(covered: &[Self]) -> Option<char> {
        let mut sorted = covered.to_vec();
        sorted.sort();
        for dom in Self::unicode_domains() {
            let mut cursor = dom.lo as u32;
            let d_hi = dom.hi as u32;
            for iv in &sorted {
                let lo = (iv.lo as u32).max(dom.lo as u32);
                let hi = (iv.hi as u32).min(d_hi);
                if lo > hi {
                    continue;
                }
                if lo > cursor {
                    return char::from_u32(cursor);
                }
                if hi >= d_hi {
                    cursor = d_hi + 1;
                    break;
                }
                if hi + 1 > cursor {
                    cursor = hi + 1;
                }
            }
            if cursor <= d_hi {
                return char::from_u32(cursor);
            }
        }
        None
    }
}

impl fmt::Display for CharInterval {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.lo == self.hi {
            f.write_str("(char ")?;
            sexpr::write_quoted_char(f, self.lo)?;
            f.write_str(")")
        } else {
            f.write_str("(char-range ")?;
            sexpr::write_quoted_char(f, self.lo)?;
            f.write_str(" ")?;
            sexpr::write_quoted_char(f, self.hi)?;
            f.write_str(")")
        }
    }
}

/// Inclusive range bound for patterns.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum RangeBound<T> {
    Unbounded,
    Inclusive(T),
    Exclusive(T),
}

impl<T: fmt::Display> fmt::Display for RangeBound<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unbounded => f.write_str("unbounded"),
            Self::Inclusive(v) => write!(f, "(inclusive {v})"),
            Self::Exclusive(v) => write!(f, "(exclusive {v})"),
        }
    }
}

/// Integer type domain metadata.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum IntType {
    SignedBits(u8),
    UnsignedBits(u8),
    OpenSigned,
    OpenUnsigned,
}

impl IntType {
    /// Whether this integer domain includes negative values.
    pub fn is_signed(self) -> bool {
        matches!(self, Self::SignedBits(_) | Self::OpenSigned)
    }
}

impl fmt::Display for IntType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SignedBits(bits) => write!(f, "(signed-bits {bits})"),
            Self::UnsignedBits(bits) => write!(f, "(unsigned-bits {bits})"),
            Self::OpenSigned => f.write_str("(open-signed-int)"),
            Self::OpenUnsigned => f.write_str("(open-unsigned-int)"),
        }
    }
}

/// Identifier of an error tag inside `!E T`.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ErrorTag(pub String);

impl ErrorTag {
    /// Construct a new error tag.
    pub fn new(name: impl Into<String>) -> Self {
        Self(name.into())
    }
}

impl fmt::Display for ErrorTag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        sexpr::write_quoted_str(f, &self.0)
    }
}

/// Type/domain metadata for a native error row `!E T`.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ErrorRowType {
    pub ok_type: Box<TypeDesc>,
    pub error_tags: BTreeMap<ErrorTag, TypeDesc>,
    pub is_open: bool,
}

impl fmt::Display for ErrorRowType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(error-row-type (open {}) (ok {}) (errors",
            self.is_open, self.ok_type
        )?;
        for (tag, payload) in &self.error_tags {
            write!(f, " ({tag} {payload})")?;
        }
        f.write_str("))")
    }
}

/// Identifier of an enum variant (named or discrete integer tag).
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum VariantTag {
    Named(String),
    Discrete(i128),
}

impl VariantTag {
    /// Construct a named variant tag.
    pub fn named(name: impl Into<String>) -> Self {
        Self::Named(name.into())
    }

    /// Construct a discrete tag.
    pub fn discrete(code: i128) -> Self {
        Self::Discrete(code)
    }
}

impl fmt::Display for VariantTag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Named(name) => {
                f.write_str("(named ")?;
                sexpr::write_quoted_str(f, name)?;
                f.write_str(")")
            }
            Self::Discrete(code) => write!(f, "(discrete {code})"),
        }
    }
}

/// Payload schema for a single enum variant.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum VariantPayloadType {
    Unit,
    Tuple(Vec<TypeDesc>),
    Record(BTreeMap<String, TypeDesc>),
}

impl fmt::Display for VariantPayloadType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unit => f.write_str("unit"),
            Self::Tuple(fields) => {
                f.write_str("(tuple")?;
                for ty in fields {
                    write!(f, " {ty}")?;
                }
                f.write_str(")")
            }
            Self::Record(fields) => {
                f.write_str("(record")?;
                for (name, ty) in fields {
                    f.write_str(" (")?;
                    sexpr::write_quoted_str(f, name)?;
                    write!(f, " {ty})")?;
                }
                f.write_str(")")
            }
        }
    }
}

/// Schema of a single enum variant, including optional discrete representation code.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct VariantSchema {
    pub discrete_code: Option<i128>,
    pub payload: VariantPayloadType,
}

impl fmt::Display for VariantSchema {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("(variant-schema")?;
        if let Some(code) = self.discrete_code {
            write!(f, " (code {code})")?;
        }
        write!(f, " (payload {}))", self.payload)
    }
}

/// Enum type universe metadata (closed or open).
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct EnumType {
    pub name: String,
    pub variants: BTreeMap<VariantTag, VariantSchema>,
    pub is_open: bool,
}

impl EnumType {
    /// Construct a closed enum with deterministic ordered variants.
    pub fn closed(name: impl Into<String>, variants: BTreeMap<VariantTag, VariantSchema>) -> Self {
        Self {
            name: name.into(),
            variants,
            is_open: false,
        }
    }

    /// Construct an open enum / discrete tag universe.
    pub fn open(name: impl Into<String>, variants: BTreeMap<VariantTag, VariantSchema>) -> Self {
        Self {
            name: name.into(),
            variants,
            is_open: true,
        }
    }
}

impl fmt::Display for EnumType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("(enum-type ")?;
        sexpr::write_quoted_str(f, &self.name)?;
        write!(f, " (open {}) (variants", self.is_open)?;
        for (tag, schema) in &self.variants {
            write!(f, " ({tag} {schema})")?;
        }
        f.write_str("))")
    }
}

/// Scrutinee or sub-place domain descriptor used for exact specialization and exhaustiveness.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum TypeDesc {
    Bool,
    Char,
    Int(IntType),
    Optional(Box<TypeDesc>),
    ErrorRow(ErrorRowType),
    Enum(EnumType),
    Tuple(Vec<TypeDesc>),
    Record(BTreeMap<String, TypeDesc>),
    List(Box<TypeDesc>),
    BitVec(u32),
    Any,
}

impl fmt::Display for TypeDesc {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Bool => f.write_str("bool"),
            Self::Char => f.write_str("char"),
            Self::Int(it) => write!(f, "{it}"),
            Self::Optional(inner) => write!(f, "(optional {inner})"),
            Self::ErrorRow(row) => write!(f, "{row}"),
            Self::Enum(en) => write!(f, "{en}"),
            Self::Tuple(elems) => {
                f.write_str("(tuple")?;
                for ty in elems {
                    write!(f, " {ty}")?;
                }
                f.write_str(")")
            }
            Self::Record(fields) => {
                f.write_str("(record")?;
                for (name, ty) in fields {
                    f.write_str(" (")?;
                    sexpr::write_quoted_str(f, name)?;
                    write!(f, " {ty})")?;
                }
                f.write_str(")")
            }
            Self::List(elem) => write!(f, "(list {elem})"),
            Self::BitVec(width) => write!(f, "(bitvec {width})"),
            Self::Any => f.write_str("any"),
        }
    }
}

/// One segment inside a `BitVec<N>` pattern, ordered MSB-to-LSB.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum BitSegmentPattern {
    /// Fixed literal bits (e.g. `0b101`, `0xAF`, `<comptime_expr>`).
    Fixed { width: u32, value: BitValue },
    /// Typed slice `(pattern : ty)` of exact bit width `width`.
    Typed {
        width: u32,
        ty: TypeDesc,
        pattern: Box<Pattern>,
    },
}

impl BitSegmentPattern {
    /// Construct a fixed-bit segment from an existing [`BitValue`].
    pub fn fixed(value: BitValue) -> Self {
        Self::Fixed {
            width: value.width(),
            value,
        }
    }

    /// Construct a fixed-bit segment from radix (`2`, `8`, `16`) digits.
    pub fn from_radix_str(radix: u32, digits: &str) -> Result<Self, crate::bitvec::BitVecError> {
        let value = BitValue::from_radix_str(radix, digits)?;
        Ok(Self::fixed(value))
    }

    /// Construct a typed unsigned integer segment `(pattern : u<width>)`.
    pub fn typed_u(bits: u8, pattern: Pattern) -> Self {
        Self::Typed {
            width: u32::from(bits),
            ty: TypeDesc::Int(IntType::UnsignedBits(bits)),
            pattern: Box::new(pattern),
        }
    }

    /// Construct a typed signed integer segment `(pattern : i<width>)`.
    pub fn typed_i(bits: u8, pattern: Pattern) -> Self {
        Self::Typed {
            width: u32::from(bits),
            ty: TypeDesc::Int(IntType::SignedBits(bits)),
            pattern: Box::new(pattern),
        }
    }

    /// Construct a typed `BitVec<width>` slice segment `(pattern : BitVec<width>)`.
    pub fn typed_bitvec(width: u32, pattern: Pattern) -> Self {
        Self::Typed {
            width,
            ty: TypeDesc::BitVec(width),
            pattern: Box::new(pattern),
        }
    }

    /// Construct a typed `bool` 1-bit slice segment `(pattern : bool)`.
    pub fn typed_bool(pattern: Pattern) -> Self {
        Self::Typed {
            width: 1,
            ty: TypeDesc::Bool,
            pattern: Box::new(pattern),
        }
    }

    /// Bit width consumed by this segment.
    pub fn width(&self) -> u32 {
        match self {
            Self::Fixed { width, .. } | Self::Typed { width, .. } => *width,
        }
    }
}

impl fmt::Display for BitSegmentPattern {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Fixed { width, value } => write!(f, "(fixed {width} {value})"),
            Self::Typed { width, ty, pattern } => {
                write!(f, "(typed {width} {ty} {pattern})")
            }
        }
    }
}

/// Bit-vector pattern `0b/0o/0x segment+` written MSB-to-LSB.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BitVecPattern {
    pub total_width: Option<u32>,
    pub segments: Vec<BitSegmentPattern>,
}

impl BitVecPattern {
    /// Create a bit-vector pattern from segments written MSB-to-LSB.
    pub fn new(segments: Vec<BitSegmentPattern>) -> Self {
        Self {
            total_width: None,
            segments,
        }
    }

    /// Create a bit-vector pattern asserting an explicit total width.
    pub fn with_width(total_width: u32, segments: Vec<BitSegmentPattern>) -> Self {
        Self {
            total_width: Some(total_width),
            segments,
        }
    }

    /// Sum of segment widths.
    pub fn computed_width(&self) -> u32 {
        self.segments.iter().map(BitSegmentPattern::width).sum()
    }
}

impl fmt::Display for BitVecPattern {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("(bitvec-pat")?;
        if let Some(w) = self.total_width {
            write!(f, " (width {w})")?;
        }
        for seg in &self.segments {
            write!(f, " {seg}")?;
        }
        f.write_str(")")
    }
}

/// Payload pattern for an enum variant.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum VariantPayloadPattern {
    Unit,
    Tuple(Vec<Pattern>),
    Record(BTreeMap<String, Pattern>),
}

impl fmt::Display for VariantPayloadPattern {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unit => f.write_str("unit"),
            Self::Tuple(pats) => {
                f.write_str("(tuple")?;
                for pat in pats {
                    write!(f, " {pat}")?;
                }
                f.write_str(")")
            }
            Self::Record(fields) => {
                f.write_str("(record")?;
                for (name, pat) in fields {
                    f.write_str(" (")?;
                    sexpr::write_quoted_str(f, name)?;
                    write!(f, " {pat})")?;
                }
                f.write_str(")")
            }
        }
    }
}

/// Pattern AST node covering all Flurry runtime pattern matching constructs.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Pattern {
    /// Wildcard `_`.
    Wild,
    /// Binding `name` (`subpattern == Wild`) or `subpattern as name`.
    Bind {
        name: String,
        subpattern: Box<Pattern>,
    },
    /// Boolean literal.
    Bool(bool),
    /// Character literal.
    Char(char),
    /// Closed/half-open character range.
    CharRange {
        start: RangeBound<char>,
        end: RangeBound<char>,
    },
    /// Signed or unsigned integer literal.
    Int(IntLit),
    /// Closed/half-open integer range (`a..b`, `a..=b`, `a..`, `..b`, `..=b`).
    IntRange {
        start: RangeBound<IntLit>,
        end: RangeBound<IntLit>,
    },
    /// Native optional `?T` absent branch (`null`).
    OptionalNull,
    /// Native optional `?T` present branch (`inner?`).
    OptionalPresent(Box<Pattern>),
    /// Native error row `!E T` success branch (`inner!`).
    ErrorOk(Box<Pattern>),
    /// Native error row `!E T` error branch (`error inner` or `error Tag(inner)`).
    ErrorErr {
        tag: Option<ErrorTag>,
        inner: Box<Pattern>,
    },
    /// Enum variant `.tag`, `.tag(p1, p2)`, or `.tag { f1: p1 }`.
    EnumVariant {
        tag: VariantTag,
        payload: VariantPayloadPattern,
    },
    /// Fixed-arity tuple pattern `(p0, p1, ...)`.
    Tuple(Vec<Pattern>),
    /// Record/struct field pattern `{ f0: p0, f1: p1 }` (omitted fields act as `_`).
    Record(BTreeMap<String, Pattern>),
    /// List/slice pattern `[p0, p1]` (`rest = None`) or `[p0, ...rest]` (`rest = Some(...)`).
    List {
        prefix: Vec<Pattern>,
        rest: Option<Box<Pattern>>,
    },
    /// `BitVec<N>` segmented pattern.
    BitVec(BitVecPattern),
    /// Alternative `p1 or p2 or ...` pattern (branches must share identical binder sets).
    Or(Vec<Pattern>),
}

impl Pattern {
    /// Construct a bare binder `name` (`Bind { name, subpattern: Wild }`).
    pub fn var(name: impl Into<String>) -> Self {
        Self::Bind {
            name: name.into(),
            subpattern: Box::new(Self::Wild),
        }
    }

    /// Construct `subpattern as name`.
    pub fn bind_as(name: impl Into<String>, subpattern: Pattern) -> Self {
        Self::Bind {
            name: name.into(),
            subpattern: Box::new(subpattern),
        }
    }

    /// Construct a signed integer literal pattern.
    pub fn signed(value: i128) -> Self {
        Self::Int(IntLit::Signed(value))
    }

    /// Construct an unsigned integer literal pattern.
    pub fn unsigned(value: u128) -> Self {
        Self::Int(IntLit::Unsigned(value))
    }

    /// Construct `?inner`.
    pub fn present(inner: Pattern) -> Self {
        Self::OptionalPresent(Box::new(inner))
    }

    /// Construct `inner!`.
    pub fn ok(inner: Pattern) -> Self {
        Self::ErrorOk(Box::new(inner))
    }

    /// Construct `error Tag(inner)`.
    pub fn err_tag(tag: impl Into<String>, inner: Pattern) -> Self {
        Self::ErrorErr {
            tag: Some(ErrorTag::new(tag)),
            inner: Box::new(inner),
        }
    }

    /// Construct `error inner` matching any error tag.
    pub fn err_any(inner: Pattern) -> Self {
        Self::ErrorErr {
            tag: None,
            inner: Box::new(inner),
        }
    }

    /// Construct a unit enum variant `.tag`.
    pub fn unit_variant(tag: VariantTag) -> Self {
        Self::EnumVariant {
            tag,
            payload: VariantPayloadPattern::Unit,
        }
    }

    /// Construct a tuple-payload enum variant `.tag(p0, p1, ...)`.
    pub fn tuple_variant(tag: VariantTag, fields: Vec<Pattern>) -> Self {
        Self::EnumVariant {
            tag,
            payload: VariantPayloadPattern::Tuple(fields),
        }
    }

    /// Construct a record-payload enum variant `.tag { k0: p0, ... }`.
    pub fn record_variant(tag: VariantTag, fields: BTreeMap<String, Pattern>) -> Self {
        Self::EnumVariant {
            tag,
            payload: VariantPayloadPattern::Record(fields),
        }
    }

    /// Validate structural binder rules (non-empty `Or`, consistent `Or` binder sets,
    /// no duplicate binders in conjunctive patterns) and return the exported binder names.
    pub fn validate_binders(&self) -> Result<BTreeSet<String>, CompileError> {
        let mut out = BTreeSet::new();
        self.collect_binders(&mut out)?;
        Ok(out)
    }

    fn collect_binders(&self, out: &mut BTreeSet<String>) -> Result<(), CompileError> {
        match self {
            Self::Wild
            | Self::Bool(_)
            | Self::Char(_)
            | Self::CharRange { .. }
            | Self::Int(_)
            | Self::IntRange { .. }
            | Self::OptionalNull => Ok(()),
            Self::Bind { name, subpattern } => {
                subpattern.collect_binders(out)?;
                if !out.insert(name.clone()) {
                    return Err(CompileError::DuplicateBinding { name: name.clone() });
                }
                Ok(())
            }
            Self::OptionalPresent(inner) | Self::ErrorOk(inner) => inner.collect_binders(out),
            Self::ErrorErr { inner, .. } => inner.collect_binders(out),
            Self::EnumVariant { payload, .. } => match payload {
                VariantPayloadPattern::Unit => Ok(()),
                VariantPayloadPattern::Tuple(pats) => {
                    for pat in pats {
                        pat.collect_binders(out)?;
                    }
                    Ok(())
                }
                VariantPayloadPattern::Record(fields) => {
                    for pat in fields.values() {
                        pat.collect_binders(out)?;
                    }
                    Ok(())
                }
            },
            Self::Tuple(pats) => {
                for pat in pats {
                    pat.collect_binders(out)?;
                }
                Ok(())
            }
            Self::Record(fields) => {
                for pat in fields.values() {
                    pat.collect_binders(out)?;
                }
                Ok(())
            }
            Self::List { prefix, rest } => {
                for pat in prefix {
                    pat.collect_binders(out)?;
                }
                if let Some(rest_pat) = rest {
                    rest_pat.collect_binders(out)?;
                }
                Ok(())
            }
            Self::BitVec(bv) => {
                for seg in &bv.segments {
                    if let BitSegmentPattern::Typed { pattern, .. } = seg {
                        pattern.collect_binders(out)?;
                    }
                }
                Ok(())
            }
            Self::Or(alts) => {
                if alts.is_empty() {
                    return Err(CompileError::EmptyOrPattern);
                }
                let expected = alts[0].validate_binders()?;
                for alt in &alts[1..] {
                    let found = alt.validate_binders()?;
                    if found != expected {
                        return Err(CompileError::InconsistentOrBindings { expected, found });
                    }
                }
                for name in expected {
                    if !out.insert(name.clone()) {
                        return Err(CompileError::DuplicateBinding { name });
                    }
                }
                Ok(())
            }
        }
    }

    /// Validate that this pattern is type-compatible with `ty`.
    pub fn validate_against_type(&self, ty: &TypeDesc) -> Result<(), CompileError> {
        match (self, ty) {
            (Self::Wild, _) => Ok(()),
            (Self::Bind { subpattern, .. }, _) => subpattern.validate_against_type(ty),
            (Self::Or(alts), _) => {
                for alt in alts {
                    alt.validate_against_type(ty)?;
                }
                Ok(())
            }
            (Self::Bool(_), TypeDesc::Bool) => Ok(()),
            (Self::Char(_) | Self::CharRange { .. }, TypeDesc::Char) => Ok(()),
            (Self::Int(_) | Self::IntRange { .. }, TypeDesc::Int(_)) => Ok(()),
            (Self::OptionalNull, TypeDesc::Optional(_)) => Ok(()),
            (Self::OptionalPresent(inner), TypeDesc::Optional(inner_ty)) => {
                inner.validate_against_type(inner_ty)
            }
            (Self::ErrorOk(inner), TypeDesc::ErrorRow(row)) => {
                inner.validate_against_type(&row.ok_type)
            }
            (
                Self::ErrorErr {
                    tag: Some(t),
                    inner,
                },
                TypeDesc::ErrorRow(row),
            ) => {
                if let Some(payload_ty) = row.error_tags.get(t) {
                    inner.validate_against_type(payload_ty)
                } else if row.is_open {
                    inner.validate_against_type(&TypeDesc::Any)
                } else {
                    Err(CompileError::UnknownErrorTag { tag: t.clone() })
                }
            }
            (Self::ErrorErr { tag: None, inner }, TypeDesc::ErrorRow(row)) => {
                if row.error_tags.is_empty() && row.is_open {
                    inner.validate_against_type(&TypeDesc::Any)
                } else if !row.is_open && row.error_tags.len() == 1 {
                    let payload_ty = row.error_tags.values().next().expect("checked len == 1");
                    inner.validate_against_type(payload_ty)
                } else {
                    // For multi-tag or open error rows, an untagged `error inner` must match any tag payload.
                    for payload_ty in row.error_tags.values() {
                        inner.validate_against_type(payload_ty)?;
                    }
                    if row.is_open {
                        inner.validate_against_type(&TypeDesc::Any)?;
                    }
                    Ok(())
                }
            }
            (Self::EnumVariant { tag, payload }, TypeDesc::Enum(enum_ty)) => {
                if let Some(schema) = enum_ty.variants.get(tag) {
                    validate_variant_payload(payload, &schema.payload)
                } else if enum_ty.is_open {
                    validate_open_variant_payload(payload)
                } else {
                    Err(CompileError::UnknownVariant {
                        enum_name: enum_ty.name.clone(),
                        tag: tag.clone(),
                    })
                }
            }
            (Self::Tuple(pats), TypeDesc::Tuple(tys)) => {
                if pats.len() != tys.len() {
                    return Err(CompileError::TypeMismatch {
                        expected: format!("tuple of arity {}", tys.len()),
                        found: format!("tuple of arity {}", pats.len()),
                    });
                }
                for (p, t) in pats.iter().zip(tys.iter()) {
                    p.validate_against_type(t)?;
                }
                Ok(())
            }
            (Self::Record(pat_fields), TypeDesc::Record(ty_fields)) => {
                for (name, pat) in pat_fields {
                    let Some(field_ty) = ty_fields.get(name) else {
                        return Err(CompileError::UnknownField {
                            field: name.clone(),
                        });
                    };
                    pat.validate_against_type(field_ty)?;
                }
                Ok(())
            }
            (Self::List { prefix, rest }, TypeDesc::List(elem_ty)) => {
                for pat in prefix {
                    pat.validate_against_type(elem_ty)?;
                }
                if let Some(rest_pat) = rest {
                    rest_pat.validate_against_type(ty)?;
                }
                Ok(())
            }
            (Self::BitVec(bv), TypeDesc::BitVec(width)) => {
                let sum = bv.computed_width();
                if sum != *width {
                    return Err(CompileError::InvalidBitVecWidth {
                        expected: *width,
                        found: sum,
                    });
                }
                if let Some(declared) = bv.total_width
                    && declared != *width
                {
                    return Err(CompileError::InvalidBitVecWidth {
                        expected: *width,
                        found: declared,
                    });
                }
                for seg in &bv.segments {
                    if let BitSegmentPattern::Typed {
                        width: seg_w,
                        ty: seg_ty,
                        pattern,
                    } = seg
                    {
                        let valid_width = match seg_ty {
                            TypeDesc::BitVec(w) => *w == *seg_w,
                            TypeDesc::Bool => *seg_w == 1,
                            TypeDesc::Int(IntType::UnsignedBits(w))
                            | TypeDesc::Int(IntType::SignedBits(w)) => u32::from(*w) == *seg_w,
                            _ => false,
                        };
                        if !valid_width {
                            return Err(CompileError::TypeMismatch {
                                expected: format!("bit-slice type of width {seg_w}"),
                                found: seg_ty.to_string(),
                            });
                        }
                        pattern.validate_against_type(seg_ty)?;
                    }
                }
                Ok(())
            }
            (other, expected_ty) => Err(CompileError::TypeMismatch {
                expected: expected_ty.to_string(),
                found: other.to_string(),
            }),
        }
    }
}

fn validate_variant_payload(
    pat: &VariantPayloadPattern,
    ty: &VariantPayloadType,
) -> Result<(), CompileError> {
    match (pat, ty) {
        (VariantPayloadPattern::Unit, VariantPayloadType::Unit) => Ok(()),
        (VariantPayloadPattern::Tuple(ps), VariantPayloadType::Tuple(ts)) => {
            if ps.len() != ts.len() {
                return Err(CompileError::TypeMismatch {
                    expected: format!("variant tuple payload of arity {}", ts.len()),
                    found: format!("variant tuple payload of arity {}", ps.len()),
                });
            }
            for (p, t) in ps.iter().zip(ts.iter()) {
                p.validate_against_type(t)?;
            }
            Ok(())
        }
        (VariantPayloadPattern::Record(p_fields), VariantPayloadType::Record(t_fields)) => {
            for (name, p) in p_fields {
                let Some(t) = t_fields.get(name) else {
                    return Err(CompileError::UnknownField {
                        field: name.clone(),
                    });
                };
                p.validate_against_type(t)?;
            }
            Ok(())
        }
        _ => Err(CompileError::TypeMismatch {
            expected: ty.to_string(),
            found: pat.to_string(),
        }),
    }
}

fn validate_open_variant_payload(pat: &VariantPayloadPattern) -> Result<(), CompileError> {
    match pat {
        VariantPayloadPattern::Unit => Ok(()),
        VariantPayloadPattern::Tuple(ps) => {
            for p in ps {
                p.validate_against_type(&TypeDesc::Any)?;
            }
            Ok(())
        }
        VariantPayloadPattern::Record(fields) => {
            for p in fields.values() {
                p.validate_against_type(&TypeDesc::Any)?;
            }
            Ok(())
        }
    }
}

impl fmt::Display for Pattern {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Wild => f.write_str("_"),
            Self::Bind { name, subpattern } => {
                f.write_str("(bind ")?;
                sexpr::write_quoted_str(f, name)?;
                write!(f, " {subpattern})")
            }
            Self::Bool(b) => write!(f, "(bool {b})"),
            Self::Char(c) => {
                f.write_str("(char ")?;
                sexpr::write_quoted_char(f, *c)?;
                f.write_str(")")
            }
            Self::CharRange { start, end } => {
                f.write_str("(char-range ")?;
                write_char_bound(f, *start)?;
                f.write_str(" ")?;
                write_char_bound(f, *end)?;
                f.write_str(")")
            }
            Self::Int(lit) => write!(f, "{lit}"),
            Self::IntRange { start, end } => write!(f, "(int-range {start} {end})"),
            Self::OptionalNull => f.write_str("(null)"),
            Self::OptionalPresent(inner) => write!(f, "(present {inner})"),
            Self::ErrorOk(inner) => write!(f, "(ok {inner})"),
            Self::ErrorErr { tag, inner } => match tag {
                Some(t) => write!(f, "(err {t} {inner})"),
                None => write!(f, "(err-any {inner})"),
            },
            Self::EnumVariant { tag, payload } => write!(f, "(variant {tag} {payload})"),
            Self::Tuple(pats) => {
                f.write_str("(tuple")?;
                for pat in pats {
                    write!(f, " {pat}")?;
                }
                f.write_str(")")
            }
            Self::Record(fields) => {
                f.write_str("(record")?;
                for (name, pat) in fields {
                    f.write_str(" (")?;
                    sexpr::write_quoted_str(f, name)?;
                    write!(f, " {pat})")?;
                }
                f.write_str(")")
            }
            Self::List { prefix, rest } => {
                f.write_str("(list (prefix")?;
                for pat in prefix {
                    write!(f, " {pat}")?;
                }
                f.write_str(")")?;
                if let Some(rest_pat) = rest {
                    write!(f, " (rest {rest_pat})")?;
                }
                f.write_str(")")
            }
            Self::BitVec(bv) => write!(f, "{bv}"),
            Self::Or(alts) => {
                f.write_str("(or")?;
                for alt in alts {
                    write!(f, " {alt}")?;
                }
                f.write_str(")")
            }
        }
    }
}

fn write_char_bound(f: &mut fmt::Formatter<'_>, bound: RangeBound<char>) -> fmt::Result {
    match bound {
        RangeBound::Unbounded => f.write_str("unbounded"),
        RangeBound::Inclusive(c) => {
            f.write_str("(inclusive ")?;
            sexpr::write_quoted_char(f, c)?;
            f.write_str(")")
        }
        RangeBound::Exclusive(c) => {
            f.write_str("(exclusive ")?;
            sexpr::write_quoted_char(f, c)?;
            f.write_str(")")
        }
    }
}

/// Single arm in a pattern match matrix.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MatchArm {
    pub arm_index: usize,
    pub pattern: Pattern,
    pub has_guard: bool,
    pub bindings: BTreeSet<String>,
}

impl MatchArm {
    /// Construct an unguarded match arm, extracting binder names automatically.
    pub fn unguarded(arm_index: usize, pattern: Pattern) -> Result<Self, CompileError> {
        let bindings = pattern.validate_binders()?;
        Ok(Self {
            arm_index,
            pattern,
            has_guard: false,
            bindings,
        })
    }

    /// Construct a guarded match arm, extracting binder names automatically.
    pub fn guarded(arm_index: usize, pattern: Pattern) -> Result<Self, CompileError> {
        let bindings = pattern.validate_binders()?;
        Ok(Self {
            arm_index,
            pattern,
            has_guard: true,
            bindings,
        })
    }

    /// Construct a match arm with explicit guard flag and validated binder set.
    pub fn new(arm_index: usize, pattern: Pattern, has_guard: bool) -> Result<Self, CompileError> {
        let bindings = pattern.validate_binders()?;
        Ok(Self {
            arm_index,
            pattern,
            has_guard,
            bindings,
        })
    }
}

impl fmt::Display for MatchArm {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(arm (index {}) (guard {}) (bindings",
            self.arm_index, self.has_guard
        )?;
        for name in &self.bindings {
            f.write_str(" ")?;
            sexpr::write_quoted_str(f, name)?;
        }
        write!(f, ") (pattern {}))", self.pattern)
    }
}

/// Top-level pattern-match compilation input matrix.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PatternMatrix {
    pub scrutinee_type: TypeDesc,
    pub arms: Vec<MatchArm>,
}

impl PatternMatrix {
    /// Create a validated pattern matrix for `scrutinee_type`.
    pub fn new(scrutinee_type: TypeDesc, arms: Vec<MatchArm>) -> Result<Self, CompileError> {
        let matrix = Self {
            scrutinee_type,
            arms,
        };
        matrix.validate()?;
        Ok(matrix)
    }

    /// Validate binders and type-compatibility across all arms.
    pub fn validate(&self) -> Result<(), CompileError> {
        for arm in &self.arms {
            let computed = arm.pattern.validate_binders()?;
            if computed != arm.bindings {
                return Err(CompileError::InconsistentOrBindings {
                    expected: arm.bindings.clone(),
                    found: computed,
                });
            }
            arm.pattern.validate_against_type(&self.scrutinee_type)?;
        }
        Ok(())
    }
}

impl fmt::Display for PatternMatrix {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "(pattern-matrix (type {}) (arms", self.scrutinee_type)?;
        for arm in &self.arms {
            write!(f, " {arm}")?;
        }
        f.write_str("))")
    }
}

/// Compilation budget bounding specialization steps and allocated DAG nodes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CompileBudget {
    pub max_steps: usize,
    pub max_nodes: usize,
    pub max_witnesses: usize,
}

impl Default for CompileBudget {
    fn default() -> Self {
        Self {
            max_steps: 100_000,
            max_nodes: 50_000,
            max_witnesses: 32,
        }
    }
}

impl fmt::Display for CompileBudget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(budget (max-steps {}) (max-nodes {}) (max-witnesses {}))",
            self.max_steps, self.max_nodes, self.max_witnesses
        )
    }
}

/// Errors returned during pattern validation or decision-DAG compilation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CompileError {
    /// Compilation step or node budget exceeded before completing the DAG.
    BudgetExceeded {
        steps_used: usize,
        nodes_created: usize,
        budget: CompileBudget,
    },
    /// `or` pattern branches export inconsistent binder sets.
    InconsistentOrBindings {
        expected: BTreeSet<String>,
        found: BTreeSet<String>,
    },
    /// `or` pattern contained zero alternatives.
    EmptyOrPattern,
    /// Same binder appeared more than once inside one conjunctive pattern branch.
    DuplicateBinding { name: String },
    /// `BitVec<N>` segment widths did not match expected `N`.
    InvalidBitVecWidth { expected: u32, found: u32 },
    /// Pattern does not match the column's `TypeDesc`.
    TypeMismatch { expected: String, found: String },
    /// Record or variant record pattern referenced an unknown field name.
    UnknownField { field: String },
    /// Enum pattern referenced a variant absent from a closed enum universe.
    UnknownVariant { enum_name: String, tag: VariantTag },
    /// Error pattern referenced an error tag absent from a closed error row.
    UnknownErrorTag { tag: ErrorTag },
}

impl fmt::Display for CompileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BudgetExceeded {
                steps_used,
                nodes_created,
                budget,
            } => write!(
                f,
                "(compile-error budget-exceeded (steps {steps_used}) (nodes {nodes_created}) {budget})"
            ),
            Self::InconsistentOrBindings { expected, found } => {
                f.write_str("(compile-error inconsistent-or-bindings (expected")?;
                for name in expected {
                    f.write_str(" ")?;
                    sexpr::write_quoted_str(f, name)?;
                }
                f.write_str(") (found")?;
                for name in found {
                    f.write_str(" ")?;
                    sexpr::write_quoted_str(f, name)?;
                }
                f.write_str("))")
            }
            Self::EmptyOrPattern => f.write_str("(compile-error empty-or-pattern)"),
            Self::DuplicateBinding { name } => {
                f.write_str("(compile-error duplicate-binding ")?;
                sexpr::write_quoted_str(f, name)?;
                f.write_str(")")
            }
            Self::InvalidBitVecWidth { expected, found } => write!(
                f,
                "(compile-error invalid-bitvec-width (expected {expected}) (found {found}))"
            ),
            Self::TypeMismatch { expected, found } => {
                f.write_str("(compile-error type-mismatch (expected ")?;
                sexpr::write_quoted_str(f, expected)?;
                f.write_str(") (found ")?;
                sexpr::write_quoted_str(f, found)?;
                f.write_str("))")
            }
            Self::UnknownField { field } => {
                f.write_str("(compile-error unknown-field ")?;
                sexpr::write_quoted_str(f, field)?;
                f.write_str(")")
            }
            Self::UnknownVariant { enum_name, tag } => {
                f.write_str("(compile-error unknown-variant ")?;
                sexpr::write_quoted_str(f, enum_name)?;
                write!(f, " {tag})")
            }
            Self::UnknownErrorTag { tag } => write!(f, "(compile-error unknown-error-tag {tag})"),
        }
    }
}

impl std::error::Error for CompileError {}
