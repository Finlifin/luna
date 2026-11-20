//! Pure finite-DAG BitVec IR, domain constraints, and behavior boundaries.
//!
//! This module implements the core intermediate representation described in
//! `docs/program-synthesis/bitvec-algebra.md`:
//! - arbitrary-width `BitValue` (LSB = index 0, `a ++ b` places `a` in high bits),
//! - finite acyclic `Node` DAGs with structural sharing that never merges
//!   distinct memory versions or side-effecting evaluation orders,
//! - explicit `DomainConstraint` (`P(x)`) for valid input domains and bounds,
//! - explicit `BehaviorBoundary` preserving evaluation order, memory reads,
//!   side effects, dynamic bounds checks, and cleanup actions.

use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::{self, Display, Write};

use crate::sexpr::{Quoted, write_list};

/// Identifier for a DAG node within a [`Region`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NodeId(pub usize);

impl Display for NodeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "(node-id {})", self.0)
    }
}

/// Errors produced when constructing or validating BitVec values and regions.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IrError {
    /// Integer literal does not fit in the target bit width (`0 <= v < 2^n`).
    LiteralOutOfRange { width: u32, detail: String },
    /// Expected matching widths for a binary or equality operation.
    WidthMismatch {
        op: &'static str,
        left: u32,
        right: u32,
    },
    /// Bit width overflow (`checked_add` exceeded `u32::MAX`).
    WidthOverflow { left: u32, right: u32 },
    /// Static slice `[start .. start + width)` exceeds input width.
    SliceOutOfBounds {
        input_width: u32,
        start: u32,
        slice_width: u32,
    },
    /// Narrowing `ZeroExtend` is statically illegal; slicing must be explicit.
    InvalidZeroExtend { input_width: u32, target_width: u32 },
    /// Referenced node index does not exist in the DAG.
    InvalidNodeId(NodeId),
    /// Dynamic extract index must have positive bit width.
    ZeroWidthDynamicIndex,
    /// Single-bit index out of bounds for `BitValue::bit`.
    BitIndexOutOfBounds { width: u32, index: u32 },
    /// Cycle detected in DAG or topological invariant violated.
    CyclicDag(NodeId),
    /// Input width mismatch across DAG nodes or region inputs.
    ConflictingInputWidth {
        identity: InputIdentity,
        expected: u32,
        actual: u32,
    },
    /// Missing input value during evaluation.
    MissingInput(InputIdentity),
    /// Provided input value has wrong bit width.
    InputWidthMismatch {
        identity: InputIdentity,
        expected: u32,
        actual: u32,
    },
    /// Word slice provided to `BitValue::from_words` does not match `width`.
    InvalidWordSlice {
        width: u32,
        expected_words: usize,
        actual_words: usize,
        non_zero_padding: bool,
    },
    /// Byte swap requires a non-zero multiple-of-8 bit width.
    InvalidByteSwapWidth(u32),
    /// Region output width does not match root node width or boolean expectation.
    InvalidOutputKind {
        root_width: u32,
        expected: OutputKind,
    },
}

impl Display for IrError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::LiteralOutOfRange { width, detail } => {
                write!(
                    f,
                    "(ir-error literal-out-of-range (width {width}) (detail {}))",
                    Quoted(detail)
                )
            }
            Self::WidthMismatch { op, left, right } => {
                write!(
                    f,
                    "(ir-error width-mismatch (op {}) (left {left}) (right {right}))",
                    Quoted(op)
                )
            }
            Self::WidthOverflow { left, right } => {
                write!(f, "(ir-error width-overflow (left {left}) (right {right}))")
            }
            Self::SliceOutOfBounds {
                input_width,
                start,
                slice_width,
            } => write!(
                f,
                "(ir-error slice-out-of-bounds (input-width {input_width}) (start {start}) (width {slice_width}))"
            ),
            Self::InvalidZeroExtend {
                input_width,
                target_width,
            } => write!(
                f,
                "(ir-error invalid-zero-extend (input-width {input_width}) (target-width {target_width}))"
            ),
            Self::InvalidNodeId(id) => write!(f, "(ir-error invalid-node {id})"),
            Self::ZeroWidthDynamicIndex => f.write_str("(ir-error zero-width-dynamic-index)"),
            Self::BitIndexOutOfBounds { width, index } => {
                write!(
                    f,
                    "(ir-error bit-index-out-of-bounds (width {width}) (index {index}))"
                )
            }
            Self::CyclicDag(id) => write!(f, "(ir-error cyclic-dag {id})"),
            Self::ConflictingInputWidth {
                identity,
                expected,
                actual,
            } => write!(
                f,
                "(ir-error conflicting-input-width (identity {identity}) (expected {expected}) (actual {actual}))"
            ),
            Self::MissingInput(identity) => {
                write!(f, "(ir-error missing-input {identity})")
            }
            Self::InputWidthMismatch {
                identity,
                expected,
                actual,
            } => write!(
                f,
                "(ir-error input-width-mismatch (identity {identity}) (expected {expected}) (actual {actual}))"
            ),
            Self::InvalidWordSlice {
                width,
                expected_words,
                actual_words,
                non_zero_padding,
            } => write!(
                f,
                "(ir-error invalid-word-slice (width {width}) (expected-words {expected_words}) (actual-words {actual_words}) (non-zero-padding {non_zero_padding}))"
            ),
            Self::InvalidByteSwapWidth(width) => {
                write!(f, "(ir-error invalid-bswap-width (width {width}))")
            }
            Self::InvalidOutputKind {
                root_width,
                expected,
            } => write!(
                f,
                "(ir-error invalid-output-kind (root-width {root_width}) (expected {expected}))"
            ),
        }
    }
}

impl std::error::Error for IrError {}

/// Returns the bitmask for valid bits in the highest 64-bit word of a `width`-bit value.
///
/// Explicitly guards against `1u64 << 64` undefined behavior when `width % 64 == 0`.
pub fn top_word_mask(width: u32) -> u64 {
    if width == 0 {
        return 0;
    }
    let rem = width % 64;
    if rem == 0 {
        u64::MAX
    } else {
        (1u64 << rem) - 1
    }
}

/// Arbitrary-width unsigned bit-vector value.
///
/// Bits are stored in little-endian 64-bit word order (`words[0]` holds bits
/// `0..64`, index `0` is LSB). Unused high padding bits in the top word are
/// strictly masked to zero. `BitVec<0>` has `width == 0` and an empty `words` vec.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BitValue {
    width: u32,
    words: Vec<u64>,
}

impl BitValue {
    /// Returns the unique empty value of `BitVec<0>`.
    pub fn empty() -> Self {
        Self {
            width: 0,
            words: Vec::new(),
        }
    }

    /// Returns a `width`-bit zero value.
    pub fn zeros(width: u32) -> Self {
        let num_words = width.div_ceil(64) as usize;
        Self {
            width,
            words: vec![0; num_words],
        }
    }

    /// Returns a `width`-bit all-ones value (`not 0`), with unused tail bits masked to 0.
    pub fn ones(width: u32) -> Self {
        if width == 0 {
            return Self::empty();
        }
        let num_words = width.div_ceil(64) as usize;
        let mut words = vec![u64::MAX; num_words];
        if let Some(last) = words.last_mut() {
            *last &= top_word_mask(width);
        }
        Self { width, words }
    }

    /// Returns the 1-bit encoding of a boolean (`1` for `true`, `0` for `false`).
    pub fn from_bool(value: bool) -> Self {
        Self {
            width: 1,
            words: vec![u64::from(value)],
        }
    }

    /// Constructs a `width`-bit value from `u64`, rejecting literals outside `0..2^width`.
    pub fn from_u64(width: u32, value: u64) -> Result<Self, IrError> {
        if width == 0 {
            if value == 0 {
                return Ok(Self::empty());
            }
            return Err(IrError::LiteralOutOfRange {
                width,
                detail: format!("{value} does not fit in 0 bits"),
            });
        }
        if width < 64 {
            let max_mask = (1u64 << width) - 1;
            if value > max_mask {
                return Err(IrError::LiteralOutOfRange {
                    width,
                    detail: format!("{value} >= 2^{width}"),
                });
            }
        }
        let num_words = width.div_ceil(64) as usize;
        let mut words = vec![0; num_words];
        words[0] = value;
        Ok(Self { width, words })
    }

    /// Constructs a `width`-bit value from `u128`, rejecting literals outside `0..2^width`.
    pub fn from_u128(width: u32, value: u128) -> Result<Self, IrError> {
        if width == 0 {
            if value == 0 {
                return Ok(Self::empty());
            }
            return Err(IrError::LiteralOutOfRange {
                width,
                detail: format!("{value} does not fit in 0 bits"),
            });
        }
        if width < 128 {
            let max_mask = (1u128 << width) - 1;
            if value > max_mask {
                return Err(IrError::LiteralOutOfRange {
                    width,
                    detail: format!("{value} >= 2^{width}"),
                });
            }
        }
        let num_words = width.div_ceil(64) as usize;
        let mut words = vec![0; num_words];
        words[0] = value as u64;
        if num_words > 1 {
            words[1] = (value >> 64) as u64;
        }
        Ok(Self { width, words })
    }

    /// Constructs a `BitValue` from exact little-endian 64-bit words.
    pub fn from_words(width: u32, words: Vec<u64>) -> Result<Self, IrError> {
        let expected_words = width.div_ceil(64) as usize;
        if words.len() != expected_words {
            return Err(IrError::InvalidWordSlice {
                width,
                expected_words,
                actual_words: words.len(),
                non_zero_padding: false,
            });
        }
        if let Some(&last) = words.last() {
            let mask = top_word_mask(width);
            if (last & !mask) != 0 {
                return Err(IrError::InvalidWordSlice {
                    width,
                    expected_words,
                    actual_words: words.len(),
                    non_zero_padding: true,
                });
            }
        }
        Ok(Self { width, words })
    }

    /// Constructs a `BitValue` from an LSB-first slice of booleans.
    pub fn from_bits_lsb(bits: &[bool]) -> Self {
        let width = bits.len() as u32;
        if width == 0 {
            return Self::empty();
        }
        let mut out = Self::zeros(width);
        for (idx, &b) in bits.iter().enumerate() {
            if b {
                out.words[idx / 64] |= 1u64 << (idx % 64);
            }
        }
        out
    }

    /// Bit width of this value.
    pub fn width(&self) -> u32 {
        self.width
    }

    /// Little-endian 64-bit backing words with zeroed unused high padding bits.
    pub fn words(&self) -> &[u64] {
        &self.words
    }

    /// Reads bit `index` (where `0` is LSB), returning `false` or `true`.
    pub fn bit(&self, index: u32) -> Result<bool, IrError> {
        if index >= self.width {
            return Err(IrError::BitIndexOutOfBounds {
                width: self.width,
                index,
            });
        }
        let word_idx = (index / 64) as usize;
        let bit_idx = index % 64;
        Ok(((self.words[word_idx] >> bit_idx) & 1) == 1)
    }

    /// Returns `true` if all bits are zero (including `BitVec<0>`).
    pub fn is_zero(&self) -> bool {
        self.words.iter().all(|&w| w == 0)
    }

    /// Returns `true` if all `width` bits are `1` (including `BitVec<0>`).
    pub fn is_all_ones(&self) -> bool {
        if self.width == 0 {
            return true;
        }
        let last_idx = self.words.len() - 1;
        for (idx, &w) in self.words.iter().enumerate() {
            let expected = if idx == last_idx {
                top_word_mask(self.width)
            } else {
                u64::MAX
            };
            if w != expected {
                return false;
            }
        }
        true
    }

    /// Returns `Some(val)` when `width <= 64`, otherwise `None`.
    pub fn to_u64(&self) -> Option<u64> {
        if self.width == 0 {
            Some(0)
        } else if self.width <= 64 {
            Some(self.words[0])
        } else {
            None
        }
    }

    /// Returns `Some(val)` when `width <= 128`, otherwise `None`.
    pub fn to_u128(&self) -> Option<u128> {
        if self.width == 0 {
            Some(0)
        } else if self.width <= 64 {
            Some(u128::from(self.words[0]))
        } else if self.width <= 128 {
            Some(u128::from(self.words[0]) | (u128::from(self.words[1]) << 64))
        } else {
            None
        }
    }

    /// Interprets a 1-bit `BitValue` as a `bool`, returning `None` if `width != 1`.
    pub fn to_bool(&self) -> Option<bool> {
        if self.width == 1 {
            Some(self.words[0] == 1)
        } else {
            None
        }
    }

    /// Concatenates `self` (high bits) and `low` (low bits): `self ++ low`.
    pub fn concat(&self, low: &Self) -> Result<Self, IrError> {
        let total_width = self
            .width
            .checked_add(low.width)
            .ok_or(IrError::WidthOverflow {
                left: self.width,
                right: low.width,
            })?;
        if total_width == 0 {
            return Ok(Self::empty());
        }
        if self.width == 0 {
            return Ok(low.clone());
        }
        if low.width == 0 {
            return Ok(self.clone());
        }
        let num_words = total_width.div_ceil(64) as usize;
        let mut out = vec![0u64; num_words];
        out[..low.words.len()].copy_from_slice(&low.words);

        let word_off = (low.width / 64) as usize;
        let shift = low.width % 64;
        if shift == 0 {
            for (idx, &w) in self.words.iter().enumerate() {
                out[word_off + idx] = w;
            }
        } else {
            let inv_shift = 64 - shift;
            for (idx, &w) in self.words.iter().enumerate() {
                out[word_off + idx] |= w << shift;
                if word_off + idx + 1 < num_words {
                    out[word_off + idx + 1] |= w >> inv_shift;
                }
            }
        }
        if let Some(last) = out.last_mut() {
            *last &= top_word_mask(total_width);
        }
        Ok(Self {
            width: total_width,
            words: out,
        })
    }

    /// Extracts half-open bit range `[start .. start + width)`.
    pub fn extract(&self, start: u32, width: u32) -> Result<Self, IrError> {
        let end = start.checked_add(width).ok_or(IrError::SliceOutOfBounds {
            input_width: self.width,
            start,
            slice_width: width,
        })?;
        if end > self.width {
            return Err(IrError::SliceOutOfBounds {
                input_width: self.width,
                start,
                slice_width: width,
            });
        }
        if width == 0 {
            return Ok(Self::empty());
        }
        let num_words = width.div_ceil(64) as usize;
        let mut out = vec![0u64; num_words];
        let word_off = (start / 64) as usize;
        let shift = start % 64;
        if shift == 0 {
            for (idx, slot) in out.iter_mut().enumerate() {
                *slot = self.words[word_off + idx];
            }
        } else {
            let inv_shift = 64 - shift;
            for (idx, slot) in out.iter_mut().enumerate() {
                let lo = self.words[word_off + idx] >> shift;
                let hi = if word_off + idx + 1 < self.words.len() {
                    self.words[word_off + idx + 1] << inv_shift
                } else {
                    0
                };
                *slot = lo | hi;
            }
        }
        if let Some(last) = out.last_mut() {
            *last &= top_word_mask(width);
        }
        Ok(Self { width, words: out })
    }

    /// Bitwise AND of two same-width values.
    pub fn bit_and(&self, rhs: &Self) -> Result<Self, IrError> {
        if self.width != rhs.width {
            return Err(IrError::WidthMismatch {
                op: "and",
                left: self.width,
                right: rhs.width,
            });
        }
        let words = self
            .words
            .iter()
            .zip(&rhs.words)
            .map(|(&a, &b)| a & b)
            .collect();
        Ok(Self {
            width: self.width,
            words,
        })
    }

    /// Bitwise OR of two same-width values.
    pub fn bit_or(&self, rhs: &Self) -> Result<Self, IrError> {
        if self.width != rhs.width {
            return Err(IrError::WidthMismatch {
                op: "or",
                left: self.width,
                right: rhs.width,
            });
        }
        let words = self
            .words
            .iter()
            .zip(&rhs.words)
            .map(|(&a, &b)| a | b)
            .collect();
        Ok(Self {
            width: self.width,
            words,
        })
    }

    /// Bitwise XOR of two same-width values.
    pub fn bit_xor(&self, rhs: &Self) -> Result<Self, IrError> {
        if self.width != rhs.width {
            return Err(IrError::WidthMismatch {
                op: "xor",
                left: self.width,
                right: rhs.width,
            });
        }
        let words = self
            .words
            .iter()
            .zip(&rhs.words)
            .map(|(&a, &b)| a ^ b)
            .collect();
        Ok(Self {
            width: self.width,
            words,
        })
    }

    /// Bitwise inversion flipping only the valid `width` bits (clearing unused padding bits).
    pub fn bit_not(&self) -> Self {
        if self.width == 0 {
            return Self::empty();
        }
        let mut words: Vec<u64> = self.words.iter().map(|&w| !w).collect();
        if let Some(last) = words.last_mut() {
            *last &= top_word_mask(self.width);
        }
        Self {
            width: self.width,
            words,
        }
    }

    /// Zero-extends `self` to `target_width >= self.width`.
    pub fn zero_extend(&self, target_width: u32) -> Result<Self, IrError> {
        if target_width < self.width {
            return Err(IrError::InvalidZeroExtend {
                input_width: self.width,
                target_width,
            });
        }
        if target_width == 0 {
            return Ok(Self::empty());
        }
        let num_words = target_width.div_ceil(64) as usize;
        let mut words = vec![0u64; num_words];
        words[..self.words.len()].copy_from_slice(&self.words);
        Ok(Self {
            width: target_width,
            words,
        })
    }

    /// Unsigned integer comparison across arbitrary bit widths (zero-extending shorter operand).
    pub fn cmp_unsigned(&self, rhs: &Self) -> Ordering {
        let max_words = self.words.len().max(rhs.words.len());
        for idx in (0..max_words).rev() {
            let a = self.words.get(idx).copied().unwrap_or(0);
            let b = rhs.words.get(idx).copied().unwrap_or(0);
            match a.cmp(&b) {
                Ordering::Equal => continue,
                non_eq => return non_eq,
            }
        }
        Ordering::Equal
    }

    /// Logical left shift by a bounded constant `amount < self.width`.
    pub fn shl_const(&self, amount: u32) -> Result<Self, IrError> {
        if self.width == 0 || amount >= self.width {
            return Err(IrError::SliceOutOfBounds {
                input_width: self.width,
                start: amount,
                slice_width: 1,
            });
        }
        if amount == 0 {
            return Ok(self.clone());
        }
        let low = Self::zeros(amount);
        let kept = self.extract(0, self.width - amount)?;
        kept.concat(&low)
    }

    /// Logical right shift by a bounded constant `amount < self.width`.
    pub fn lshr_const(&self, amount: u32) -> Result<Self, IrError> {
        if self.width == 0 || amount >= self.width {
            return Err(IrError::SliceOutOfBounds {
                input_width: self.width,
                start: amount,
                slice_width: 1,
            });
        }
        if amount == 0 {
            return Ok(self.clone());
        }
        let kept = self.extract(amount, self.width - amount)?;
        kept.zero_extend(self.width)
    }

    /// Rotate right by `amount` bits (exact bit semantics, safe even for non-power-of-2 widths).
    pub fn rotr_const(&self, amount: u32) -> Self {
        if self.width <= 1 {
            return self.clone();
        }
        let k = amount % self.width;
        if k == 0 {
            return self.clone();
        }
        let low = self.extract(0, k).expect("k < width");
        let high = self.extract(k, self.width - k).expect("k < width");
        low.concat(&high).expect("sum equals width")
    }

    /// Rotate left by `amount` bits.
    pub fn rotl_const(&self, amount: u32) -> Self {
        if self.width <= 1 {
            return self.clone();
        }
        let k = amount % self.width;
        if k == 0 {
            return self.clone();
        }
        self.rotr_const(self.width - k)
    }

    /// Reverses the byte order of a multiple-of-8 bit value.
    pub fn bswap_bytes(&self) -> Result<Self, IrError> {
        if self.width == 0 || !self.width.is_multiple_of(8) {
            return Err(IrError::InvalidByteSwapWidth(self.width));
        }
        let num_bytes = self.width / 8;
        let mut acc = Self::empty();
        for b in 0..num_bytes {
            let byte = self.extract(b * 8, 8)?;
            acc = acc.concat(&byte)?;
        }
        Ok(acc)
    }

    /// Formats the value as fixed-width lowercase hex (`0x...`, or `0x0` for width 0).
    pub fn to_hex_string(&self) -> String {
        if self.width == 0 {
            return String::from("0");
        }
        let hex_digits = self.width.div_ceil(4) as usize;
        let mut s = String::with_capacity(hex_digits);
        for d in (0..hex_digits).rev() {
            let bit_off = (d * 4) as u32;
            let word_idx = (bit_off / 64) as usize;
            let shift = bit_off % 64;
            let nibble = ((self.words[word_idx] >> shift) & 0xF) as u8;
            let ch = match nibble {
                0..=9 => (b'0' + nibble) as char,
                10..=15 => (b'a' + (nibble - 10)) as char,
                _ => unreachable!(),
            };
            s.push(ch);
        }
        s
    }
}

impl Display for BitValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "(bv {} #x{})", self.width, self.to_hex_string())
    }
}

/// Identifies an input acquisition point.
///
/// Distinct `place_id`, `memory_version`, or `eval_order` values represent
/// distinct runtime reads or calls and MUST NEVER be merged solely by
/// syntactic name or bit width.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct InputIdentity {
    pub name: String,
    pub place_id: u64,
    pub memory_version: u64,
    pub eval_order: u32,
}

impl InputIdentity {
    /// Constructs a full input identity with explicit place, memory version, and order.
    pub fn new(
        name: impl Into<String>,
        place_id: u64,
        memory_version: u64,
        eval_order: u32,
    ) -> Self {
        Self {
            name: name.into(),
            place_id,
            memory_version,
            eval_order,
        }
    }

    /// Convenience constructor for a pure version-0 input value.
    pub fn pure(name: impl Into<String>, place_id: u64) -> Self {
        Self::new(name, place_id, 0, place_id as u32)
    }
}

impl Display for InputIdentity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(input-id (name {}) (place {}) (mem-version {}) (eval-order {}))",
            Quoted(&self.name),
            self.place_id,
            self.memory_version,
            self.eval_order
        )
    }
}

/// Starting bit index for an [`Node::Extract`] slice.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ExtractStart {
    /// Static compile-time bit offset.
    Const(u32),
    /// Runtime dynamic bit offset produced by another DAG node.
    Dynamic(NodeId),
}

impl From<u32> for ExtractStart {
    fn from(value: u32) -> Self {
        Self::Const(value)
    }
}

impl Display for ExtractStart {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Const(offset) => write!(f, "(const-start {offset})"),
            Self::Dynamic(node) => write!(f, "(dynamic-start {node})"),
        }
    }
}

/// Pure finite-DAG BitVec IR node.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Node {
    Input {
        width: u32,
        identity: InputIdentity,
    },
    Constant {
        width: u32,
        bits: BitValue,
    },
    Concat(NodeId, NodeId),
    Extract {
        input: NodeId,
        start: ExtractStart,
        width: u32,
    },
    And(NodeId, NodeId),
    Or(NodeId, NodeId),
    Not(NodeId),
    Equal(NodeId, NodeId),
    ZeroExtend {
        input: NodeId,
        width: u32,
    },
}

impl Display for Node {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Input { width, identity } => {
                write!(f, "(input (width {width}) {identity})")
            }
            Self::Constant { width, bits } => {
                write!(f, "(constant (width {width}) (bits {bits}))")
            }
            Self::Concat(left, right) => write!(f, "(concat {left} {right})"),
            Self::Extract {
                input,
                start,
                width,
            } => write!(f, "(extract {input} {start} (width {width}))"),
            Self::And(left, right) => write!(f, "(and {left} {right})"),
            Self::Or(left, right) => write!(f, "(or {left} {right})"),
            Self::Not(input) => write!(f, "(not {input})"),
            Self::Equal(left, right) => write!(f, "(equal {left} {right})"),
            Self::ZeroExtend { input, width } => {
                write!(f, "(zero-extend {input} (width {width}))")
            }
        }
    }
}

/// Unsigned comparison operators used in [`DomainConstraint`] preconditions.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum CmpOp {
    Eq,
    Ne,
    Ult,
    Ule,
    Ugt,
    Uge,
}

impl Display for CmpOp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Eq => "bv-eq",
            Self::Ne => "bv-ne",
            Self::Ult => "bv-ult",
            Self::Ule => "bv-ule",
            Self::Ugt => "bv-ugt",
            Self::Uge => "bv-uge",
        })
    }
}

/// Symbolic bit-vector expression inside [`DomainConstraint`] and bounds checks.
///
/// Expressed over [`InputIdentity`] rather than DAG [`NodeId`]s so constraints
/// remain invariant across DAG rewrites and zero-width elimination.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DomainTerm {
    Input {
        identity: InputIdentity,
        width: u32,
    },
    Constant(BitValue),
    Extract {
        term: Box<DomainTerm>,
        start: u32,
        width: u32,
    },
    Concat(Box<DomainTerm>, Box<DomainTerm>),
    And(Box<DomainTerm>, Box<DomainTerm>),
    Or(Box<DomainTerm>, Box<DomainTerm>),
    Not(Box<DomainTerm>),
    ZeroExtend {
        term: Box<DomainTerm>,
        width: u32,
    },
}

impl DomainTerm {
    /// Convenience constructor referencing a region input.
    pub fn input(identity: InputIdentity, width: u32) -> Self {
        Self::Input { identity, width }
    }

    /// Computes and validates the bit width of this term.
    pub fn width(&self) -> Result<u32, IrError> {
        match self {
            Self::Input { width, .. } => Ok(*width),
            Self::Constant(bits) => Ok(bits.width()),
            Self::Extract { term, start, width } => {
                let in_w = term.width()?;
                let end = start.checked_add(*width).ok_or(IrError::SliceOutOfBounds {
                    input_width: in_w,
                    start: *start,
                    slice_width: *width,
                })?;
                if end > in_w {
                    return Err(IrError::SliceOutOfBounds {
                        input_width: in_w,
                        start: *start,
                        slice_width: *width,
                    });
                }
                Ok(*width)
            }
            Self::Concat(hi, lo) => {
                let hw = hi.width()?;
                let lw = lo.width()?;
                hw.checked_add(lw).ok_or(IrError::WidthOverflow {
                    left: hw,
                    right: lw,
                })
            }
            Self::And(l, r) => {
                let lw = l.width()?;
                let rw = r.width()?;
                if lw != rw {
                    return Err(IrError::WidthMismatch {
                        op: "domain-and",
                        left: lw,
                        right: rw,
                    });
                }
                Ok(lw)
            }
            Self::Or(l, r) => {
                let lw = l.width()?;
                let rw = r.width()?;
                if lw != rw {
                    return Err(IrError::WidthMismatch {
                        op: "domain-or",
                        left: lw,
                        right: rw,
                    });
                }
                Ok(lw)
            }
            Self::Not(inner) => inner.width(),
            Self::ZeroExtend { term, width } => {
                let in_w = term.width()?;
                if *width < in_w {
                    return Err(IrError::InvalidZeroExtend {
                        input_width: in_w,
                        target_width: *width,
                    });
                }
                Ok(*width)
            }
        }
    }

    /// Evaluates this term on concrete input assignments.
    pub fn eval(&self, env: &BTreeMap<InputIdentity, BitValue>) -> Result<BitValue, IrError> {
        match self {
            Self::Input { identity, width } => {
                let val = env
                    .get(identity)
                    .ok_or_else(|| IrError::MissingInput(identity.clone()))?;
                if val.width() != *width {
                    return Err(IrError::InputWidthMismatch {
                        identity: identity.clone(),
                        expected: *width,
                        actual: val.width(),
                    });
                }
                Ok(val.clone())
            }
            Self::Constant(bits) => Ok(bits.clone()),
            Self::Extract { term, start, width } => term.eval(env)?.extract(*start, *width),
            Self::Concat(hi, lo) => hi.eval(env)?.concat(&lo.eval(env)?),
            Self::And(l, r) => l.eval(env)?.bit_and(&r.eval(env)?),
            Self::Or(l, r) => l.eval(env)?.bit_or(&r.eval(env)?),
            Self::Not(inner) => Ok(inner.eval(env)?.bit_not()),
            Self::ZeroExtend { term, width } => term.eval(env)?.zero_extend(*width),
        }
    }

    /// Collects all referenced input identities deterministically.
    pub fn collect_inputs(&self, out: &mut BTreeMap<InputIdentity, u32>) -> Result<(), IrError> {
        match self {
            Self::Input { identity, width } => {
                if let Some(&prev) = out.get(identity)
                    && prev != *width
                {
                    return Err(IrError::ConflictingInputWidth {
                        identity: identity.clone(),
                        expected: prev,
                        actual: *width,
                    });
                }
                out.insert(identity.clone(), *width);
                Ok(())
            }
            Self::Constant(_) => Ok(()),
            Self::Extract { term, .. } | Self::Not(term) | Self::ZeroExtend { term, .. } => {
                term.collect_inputs(out)
            }
            Self::Concat(a, b) | Self::And(a, b) | Self::Or(a, b) => {
                a.collect_inputs(out)?;
                b.collect_inputs(out)
            }
        }
    }

    /// Alpha-renames input identities according to `rename`.
    pub fn canonicalize(&self, rename: &BTreeMap<InputIdentity, InputIdentity>) -> Self {
        match self {
            Self::Input { identity, width } => Self::Input {
                identity: rename
                    .get(identity)
                    .cloned()
                    .unwrap_or_else(|| identity.clone()),
                width: *width,
            },
            Self::Constant(bits) => Self::Constant(bits.clone()),
            Self::Extract { term, start, width } => Self::Extract {
                term: Box::new(term.canonicalize(rename)),
                start: *start,
                width: *width,
            },
            Self::Concat(hi, lo) => Self::Concat(
                Box::new(hi.canonicalize(rename)),
                Box::new(lo.canonicalize(rename)),
            ),
            Self::And(l, r) => Self::And(
                Box::new(l.canonicalize(rename)),
                Box::new(r.canonicalize(rename)),
            ),
            Self::Or(l, r) => Self::Or(
                Box::new(l.canonicalize(rename)),
                Box::new(r.canonicalize(rename)),
            ),
            Self::Not(inner) => Self::Not(Box::new(inner.canonicalize(rename))),
            Self::ZeroExtend { term, width } => Self::ZeroExtend {
                term: Box::new(term.canonicalize(rename)),
                width: *width,
            },
        }
    }
}

impl Display for DomainTerm {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Input { identity, width } => {
                write!(f, "(term-input (width {width}) {identity})")
            }
            Self::Constant(bits) => write!(f, "(term-const {bits})"),
            Self::Extract { term, start, width } => {
                write!(f, "(term-extract {term} (start {start}) (width {width}))")
            }
            Self::Concat(hi, lo) => write!(f, "(term-concat {hi} {lo})"),
            Self::And(l, r) => write!(f, "(term-and {l} {r})"),
            Self::Or(l, r) => write!(f, "(term-or {l} {r})"),
            Self::Not(inner) => write!(f, "(term-not {inner})"),
            Self::ZeroExtend { term, width } => {
                write!(f, "(term-zext {term} (width {width}))")
            }
        }
    }
}

/// Explicit input domain predicate `P(x)` constraining valid inputs, bounds, and encodings.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DomainConstraint {
    True,
    False,
    UnsignedCmp {
        lhs: DomainTerm,
        op: CmpOp,
        rhs: DomainTerm,
    },
    InSet {
        term: DomainTerm,
        allowed: BTreeSet<BitValue>,
    },
    And(Vec<DomainConstraint>),
    Or(Vec<DomainConstraint>),
    Not(Box<DomainConstraint>),
}

impl DomainConstraint {
    /// Convenience constructor for `lhs <= rhs` (unsigned).
    pub fn ule(lhs: DomainTerm, rhs: DomainTerm) -> Self {
        Self::UnsignedCmp {
            lhs,
            op: CmpOp::Ule,
            rhs,
        }
    }

    /// Convenience constructor for `lhs < rhs` (unsigned).
    pub fn ult(lhs: DomainTerm, rhs: DomainTerm) -> Self {
        Self::UnsignedCmp {
            lhs,
            op: CmpOp::Ult,
            rhs,
        }
    }

    /// Convenience constructor for `lhs == rhs`.
    pub fn eq_term(lhs: DomainTerm, rhs: DomainTerm) -> Self {
        Self::UnsignedCmp {
            lhs,
            op: CmpOp::Eq,
            rhs,
        }
    }

    /// Validates all widths and term invariants inside this domain constraint.
    pub fn validate(&self) -> Result<(), IrError> {
        match self {
            Self::True | Self::False => Ok(()),
            Self::UnsignedCmp { lhs, op, rhs } => {
                let lw = lhs.width()?;
                let rw = rhs.width()?;
                if lw != rw {
                    return Err(IrError::WidthMismatch {
                        op: match op {
                            CmpOp::Eq => "domain-eq",
                            CmpOp::Ne => "domain-ne",
                            CmpOp::Ult => "domain-ult",
                            CmpOp::Ule => "domain-ule",
                            CmpOp::Ugt => "domain-ugt",
                            CmpOp::Uge => "domain-uge",
                        },
                        left: lw,
                        right: rw,
                    });
                }
                Ok(())
            }
            Self::InSet { term, allowed } => {
                let tw = term.width()?;
                for val in allowed {
                    if val.width() != tw {
                        return Err(IrError::WidthMismatch {
                            op: "domain-in-set",
                            left: tw,
                            right: val.width(),
                        });
                    }
                }
                Ok(())
            }
            Self::And(items) | Self::Or(items) => {
                for item in items {
                    item.validate()?;
                }
                Ok(())
            }
            Self::Not(inner) => inner.validate(),
        }
    }

    /// Evaluates `P(x)` on concrete input assignments.
    pub fn eval(&self, env: &BTreeMap<InputIdentity, BitValue>) -> Result<bool, IrError> {
        match self {
            Self::True => Ok(true),
            Self::False => Ok(false),
            Self::UnsignedCmp { lhs, op, rhs } => {
                let lv = lhs.eval(env)?;
                let rv = rhs.eval(env)?;
                if lv.width() != rv.width() {
                    return Err(IrError::WidthMismatch {
                        op: "domain-cmp",
                        left: lv.width(),
                        right: rv.width(),
                    });
                }
                let ord = lv.cmp_unsigned(&rv);
                Ok(match op {
                    CmpOp::Eq => ord == Ordering::Equal,
                    CmpOp::Ne => ord != Ordering::Equal,
                    CmpOp::Ult => ord == Ordering::Less,
                    CmpOp::Ule => ord != Ordering::Greater,
                    CmpOp::Ugt => ord == Ordering::Greater,
                    CmpOp::Uge => ord != Ordering::Less,
                })
            }
            Self::InSet { term, allowed } => {
                let tv = term.eval(env)?;
                Ok(allowed.contains(&tv))
            }
            Self::And(items) => {
                for item in items {
                    if !item.eval(env)? {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
            Self::Or(items) => {
                for item in items {
                    if item.eval(env)? {
                        return Ok(true);
                    }
                }
                Ok(false)
            }
            Self::Not(inner) => Ok(!inner.eval(env)?),
        }
    }

    /// Collects all inputs referenced anywhere in this domain constraint.
    pub fn collect_inputs(&self, out: &mut BTreeMap<InputIdentity, u32>) -> Result<(), IrError> {
        match self {
            Self::True | Self::False => Ok(()),
            Self::UnsignedCmp { lhs, rhs, .. } => {
                lhs.collect_inputs(out)?;
                rhs.collect_inputs(out)
            }
            Self::InSet { term, .. } => term.collect_inputs(out),
            Self::And(items) | Self::Or(items) => {
                for item in items {
                    item.collect_inputs(out)?;
                }
                Ok(())
            }
            Self::Not(inner) => inner.collect_inputs(out),
        }
    }

    /// Produces a deterministic alpha-renamed copy of this domain constraint.
    pub fn canonicalize(&self, rename: &BTreeMap<InputIdentity, InputIdentity>) -> Self {
        match self {
            Self::True => Self::True,
            Self::False => Self::False,
            Self::UnsignedCmp { lhs, op, rhs } => Self::UnsignedCmp {
                lhs: lhs.canonicalize(rename),
                op: *op,
                rhs: rhs.canonicalize(rename),
            },
            Self::InSet { term, allowed } => Self::InSet {
                term: term.canonicalize(rename),
                allowed: allowed.clone(),
            },
            Self::And(items) => {
                let mut normalized: Vec<Self> = items
                    .iter()
                    .map(|c| c.canonicalize(rename))
                    .filter(|c| *c != Self::True)
                    .collect();
                if normalized.contains(&Self::False) {
                    return Self::False;
                }
                normalized.sort();
                normalized.dedup();
                match normalized.len() {
                    0 => Self::True,
                    1 => normalized.pop().expect("len is 1"),
                    _ => Self::And(normalized),
                }
            }
            Self::Or(items) => {
                let mut normalized: Vec<Self> = items
                    .iter()
                    .map(|c| c.canonicalize(rename))
                    .filter(|c| *c != Self::False)
                    .collect();
                if normalized.contains(&Self::True) {
                    return Self::True;
                }
                normalized.sort();
                normalized.dedup();
                match normalized.len() {
                    0 => Self::False,
                    1 => normalized.pop().expect("len is 1"),
                    _ => Self::Or(normalized),
                }
            }
            Self::Not(inner) => Self::Not(Box::new(inner.canonicalize(rename))),
        }
    }
}

impl Display for DomainConstraint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::True => f.write_str("(domain-true)"),
            Self::False => f.write_str("(domain-false)"),
            Self::UnsignedCmp { lhs, op, rhs } => {
                write!(f, "({op} {lhs} {rhs})")
            }
            Self::InSet { term, allowed } => {
                f.write_str("(domain-in-set ")?;
                term.fmt(f)?;
                f.write_str(" (")?;
                for (idx, val) in allowed.iter().enumerate() {
                    if idx != 0 {
                        f.write_char(' ')?;
                    }
                    val.fmt(f)?;
                }
                f.write_str("))")
            }
            Self::And(items) => {
                f.write_str("(domain-and ")?;
                write_list(f, items)?;
                f.write_char(')')
            }
            Self::Or(items) => {
                f.write_str("(domain-or ")?;
                write_list(f, items)?;
                f.write_char(')')
            }
            Self::Not(inner) => write!(f, "(domain-not {inner})"),
        }
    }
}

/// Observable effect classification of a boundary input evaluation.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum InputEffect {
    /// Side-effect-free value parameter or pure snapshot.
    PureValue,
    /// Snapshot read from memory (`value'bitvec`) at a specific version.
    MemoryRead { place_id: u64, memory_version: u64 },
    /// Side-effecting call producing a BitVec value (must run exactly once in order).
    SideEffectCall { symbol: String, call_id: u64 },
}

impl Display for InputEffect {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PureValue => f.write_str("(effect-pure)"),
            Self::MemoryRead {
                place_id,
                memory_version,
            } => write!(
                f,
                "(effect-mem-read (place {place_id}) (version {memory_version}))"
            ),
            Self::SideEffectCall { symbol, call_id } => write!(
                f,
                "(effect-call (symbol {}) (call-id {call_id}))",
                Quoted(symbol)
            ),
        }
    }
}

/// Ordered input acquisition record on a region's [`BehaviorBoundary`].
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BoundaryInput {
    pub identity: InputIdentity,
    pub width: u32,
    pub effect: InputEffect,
}

impl Display for BoundaryInput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(boundary-input {} (width {}) {})",
            self.identity, self.width, self.effect
        )
    }
}

/// Runtime check that must execute at the region boundary before pure bit ops.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum BoundaryCheck {
    /// Dynamic slice range check: requires `index_term + extract_width <= input_width`
    /// without integer overflow, even when `extract_width == 0`.
    DynamicBoundsCheck {
        index_term: DomainTerm,
        extract_width: u32,
        input_width: u32,
        check_order: u32,
    },
    /// General runtime precondition check preserved across pure rewrites.
    PreconditionCheck {
        label: String,
        condition: DomainConstraint,
        check_order: u32,
    },
}

impl BoundaryCheck {
    /// Returns the ordering sequence number of this check.
    pub fn check_order(&self) -> u32 {
        match self {
            Self::DynamicBoundsCheck { check_order, .. }
            | Self::PreconditionCheck { check_order, .. } => *check_order,
        }
    }

    /// Evaluates the check on concrete boundary inputs, returning `Ok(true)` if passed.
    pub fn eval(&self, env: &BTreeMap<InputIdentity, BitValue>) -> Result<bool, IrError> {
        match self {
            Self::DynamicBoundsCheck {
                index_term,
                extract_width,
                input_width,
                ..
            } => {
                let idx_val = index_term.eval(env)?;
                if *extract_width > *input_width {
                    return Ok(false);
                }
                let max_start = u64::from(*input_width - *extract_width);
                let max_start_bv = BitValue::from_u64(idx_val.width().max(64), max_start)?;
                Ok(idx_val.cmp_unsigned(&max_start_bv) != Ordering::Greater)
            }
            Self::PreconditionCheck { condition, .. } => condition.eval(env),
        }
    }

    /// Alpha-renames inputs inside this check.
    pub fn canonicalize(&self, rename: &BTreeMap<InputIdentity, InputIdentity>) -> Self {
        match self {
            Self::DynamicBoundsCheck {
                index_term,
                extract_width,
                input_width,
                check_order,
            } => Self::DynamicBoundsCheck {
                index_term: index_term.canonicalize(rename),
                extract_width: *extract_width,
                input_width: *input_width,
                check_order: *check_order,
            },
            Self::PreconditionCheck {
                label,
                condition,
                check_order,
            } => Self::PreconditionCheck {
                label: label.clone(),
                condition: condition.canonicalize(rename),
                check_order: *check_order,
            },
        }
    }
}

impl Display for BoundaryCheck {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DynamicBoundsCheck {
                index_term,
                extract_width,
                input_width,
                check_order,
            } => write!(
                f,
                "(dynamic-bounds-check (order {check_order}) (index {index_term}) (extract-width {extract_width}) (input-width {input_width}))"
            ),
            Self::PreconditionCheck {
                label,
                condition,
                check_order,
            } => write!(
                f,
                "(precondition-check (order {check_order}) (label {}) (condition {condition}))",
                Quoted(label)
            ),
        }
    }
}

/// Ordered resource cleanup / drop action preserved across pure bit optimizations.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CleanupAction {
    pub cleanup_order: u32,
    pub resource_id: u64,
    pub symbol: String,
}

impl Display for CleanupAction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(cleanup (order {}) (resource-id {}) (symbol {}))",
            self.cleanup_order,
            self.resource_id,
            Quoted(&self.symbol)
        )
    }
}

/// Explicit boundary contract preserving input evaluation order, observable
/// side effects, dynamic bounds checks, and cleanup ordering.
///
/// Pure DAG rewrites, zero-width elimination, constant folding, and synthesis
/// may simplify the value mapping inside a [`Region`], but MUST preserve this
/// boundary intact.
#[derive(Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BehaviorBoundary {
    pub inputs: Vec<BoundaryInput>,
    pub checks: Vec<BoundaryCheck>,
    pub cleanups: Vec<CleanupAction>,
}

impl BehaviorBoundary {
    /// Returns `true` if any observable effect, check, or cleanup exists on the boundary.
    pub fn has_observable_behavior(&self) -> bool {
        !self.checks.is_empty()
            || !self.cleanups.is_empty()
            || self
                .inputs
                .iter()
                .any(|item| !matches!(item.effect, InputEffect::PureValue))
    }

    /// Verifies that `other` preserves the exact boundary contract of `self`.
    pub fn matches_contract(&self, other: &Self) -> bool {
        self == other
    }

    /// Produces an alpha-renamed boundary preserving effect kinds, orders, checks, and cleanups.
    pub fn canonicalize(&self, rename: &BTreeMap<InputIdentity, InputIdentity>) -> Self {
        let inputs = self
            .inputs
            .iter()
            .map(|inp| BoundaryInput {
                identity: rename
                    .get(&inp.identity)
                    .cloned()
                    .unwrap_or_else(|| inp.identity.clone()),
                width: inp.width,
                effect: inp.effect.clone(),
            })
            .collect();
        let checks = self
            .checks
            .iter()
            .map(|chk| chk.canonicalize(rename))
            .collect();
        Self {
            inputs,
            checks,
            cleanups: self.cleanups.clone(),
        }
    }
}

impl Display for BehaviorBoundary {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("(behavior-boundary (inputs ")?;
        write_list(f, &self.inputs)?;
        f.write_str(") (checks ")?;
        write_list(f, &self.checks)?;
        f.write_str(") (cleanups ")?;
        write_list(f, &self.cleanups)?;
        f.write_str("))")
    }
}

/// Output type classification of a BitVec [`Region`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum OutputKind {
    /// `BitVec<width>` (including `width == 0`).
    BitVec(u32),
    /// Boolean result (from `==` / `!=`, encoded on 1 bit).
    Bool,
}

impl OutputKind {
    /// Underlying bit width (`1` for `Bool`).
    pub fn bit_width(self) -> u32 {
        match self {
            Self::BitVec(w) => w,
            Self::Bool => 1,
        }
    }
}

impl Display for OutputKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BitVec(w) => write!(f, "(output-bitvec {w})"),
            Self::Bool => f.write_str("(output-bool)"),
        }
    }
}

/// Complete BitVec optimization/synthesis region combining inputs, domain constraint
/// `P(x)`, hash-consed pure DAG nodes, root output, and [`BehaviorBoundary`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Region {
    pub inputs: BTreeMap<InputIdentity, u32>,
    pub domain: DomainConstraint,
    pub nodes: Vec<Node>,
    pub root: NodeId,
    pub output_kind: OutputKind,
    pub boundary: BehaviorBoundary,
}

impl Region {
    /// Validates all DAG edges, topological ordering, width rules, domain, and boundary.
    pub fn validate(&self) -> Result<(), IrError> {
        let mut widths = Vec::with_capacity(self.nodes.len());
        let mut seen_inputs: BTreeMap<InputIdentity, u32> = BTreeMap::new();

        for (idx, node) in self.nodes.iter().enumerate() {
            let cur = NodeId(idx);
            let w = match node {
                Node::Input { width, identity } => {
                    if let Some(&prev) = seen_inputs.get(identity)
                        && prev != *width
                    {
                        return Err(IrError::ConflictingInputWidth {
                            identity: identity.clone(),
                            expected: prev,
                            actual: *width,
                        });
                    }
                    seen_inputs.insert(identity.clone(), *width);
                    if let Some(&declared) = self.inputs.get(identity)
                        && declared != *width
                    {
                        return Err(IrError::ConflictingInputWidth {
                            identity: identity.clone(),
                            expected: declared,
                            actual: *width,
                        });
                    }
                    *width
                }
                Node::Constant { width, bits } => {
                    if bits.width() != *width {
                        return Err(IrError::WidthMismatch {
                            op: "constant",
                            left: *width,
                            right: bits.width(),
                        });
                    }
                    *width
                }
                Node::Concat(hi, lo) => {
                    let hw = checked_prior_width(&widths, cur, *hi)?;
                    let lw = checked_prior_width(&widths, cur, *lo)?;
                    hw.checked_add(lw).ok_or(IrError::WidthOverflow {
                        left: hw,
                        right: lw,
                    })?
                }
                Node::Extract {
                    input,
                    start,
                    width,
                } => {
                    let in_w = checked_prior_width(&widths, cur, *input)?;
                    match start {
                        ExtractStart::Const(lo) => {
                            let end = lo.checked_add(*width).ok_or(IrError::SliceOutOfBounds {
                                input_width: in_w,
                                start: *lo,
                                slice_width: *width,
                            })?;
                            if end > in_w {
                                return Err(IrError::SliceOutOfBounds {
                                    input_width: in_w,
                                    start: *lo,
                                    slice_width: *width,
                                });
                            }
                        }
                        ExtractStart::Dynamic(idx_node) => {
                            let idx_w = checked_prior_width(&widths, cur, *idx_node)?;
                            if idx_w == 0 {
                                return Err(IrError::ZeroWidthDynamicIndex);
                            }
                        }
                    }
                    *width
                }
                Node::And(l, r) => {
                    let lw = checked_prior_width(&widths, cur, *l)?;
                    let rw = checked_prior_width(&widths, cur, *r)?;
                    if lw != rw {
                        return Err(IrError::WidthMismatch {
                            op: "and",
                            left: lw,
                            right: rw,
                        });
                    }
                    lw
                }
                Node::Or(l, r) => {
                    let lw = checked_prior_width(&widths, cur, *l)?;
                    let rw = checked_prior_width(&widths, cur, *r)?;
                    if lw != rw {
                        return Err(IrError::WidthMismatch {
                            op: "or",
                            left: lw,
                            right: rw,
                        });
                    }
                    lw
                }
                Node::Not(inner) => checked_prior_width(&widths, cur, *inner)?,
                Node::Equal(l, r) => {
                    let lw = checked_prior_width(&widths, cur, *l)?;
                    let rw = checked_prior_width(&widths, cur, *r)?;
                    if lw != rw {
                        return Err(IrError::WidthMismatch {
                            op: "equal",
                            left: lw,
                            right: rw,
                        });
                    }
                    1
                }
                Node::ZeroExtend { input, width } => {
                    let in_w = checked_prior_width(&widths, cur, *input)?;
                    if *width < in_w {
                        return Err(IrError::InvalidZeroExtend {
                            input_width: in_w,
                            target_width: *width,
                        });
                    }
                    *width
                }
            };
            widths.push(w);
        }

        if self.root.0 >= self.nodes.len() {
            return Err(IrError::InvalidNodeId(self.root));
        }
        let root_w = widths[self.root.0];
        if root_w != self.output_kind.bit_width() {
            return Err(IrError::InvalidOutputKind {
                root_width: root_w,
                expected: self.output_kind,
            });
        }

        self.domain.validate()?;
        let mut dom_inputs = BTreeMap::new();
        self.domain.collect_inputs(&mut dom_inputs)?;
        for (id, w) in dom_inputs {
            if let Some(&declared) = self.inputs.get(&id)
                && declared != w
            {
                return Err(IrError::ConflictingInputWidth {
                    identity: id,
                    expected: declared,
                    actual: w,
                });
            }
        }
        for b_in in &self.boundary.inputs {
            if let Some(&declared) = self.inputs.get(&b_in.identity)
                && declared != b_in.width
            {
                return Err(IrError::ConflictingInputWidth {
                    identity: b_in.identity.clone(),
                    expected: declared,
                    actual: b_in.width,
                });
            }
        }
        for chk in &self.boundary.checks {
            if let BoundaryCheck::PreconditionCheck { condition, .. } = chk {
                condition.validate()?;
            } else if let BoundaryCheck::DynamicBoundsCheck { index_term, .. } = chk {
                let idx_w = index_term.width()?;
                if idx_w == 0 {
                    return Err(IrError::ZeroWidthDynamicIndex);
                }
            }
        }

        Ok(())
    }

    /// Computes the bit width of every node in `self.nodes`.
    pub fn node_widths(&self) -> Result<Vec<u32>, IrError> {
        let mut widths = Vec::with_capacity(self.nodes.len());
        for (idx, node) in self.nodes.iter().enumerate() {
            let cur = NodeId(idx);
            let w = match node {
                Node::Input { width, .. } | Node::Constant { width, .. } => *width,
                Node::Concat(hi, lo) => {
                    let hw = checked_prior_width(&widths, cur, *hi)?;
                    let lw = checked_prior_width(&widths, cur, *lo)?;
                    hw.checked_add(lw).ok_or(IrError::WidthOverflow {
                        left: hw,
                        right: lw,
                    })?
                }
                Node::Extract { width, .. } => *width,
                Node::And(l, r) | Node::Or(l, r) => {
                    let lw = checked_prior_width(&widths, cur, *l)?;
                    let _ = checked_prior_width(&widths, cur, *r)?;
                    lw
                }
                Node::Not(inner) => checked_prior_width(&widths, cur, *inner)?,
                Node::Equal(_, _) => 1,
                Node::ZeroExtend { width, .. } => *width,
            };
            widths.push(w);
        }
        Ok(widths)
    }

    /// Returns the bit width of a specific `NodeId`.
    pub fn node_width(&self, id: NodeId) -> Result<u32, IrError> {
        let widths = self.node_widths()?;
        widths.get(id.0).copied().ok_or(IrError::InvalidNodeId(id))
    }

    /// Produces a deterministic, hash-consed canonical representation suitable for cache keys.
    ///
    /// Distinct memory versions, places, and evaluation orders receive distinct
    /// canonical input slots, guaranteeing that unrelated memory versions are
    /// never conflated while structurally alpha-equivalent pure regions match.
    pub fn canonicalize(&self) -> Result<CanonicalRegion, IrError> {
        Ok(self.canonicalize_with_map()?.0)
    }

    /// Canonicalizes this region and also returns the `canonical -> original` input map.
    pub fn canonicalize_with_map(
        &self,
    ) -> Result<(CanonicalRegion, BTreeMap<InputIdentity, InputIdentity>), IrError> {
        self.validate()?;
        let mut reachable = Vec::new();
        let mut visited = vec![false; self.nodes.len()];
        collect_postorder(&self.nodes, self.root, &mut visited, &mut reachable);

        // Assign canonical slots in deterministic order:
        // 1. boundary input acquisition order,
        // 2. reachable DAG input encounter order,
        // 3. remaining declared/domain inputs in BTreeMap order.
        let mut ordered_inputs: Vec<(InputIdentity, u32)> = Vec::new();
        let mut seen_ids: BTreeSet<InputIdentity> = BTreeSet::new();
        for b_in in &self.boundary.inputs {
            if seen_ids.insert(b_in.identity.clone()) {
                ordered_inputs.push((b_in.identity.clone(), b_in.width));
            }
        }
        for &nid in &reachable {
            if let Node::Input { width, identity } = &self.nodes[nid.0]
                && seen_ids.insert(identity.clone())
            {
                ordered_inputs.push((identity.clone(), *width));
            }
        }
        for (id, &w) in &self.inputs {
            if seen_ids.insert(id.clone()) {
                ordered_inputs.push((id.clone(), w));
            }
        }
        let mut dom_inputs = BTreeMap::new();
        self.domain.collect_inputs(&mut dom_inputs)?;
        for (id, w) in dom_inputs {
            if seen_ids.insert(id.clone()) {
                ordered_inputs.push((id, w));
            }
        }

        let mut rename = BTreeMap::new();
        let mut canon_to_orig = BTreeMap::new();
        let mut canon_inputs = BTreeMap::new();
        for (slot, (orig, width)) in ordered_inputs.into_iter().enumerate() {
            let canon_id = InputIdentity {
                name: format!("in{slot}"),
                place_id: slot as u64,
                memory_version: 0,
                eval_order: slot as u32,
            };
            canon_to_orig.insert(canon_id.clone(), orig.clone());
            rename.insert(orig, canon_id.clone());
            canon_inputs.insert(canon_id, width);
        }

        let mut id_map = vec![NodeId(0); self.nodes.len()];
        let mut canon_nodes = Vec::with_capacity(reachable.len());
        let mut intern_table: BTreeMap<Node, NodeId> = BTreeMap::new();

        for orig_id in reachable {
            let remapped = match &self.nodes[orig_id.0] {
                Node::Input { width, identity } => Node::Input {
                    width: *width,
                    identity: rename
                        .get(identity)
                        .cloned()
                        .unwrap_or_else(|| identity.clone()),
                },
                Node::Constant { width, bits } => Node::Constant {
                    width: *width,
                    bits: bits.clone(),
                },
                Node::Concat(l, r) => Node::Concat(id_map[l.0], id_map[r.0]),
                Node::Extract {
                    input,
                    start,
                    width,
                } => Node::Extract {
                    input: id_map[input.0],
                    start: match start {
                        ExtractStart::Const(k) => ExtractStart::Const(*k),
                        ExtractStart::Dynamic(d) => ExtractStart::Dynamic(id_map[d.0]),
                    },
                    width: *width,
                },
                Node::And(l, r) => {
                    let a = id_map[l.0];
                    let b = id_map[r.0];
                    if a <= b {
                        Node::And(a, b)
                    } else {
                        Node::And(b, a)
                    }
                }
                Node::Or(l, r) => {
                    let a = id_map[l.0];
                    let b = id_map[r.0];
                    if a <= b {
                        Node::Or(a, b)
                    } else {
                        Node::Or(b, a)
                    }
                }
                Node::Not(inner) => Node::Not(id_map[inner.0]),
                Node::Equal(l, r) => {
                    let a = id_map[l.0];
                    let b = id_map[r.0];
                    if a <= b {
                        Node::Equal(a, b)
                    } else {
                        Node::Equal(b, a)
                    }
                }
                Node::ZeroExtend { input, width } => Node::ZeroExtend {
                    input: id_map[input.0],
                    width: *width,
                },
            };

            let new_id = if let Some(&existing) = intern_table.get(&remapped) {
                existing
            } else {
                let nid = NodeId(canon_nodes.len());
                intern_table.insert(remapped.clone(), nid);
                canon_nodes.push(remapped);
                nid
            };
            id_map[orig_id.0] = new_id;
        }

        let region = Region {
            inputs: canon_inputs,
            domain: self.domain.canonicalize(&rename),
            nodes: canon_nodes,
            root: id_map[self.root.0],
            output_kind: self.output_kind,
            boundary: self.boundary.canonicalize(&rename),
        };
        region.validate()?;
        Ok((CanonicalRegion { region }, canon_to_orig))
    }
}

impl Display for Region {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("(bitvec-region (inputs (")?;
        for (idx, (id, w)) in self.inputs.iter().enumerate() {
            if idx != 0 {
                f.write_char(' ')?;
            }
            write!(f, "(decl {id} (width {w}))")?;
        }
        write!(f, ")) (domain {}) (nodes ", self.domain)?;
        write_list(f, &self.nodes)?;
        write!(
            f,
            ") (root {}) (output {}) (boundary {}))",
            self.root, self.output_kind, self.boundary
        )
    }
}

/// Canonicalized, hash-consed [`Region`] wrapper used as a structural cache key.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CanonicalRegion {
    pub region: Region,
}

impl Ord for CanonicalRegion {
    fn cmp(&self, other: &Self) -> Ordering {
        self.region
            .output_kind
            .cmp(&other.region.output_kind)
            .then_with(|| self.region.inputs.cmp(&other.region.inputs))
            .then_with(|| self.region.domain.cmp(&other.region.domain))
            .then_with(|| self.region.nodes.cmp(&other.region.nodes))
            .then_with(|| self.region.root.cmp(&other.region.root))
            .then_with(|| self.region.boundary.cmp(&other.region.boundary))
    }
}

impl PartialOrd for CanonicalRegion {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl std::hash::Hash for CanonicalRegion {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.region.inputs.hash(state);
        self.region.domain.hash(state);
        self.region.nodes.hash(state);
        self.region.root.hash(state);
        self.region.output_kind.hash(state);
        self.region.boundary.hash(state);
    }
}

impl Display for CanonicalRegion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "(canonical-region {})", self.region)
    }
}

/// Ergonomic DAG builder with hash-consing that preserves distinct memory versions
/// and evaluation orders.
#[derive(Clone, Debug, Default)]
pub struct RegionBuilder {
    inputs: BTreeMap<InputIdentity, u32>,
    nodes: Vec<Node>,
    widths: Vec<u32>,
    intern_table: BTreeMap<Node, NodeId>,
    domain: Vec<DomainConstraint>,
    boundary: BehaviorBoundary,
}

impl RegionBuilder {
    /// Creates an empty [`RegionBuilder`].
    pub fn new() -> Self {
        Self::default()
    }

    fn intern(&mut self, node: Node, width: u32) -> NodeId {
        if let Some(&existing) = self.intern_table.get(&node) {
            return existing;
        }
        let id = NodeId(self.nodes.len());
        self.intern_table.insert(node.clone(), id);
        self.nodes.push(node);
        self.widths.push(width);
        id
    }

    /// Declares an input with an explicit [`InputEffect`] on the [`BehaviorBoundary`].
    pub fn input_with_effect(
        &mut self,
        identity: InputIdentity,
        width: u32,
        effect: InputEffect,
    ) -> Result<NodeId, IrError> {
        if let Some(&prev) = self.inputs.get(&identity)
            && prev != width
        {
            return Err(IrError::ConflictingInputWidth {
                identity,
                expected: prev,
                actual: width,
            });
        }
        let is_new = !self.inputs.contains_key(&identity);
        self.inputs.insert(identity.clone(), width);
        if is_new {
            self.boundary.inputs.push(BoundaryInput {
                identity: identity.clone(),
                width,
                effect,
            });
            self.boundary
                .inputs
                .sort_by_key(|entry| entry.identity.eval_order);
        }
        Ok(self.intern(Node::Input { width, identity }, width))
    }

    /// Convenience helper for adding a pure input value.
    pub fn pure_input(&mut self, name: impl Into<String>, place_id: u64, width: u32) -> NodeId {
        let id = InputIdentity::pure(name, place_id);
        self.input_with_effect(id, width, InputEffect::PureValue)
            .expect("fresh pure input has consistent width")
    }

    /// Adds a constant bit-vector node.
    pub fn constant(&mut self, bits: BitValue) -> NodeId {
        let width = bits.width();
        self.intern(Node::Constant { width, bits }, width)
    }

    /// Adds a checked integer constant node.
    pub fn const_u64(&mut self, width: u32, value: u64) -> Result<NodeId, IrError> {
        Ok(self.constant(BitValue::from_u64(width, value)?))
    }

    /// Concatenates `high ++ low`.
    pub fn concat(&mut self, high: NodeId, low: NodeId) -> Result<NodeId, IrError> {
        let hw = self.node_width(high)?;
        let lw = self.node_width(low)?;
        let total = hw.checked_add(lw).ok_or(IrError::WidthOverflow {
            left: hw,
            right: lw,
        })?;
        Ok(self.intern(Node::Concat(high, low), total))
    }

    /// Static slice `[start .. start + width)`.
    pub fn extract_const(
        &mut self,
        input: NodeId,
        start: u32,
        width: u32,
    ) -> Result<NodeId, IrError> {
        let in_w = self.node_width(input)?;
        let end = start.checked_add(width).ok_or(IrError::SliceOutOfBounds {
            input_width: in_w,
            start,
            slice_width: width,
        })?;
        if end > in_w {
            return Err(IrError::SliceOutOfBounds {
                input_width: in_w,
                start,
                slice_width: width,
            });
        }
        Ok(self.intern(
            Node::Extract {
                input,
                start: ExtractStart::Const(start),
                width,
            },
            width,
        ))
    }

    /// Dynamic slice `[start_node .. start_node + width)` with static `width`.
    pub fn extract_dynamic(
        &mut self,
        input: NodeId,
        start: NodeId,
        width: u32,
    ) -> Result<NodeId, IrError> {
        let _ = self.node_width(input)?;
        let idx_w = self.node_width(start)?;
        if idx_w == 0 {
            return Err(IrError::ZeroWidthDynamicIndex);
        }
        Ok(self.intern(
            Node::Extract {
                input,
                start: ExtractStart::Dynamic(start),
                width,
            },
            width,
        ))
    }

    /// Bitwise AND of two equal-width nodes.
    pub fn and(&mut self, left: NodeId, right: NodeId) -> Result<NodeId, IrError> {
        let lw = self.node_width(left)?;
        let rw = self.node_width(right)?;
        if lw != rw {
            return Err(IrError::WidthMismatch {
                op: "and",
                left: lw,
                right: rw,
            });
        }
        Ok(self.intern(Node::And(left, right), lw))
    }

    /// Bitwise OR of two equal-width nodes.
    pub fn or(&mut self, left: NodeId, right: NodeId) -> Result<NodeId, IrError> {
        let lw = self.node_width(left)?;
        let rw = self.node_width(right)?;
        if lw != rw {
            return Err(IrError::WidthMismatch {
                op: "or",
                left: lw,
                right: rw,
            });
        }
        Ok(self.intern(Node::Or(left, right), lw))
    }

    /// Bitwise NOT flipping all `width` bits of `input`.
    pub fn not(&mut self, input: NodeId) -> Result<NodeId, IrError> {
        let w = self.node_width(input)?;
        Ok(self.intern(Node::Not(input), w))
    }

    /// Bitvector equality (`left == right`), returning 1-bit result.
    pub fn equal(&mut self, left: NodeId, right: NodeId) -> Result<NodeId, IrError> {
        let lw = self.node_width(left)?;
        let rw = self.node_width(right)?;
        if lw != rw {
            return Err(IrError::WidthMismatch {
                op: "equal",
                left: lw,
                right: rw,
            });
        }
        Ok(self.intern(Node::Equal(left, right), 1))
    }

    /// Zero-extension to `target_width >= width(input)`.
    pub fn zero_extend(&mut self, input: NodeId, target_width: u32) -> Result<NodeId, IrError> {
        let in_w = self.node_width(input)?;
        if target_width < in_w {
            return Err(IrError::InvalidZeroExtend {
                input_width: in_w,
                target_width,
            });
        }
        Ok(self.intern(
            Node::ZeroExtend {
                input,
                width: target_width,
            },
            target_width,
        ))
    }

    /// Appends a domain constraint clause (`P(x)`).
    pub fn add_domain(&mut self, constraint: DomainConstraint) -> Result<(), IrError> {
        constraint.validate()?;
        self.domain.push(constraint);
        Ok(())
    }

    /// Appends a runtime check to the [`BehaviorBoundary`].
    pub fn add_boundary_check(&mut self, check: BoundaryCheck) {
        self.boundary.checks.push(check);
        self.boundary.checks.sort_by_key(BoundaryCheck::check_order);
    }

    /// Appends a resource cleanup action to the [`BehaviorBoundary`].
    pub fn add_cleanup(&mut self, cleanup: CleanupAction) {
        self.boundary.cleanups.push(cleanup);
        self.boundary
            .cleanups
            .sort_by_key(|item| item.cleanup_order);
    }

    /// Returns the bit width of an existing node.
    pub fn node_width(&self, id: NodeId) -> Result<u32, IrError> {
        self.widths
            .get(id.0)
            .copied()
            .ok_or(IrError::InvalidNodeId(id))
    }

    /// Finalizes and validates a `BitVec<N>` region.
    pub fn build(self, root: NodeId) -> Result<Region, IrError> {
        let w = self.node_width(root)?;
        self.build_with_kind(root, OutputKind::BitVec(w))
    }

    /// Finalizes and validates a `bool`-returning region (`==` / `!=`).
    pub fn build_bool(self, root: NodeId) -> Result<Region, IrError> {
        self.build_with_kind(root, OutputKind::Bool)
    }

    fn build_with_kind(self, root: NodeId, output_kind: OutputKind) -> Result<Region, IrError> {
        let domain = match self.domain.len() {
            0 => DomainConstraint::True,
            1 => self.domain.into_iter().next().expect("len is 1"),
            _ => DomainConstraint::And(self.domain),
        };
        let region = Region {
            inputs: self.inputs,
            domain,
            nodes: self.nodes,
            root,
            output_kind,
            boundary: self.boundary,
        };
        region.validate()?;
        Ok(region)
    }
}

fn checked_prior_width(widths: &[u32], current: NodeId, child: NodeId) -> Result<u32, IrError> {
    if child.0 >= current.0 {
        return Err(IrError::CyclicDag(child));
    }
    widths
        .get(child.0)
        .copied()
        .ok_or(IrError::InvalidNodeId(child))
}

fn collect_postorder(nodes: &[Node], id: NodeId, visited: &mut [bool], out: &mut Vec<NodeId>) {
    if visited[id.0] {
        return;
    }
    visited[id.0] = true;
    match &nodes[id.0] {
        Node::Input { .. } | Node::Constant { .. } => {}
        Node::Concat(l, r) | Node::And(l, r) | Node::Or(l, r) | Node::Equal(l, r) => {
            collect_postorder(nodes, *l, visited, out);
            collect_postorder(nodes, *r, visited, out);
        }
        Node::Extract { input, start, .. } => {
            collect_postorder(nodes, *input, visited, out);
            if let ExtractStart::Dynamic(idx) = start {
                collect_postorder(nodes, *idx, visited, out);
            }
        }
        Node::Not(inner) | Node::ZeroExtend { input: inner, .. } => {
            collect_postorder(nodes, *inner, visited, out);
        }
    }
    out.push(id);
}
