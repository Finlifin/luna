//! Bit-level exact evaluator (`eval_region`) and machine-word baseline lowering (`BaselineProgram`).
//!
//! All machine-word operations run in `O(ceil(N / W))` word operations for
//! machine word width `W` (default 64, configurable up to 64), separating
//! zero-width, word-aligned, single-word, cross-word, and dynamic-start cases
//! while strictly avoiding shift-by-`W` undefined behavior and clearing unused
//! high padding bits on `Not`.

use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::fmt::{self, Display};

use crate::ir::{
    BehaviorBoundary, BoundaryCheck, CleanupAction, ExtractStart, InputEffect, InputIdentity,
    IrError, Node, Region,
};
pub use crate::ir::{BitValue, top_word_mask};
use crate::sexpr::{Quoted, write_list};

/// Observable runtime trace event emitted in execution order by evaluators.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TraceEvent {
    InputEvaluated {
        identity: InputIdentity,
        effect: InputEffect,
    },
    CheckPassed {
        check: BoundaryCheck,
    },
    CleanupExecuted {
        cleanup: CleanupAction,
    },
}

impl Display for TraceEvent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InputEvaluated { identity, effect } => {
                write!(f, "(trace-input {identity} {effect})")
            }
            Self::CheckPassed { check } => write!(f, "(trace-check-passed {check})"),
            Self::CleanupExecuted { cleanup } => write!(f, "(trace-cleanup {cleanup})"),
        }
    }
}

/// Complete execution result including computed [`BitValue`] and observable boundary trace.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EvalResult {
    pub value: BitValue,
    pub trace: Vec<TraceEvent>,
}

impl Display for EvalResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("(eval-result (value ")?;
        self.value.fmt(f)?;
        f.write_str(") (trace ")?;
        write_list(f, &self.trace)?;
        f.write_str("))")
    }
}

/// Runtime or lowering errors for BitVec evaluation and machine-word lowering.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EvalError {
    Ir(IrError),
    DomainViolation,
    DynamicSliceOutOfBounds {
        input_width: u32,
        slice_width: u32,
        start_detail: String,
    },
    BoundaryCheckFailed(BoundaryCheck),
    UnsupportedWordWidth(u32),
    UndefinedTargetOperation(String),
}

impl From<IrError> for EvalError {
    fn from(value: IrError) -> Self {
        Self::Ir(value)
    }
}

impl Display for EvalError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Ir(err) => write!(f, "(eval-error ir {err})"),
            Self::DomainViolation => f.write_str("(eval-error domain-violation)"),
            Self::DynamicSliceOutOfBounds {
                input_width,
                slice_width,
                start_detail,
            } => write!(
                f,
                "(eval-error dynamic-slice-oob (input-width {input_width}) (slice-width {slice_width}) (start {}))",
                Quoted(start_detail)
            ),
            Self::BoundaryCheckFailed(chk) => {
                write!(f, "(eval-error boundary-check-failed {chk})")
            }
            Self::UnsupportedWordWidth(w) => {
                write!(f, "(eval-error unsupported-word-width {w})")
            }
            Self::UndefinedTargetOperation(msg) => {
                write!(f, "(eval-error undefined-target-op {})", Quoted(msg))
            }
        }
    }
}

impl std::error::Error for EvalError {}

/// Executes boundary input acquisition, domain validation, and boundary checks
/// in their exact contract order, returning the initial trace events.
pub(crate) fn run_boundary_prefix(
    boundary: &BehaviorBoundary,
    declared_inputs: &BTreeMap<InputIdentity, u32>,
    inputs: &BTreeMap<InputIdentity, BitValue>,
) -> Result<Vec<TraceEvent>, EvalError> {
    let mut trace = Vec::new();
    let mut seen_boundary = std::collections::BTreeSet::new();

    let mut ordered = boundary.inputs.clone();
    ordered.sort_by_key(|item| item.identity.eval_order);
    for b_in in ordered {
        let val = inputs
            .get(&b_in.identity)
            .ok_or_else(|| IrError::MissingInput(b_in.identity.clone()))?;
        if val.width() != b_in.width {
            return Err(IrError::InputWidthMismatch {
                identity: b_in.identity.clone(),
                expected: b_in.width,
                actual: val.width(),
            }
            .into());
        }
        seen_boundary.insert(b_in.identity.clone());
        trace.push(TraceEvent::InputEvaluated {
            identity: b_in.identity,
            effect: b_in.effect,
        });
    }

    for (id, &expected_w) in declared_inputs {
        if seen_boundary.contains(id) {
            continue;
        }
        let val = inputs
            .get(id)
            .ok_or_else(|| IrError::MissingInput(id.clone()))?;
        if val.width() != expected_w {
            return Err(IrError::InputWidthMismatch {
                identity: id.clone(),
                expected: expected_w,
                actual: val.width(),
            }
            .into());
        }
    }

    let mut checks = boundary.checks.clone();
    checks.sort_by_key(BoundaryCheck::check_order);
    for chk in checks {
        if !chk.eval(inputs)? {
            return Err(EvalError::BoundaryCheckFailed(chk));
        }
        trace.push(TraceEvent::CheckPassed { check: chk });
    }

    Ok(trace)
}

/// Appends boundary cleanup actions in deterministic `cleanup_order`.
pub(crate) fn run_boundary_cleanups(boundary: &BehaviorBoundary, trace: &mut Vec<TraceEvent>) {
    let mut cleanups = boundary.cleanups.clone();
    cleanups.sort_by_key(|item| item.cleanup_order);
    for cleanup in cleanups {
        trace.push(TraceEvent::CleanupExecuted { cleanup });
    }
}

/// Exact bit-by-bit reference evaluator for a [`Region`].
///
/// Serves as the semantic ground truth for both `BaselineProgram` lowering and
/// algebraic/CEGIS synthesis verification.
pub fn eval_region(
    region: &Region,
    inputs: &BTreeMap<InputIdentity, BitValue>,
) -> Result<EvalResult, EvalError> {
    region.validate()?;
    let mut trace = run_boundary_prefix(&region.boundary, &region.inputs, inputs)?;

    if !region.domain.eval(inputs)? {
        return Err(EvalError::DomainViolation);
    }

    let mut node_bits: Vec<Vec<bool>> = Vec::with_capacity(region.nodes.len());
    for node in &region.nodes {
        let bits = match node {
            Node::Input { width, identity } => {
                let val = inputs
                    .get(identity)
                    .ok_or_else(|| IrError::MissingInput(identity.clone()))?;
                if val.width() != *width {
                    return Err(IrError::InputWidthMismatch {
                        identity: identity.clone(),
                        expected: *width,
                        actual: val.width(),
                    }
                    .into());
                }
                let mut vec = Vec::with_capacity(*width as usize);
                for i in 0..*width {
                    vec.push(val.bit(i)?);
                }
                vec
            }
            Node::Constant { width, bits } => {
                let mut vec = Vec::with_capacity(*width as usize);
                for i in 0..*width {
                    vec.push(bits.bit(i)?);
                }
                vec
            }
            Node::Concat(hi, lo) => {
                let lo_bits = &node_bits[lo.0];
                let hi_bits = &node_bits[hi.0];
                let mut vec = Vec::with_capacity(lo_bits.len() + hi_bits.len());
                vec.extend_from_slice(lo_bits);
                vec.extend_from_slice(hi_bits);
                vec
            }
            Node::Extract {
                input,
                start,
                width,
            } => {
                let in_bits = &node_bits[input.0];
                let start_idx: usize = match start {
                    ExtractStart::Const(k) => *k as usize,
                    ExtractStart::Dynamic(idx_node) => {
                        let idx_bv = BitValue::from_bits_lsb(&node_bits[idx_node.0]);
                        if *width as usize > in_bits.len() {
                            return Err(EvalError::DynamicSliceOutOfBounds {
                                input_width: in_bits.len() as u32,
                                slice_width: *width,
                                start_detail: idx_bv.to_string(),
                            });
                        }
                        let max_valid = (in_bits.len() - (*width as usize)) as u64;
                        let max_bv = BitValue::from_u64(idx_bv.width().max(64), max_valid)?;
                        if idx_bv.cmp_unsigned(&max_bv) == Ordering::Greater {
                            return Err(EvalError::DynamicSliceOutOfBounds {
                                input_width: in_bits.len() as u32,
                                slice_width: *width,
                                start_detail: idx_bv.to_string(),
                            });
                        }
                        idx_bv.words().first().copied().unwrap_or(0) as usize
                    }
                };
                if *width == 0 {
                    Vec::new()
                } else {
                    let end = start_idx + (*width as usize);
                    in_bits[start_idx..end].to_vec()
                }
            }
            Node::And(l, r) => node_bits[l.0]
                .iter()
                .zip(&node_bits[r.0])
                .map(|(&a, &b)| a & b)
                .collect(),
            Node::Or(l, r) => node_bits[l.0]
                .iter()
                .zip(&node_bits[r.0])
                .map(|(&a, &b)| a | b)
                .collect(),
            Node::Not(inner) => node_bits[inner.0].iter().map(|&b| !b).collect(),
            Node::Equal(l, r) => vec![node_bits[l.0] == node_bits[r.0]],
            Node::ZeroExtend { input, width } => {
                let in_bits = &node_bits[input.0];
                let mut vec = Vec::with_capacity(*width as usize);
                vec.extend_from_slice(in_bits);
                vec.resize(*width as usize, false);
                vec
            }
        };
        node_bits.push(bits);
    }

    run_boundary_cleanups(&region.boundary, &mut trace);
    let root_value = BitValue::from_bits_lsb(&node_bits[region.root.0]);
    Ok(EvalResult {
        value: root_value,
        trace,
    })
}

/// Convenience helper returning only the evaluated [`BitValue`] of `region`.
pub fn eval_region_value(
    region: &Region,
    inputs: &BTreeMap<InputIdentity, BitValue>,
) -> Result<BitValue, EvalError> {
    Ok(eval_region(region, inputs)?.value)
}

/// Virtual register index in a [`BaselineProgram`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BaselineReg(pub usize);

impl Display for BaselineReg {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "%b{}", self.0)
    }
}

/// Machine-word baseline instruction operating on `W`-bit words (`O(ceil(N/W))`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BaselineOp {
    ZeroWidthEmpty {
        dst: BaselineReg,
    },
    LoadConst {
        dst: BaselineReg,
        width: u32,
        words: Vec<u64>,
    },
    AcquireInput {
        dst: BaselineReg,
        identity: InputIdentity,
        width: u32,
    },
    WordAnd {
        dst: BaselineReg,
        lhs: BaselineReg,
        rhs: BaselineReg,
        num_words: usize,
    },
    WordOr {
        dst: BaselineReg,
        lhs: BaselineReg,
        rhs: BaselineReg,
        num_words: usize,
    },
    WordNot {
        dst: BaselineReg,
        src: BaselineReg,
        num_words: usize,
        tail_mask: u64,
    },
    WordEqual {
        dst: BaselineReg,
        lhs: BaselineReg,
        rhs: BaselineReg,
        num_words: usize,
    },
    WordZeroExtend {
        dst: BaselineReg,
        src: BaselineReg,
        src_words: usize,
        dst_words: usize,
    },
    WordConcatAligned {
        dst: BaselineReg,
        high: BaselineReg,
        low: BaselineReg,
        low_words: usize,
        high_words: usize,
    },
    WordConcatShifted {
        dst: BaselineReg,
        high: BaselineReg,
        low: BaselineReg,
        low_words: usize,
        high_words: usize,
        shift: u32,
        inv_shift: u32,
        total_words: usize,
        tail_mask: u64,
    },
    WordExtractSingleWord {
        dst: BaselineReg,
        src: BaselineReg,
        word_idx: usize,
        bit_shift: u32,
        mask: u64,
    },
    WordExtractAligned {
        dst: BaselineReg,
        src: BaselineReg,
        word_offset: usize,
        dst_words: usize,
        tail_mask: u64,
    },
    WordExtractCrossWord {
        dst: BaselineReg,
        src: BaselineReg,
        word_offset: usize,
        shift: u32,
        inv_shift: u32,
        dst_words: usize,
        tail_mask: u64,
    },
    WordExtractDynamic {
        dst: BaselineReg,
        src: BaselineReg,
        start_reg: BaselineReg,
        src_width: u32,
        width: u32,
        tail_mask: u64,
    },
}

impl BaselineOp {
    /// Returns the `O(ceil(N / W))` baseline cost of this machine-word operation.
    pub fn cost(&self) -> u64 {
        match self {
            Self::ZeroWidthEmpty { .. } | Self::AcquireInput { .. } => 0,
            Self::LoadConst { words, .. } => words.len().max(1) as u64,
            Self::WordAnd { num_words, .. }
            | Self::WordOr { num_words, .. }
            | Self::WordNot { num_words, .. }
            | Self::WordEqual { num_words, .. } => (*num_words).max(1) as u64,
            Self::WordZeroExtend { dst_words, .. } | Self::WordExtractAligned { dst_words, .. } => {
                (*dst_words).max(1) as u64
            }
            Self::WordConcatAligned {
                low_words,
                high_words,
                ..
            } => (*low_words + *high_words).max(1) as u64,
            Self::WordConcatShifted { total_words, .. } => (2 * (*total_words).max(1)) as u64,
            Self::WordExtractSingleWord { .. } => 1,
            Self::WordExtractCrossWord { dst_words, .. } => (2 * (*dst_words).max(1)) as u64,
            Self::WordExtractDynamic { width, .. } => {
                if *width == 0 {
                    1
                } else {
                    4
                }
            }
        }
    }
}

impl Display for BaselineOp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ZeroWidthEmpty { dst } => write!(f, "(zw-empty {dst})"),
            Self::LoadConst { dst, width, words } => {
                write!(f, "(load-const {dst} (width {width}) (words (")?;
                for (i, w) in words.iter().enumerate() {
                    if i != 0 {
                        f.write_str(" ")?;
                    }
                    write!(f, "#x{w:x}")?;
                }
                f.write_str(")))")
            }
            Self::AcquireInput {
                dst,
                identity,
                width,
            } => write!(f, "(acquire-input {dst} (width {width}) {identity})"),
            Self::WordAnd {
                dst,
                lhs,
                rhs,
                num_words,
            } => write!(f, "(word-and {dst} {lhs} {rhs} (words {num_words}))"),
            Self::WordOr {
                dst,
                lhs,
                rhs,
                num_words,
            } => write!(f, "(word-or {dst} {lhs} {rhs} (words {num_words}))"),
            Self::WordNot {
                dst,
                src,
                num_words,
                tail_mask,
            } => write!(
                f,
                "(word-not {dst} {src} (words {num_words}) (tail-mask #x{tail_mask:x}))"
            ),
            Self::WordEqual {
                dst,
                lhs,
                rhs,
                num_words,
            } => write!(f, "(word-equal {dst} {lhs} {rhs} (words {num_words}))"),
            Self::WordZeroExtend {
                dst,
                src,
                src_words,
                dst_words,
            } => write!(
                f,
                "(word-zext {dst} {src} (src-words {src_words}) (dst-words {dst_words}))"
            ),
            Self::WordConcatAligned {
                dst,
                high,
                low,
                low_words,
                high_words,
            } => write!(
                f,
                "(word-concat-aligned {dst} {high} {low} (low-words {low_words}) (high-words {high_words}))"
            ),
            Self::WordConcatShifted {
                dst,
                high,
                low,
                shift,
                inv_shift,
                total_words,
                tail_mask,
                ..
            } => write!(
                f,
                "(word-concat-shifted {dst} {high} {low} (shift {shift}) (inv-shift {inv_shift}) (words {total_words}) (tail-mask #x{tail_mask:x}))"
            ),
            Self::WordExtractSingleWord {
                dst,
                src,
                word_idx,
                bit_shift,
                mask,
            } => write!(
                f,
                "(word-extract-single {dst} {src} (word {word_idx}) (shift {bit_shift}) (mask #x{mask:x}))"
            ),
            Self::WordExtractAligned {
                dst,
                src,
                word_offset,
                dst_words,
                tail_mask,
            } => write!(
                f,
                "(word-extract-aligned {dst} {src} (word-off {word_offset}) (words {dst_words}) (tail-mask #x{tail_mask:x}))"
            ),
            Self::WordExtractCrossWord {
                dst,
                src,
                word_offset,
                shift,
                inv_shift,
                dst_words,
                tail_mask,
            } => write!(
                f,
                "(word-extract-cross {dst} {src} (word-off {word_offset}) (shift {shift}) (inv-shift {inv_shift}) (words {dst_words}) (tail-mask #x{tail_mask:x}))"
            ),
            Self::WordExtractDynamic {
                dst,
                src,
                start_reg,
                src_width,
                width,
                tail_mask,
            } => write!(
                f,
                "(word-extract-dynamic {dst} {src} {start_reg} (src-width {src_width}) (width {width}) (tail-mask #x{tail_mask:x}))"
            ),
        }
    }
}

/// Verified machine-word baseline implementation for a BitVec [`Region`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BaselineProgram {
    pub word_bits: u32,
    pub declared_inputs: BTreeMap<InputIdentity, u32>,
    pub domain: crate::ir::DomainConstraint,
    pub boundary: BehaviorBoundary,
    pub reg_widths: Vec<u32>,
    pub ops: Vec<BaselineOp>,
    pub output: BaselineReg,
    pub output_width: u32,
}

impl BaselineProgram {
    /// Total baseline instruction cost (`O(ceil(N / W))` per node).
    pub fn cost(&self) -> u64 {
        self.ops.iter().map(BaselineOp::cost).sum()
    }

    /// Executes the machine-word baseline program with exact `word_bits` masking
    /// and strict preservation of `BehaviorBoundary` events, checks, and cleanups.
    pub fn execute(
        &self,
        inputs: &BTreeMap<InputIdentity, BitValue>,
    ) -> Result<EvalResult, EvalError> {
        let mut trace = run_boundary_prefix(&self.boundary, &self.declared_inputs, inputs)?;
        if !self.domain.eval(inputs)? {
            return Err(EvalError::DomainViolation);
        }

        let w_bits = self.word_bits;
        let w_mask = word_mask(w_bits);
        let mut regs: Vec<Vec<u64>> = vec![Vec::new(); self.reg_widths.len()];

        for op in &self.ops {
            match op {
                BaselineOp::ZeroWidthEmpty { dst } => {
                    regs[dst.0] = Vec::new();
                }
                BaselineOp::LoadConst { dst, words, .. } => {
                    regs[dst.0] = words.clone();
                }
                BaselineOp::AcquireInput {
                    dst,
                    identity,
                    width,
                } => {
                    let val = inputs
                        .get(identity)
                        .ok_or_else(|| IrError::MissingInput(identity.clone()))?;
                    if val.width() != *width {
                        return Err(IrError::InputWidthMismatch {
                            identity: identity.clone(),
                            expected: *width,
                            actual: val.width(),
                        }
                        .into());
                    }
                    regs[dst.0] = pack_bitvalue_to_words(val, w_bits);
                }
                BaselineOp::WordAnd {
                    dst,
                    lhs,
                    rhs,
                    num_words,
                } => {
                    let mut out = Vec::with_capacity(*num_words);
                    for i in 0..*num_words {
                        out.push(regs[lhs.0][i] & regs[rhs.0][i]);
                    }
                    regs[dst.0] = out;
                }
                BaselineOp::WordOr {
                    dst,
                    lhs,
                    rhs,
                    num_words,
                } => {
                    let mut out = Vec::with_capacity(*num_words);
                    for i in 0..*num_words {
                        out.push(regs[lhs.0][i] | regs[rhs.0][i]);
                    }
                    regs[dst.0] = out;
                }
                BaselineOp::WordNot {
                    dst,
                    src,
                    num_words,
                    tail_mask,
                } => {
                    let mut out = Vec::with_capacity(*num_words);
                    for i in 0..*num_words {
                        out.push((!regs[src.0][i]) & w_mask);
                    }
                    if let Some(last) = out.last_mut() {
                        *last &= *tail_mask;
                    }
                    regs[dst.0] = out;
                }
                BaselineOp::WordEqual {
                    dst,
                    lhs,
                    rhs,
                    num_words,
                } => {
                    let mut eq = true;
                    for i in 0..*num_words {
                        if regs[lhs.0][i] != regs[rhs.0][i] {
                            eq = false;
                            break;
                        }
                    }
                    regs[dst.0] = vec![u64::from(eq)];
                }
                BaselineOp::WordZeroExtend {
                    dst,
                    src,
                    src_words,
                    dst_words,
                } => {
                    let mut out = vec![0u64; *dst_words];
                    out[..*src_words].copy_from_slice(&regs[src.0][..*src_words]);
                    regs[dst.0] = out;
                }
                BaselineOp::WordConcatAligned {
                    dst,
                    high,
                    low,
                    low_words,
                    high_words,
                } => {
                    let mut out = Vec::with_capacity(*low_words + *high_words);
                    out.extend_from_slice(&regs[low.0][..*low_words]);
                    out.extend_from_slice(&regs[high.0][..*high_words]);
                    regs[dst.0] = out;
                }
                BaselineOp::WordConcatShifted {
                    dst,
                    high,
                    low,
                    low_words,
                    high_words,
                    shift,
                    inv_shift,
                    total_words,
                    tail_mask,
                } => {
                    debug_assert!(*shift > 0 && *shift < w_bits);
                    debug_assert!(*inv_shift > 0 && *inv_shift < w_bits);
                    let mut out = vec![0u64; *total_words];
                    out[..*low_words].copy_from_slice(&regs[low.0][..*low_words]);
                    let word_off = *low_words - 1;
                    for j in 0..*high_words {
                        let hw = regs[high.0][j];
                        out[word_off + j] |= (hw << *shift) & w_mask;
                        if word_off + j + 1 < *total_words {
                            out[word_off + j + 1] |= hw >> *inv_shift;
                        }
                    }
                    if let Some(last) = out.last_mut() {
                        *last &= *tail_mask;
                    }
                    regs[dst.0] = out;
                }
                BaselineOp::WordExtractSingleWord {
                    dst,
                    src,
                    word_idx,
                    bit_shift,
                    mask,
                } => {
                    debug_assert!(*bit_shift < w_bits);
                    let word = regs[src.0][*word_idx];
                    regs[dst.0] = vec![(word >> *bit_shift) & *mask];
                }
                BaselineOp::WordExtractAligned {
                    dst,
                    src,
                    word_offset,
                    dst_words,
                    tail_mask,
                } => {
                    let mut out = regs[src.0][*word_offset..*word_offset + *dst_words].to_vec();
                    if let Some(last) = out.last_mut() {
                        *last &= *tail_mask;
                    }
                    regs[dst.0] = out;
                }
                BaselineOp::WordExtractCrossWord {
                    dst,
                    src,
                    word_offset,
                    shift,
                    inv_shift,
                    dst_words,
                    tail_mask,
                } => {
                    debug_assert!(*shift > 0 && *shift < w_bits);
                    debug_assert!(*inv_shift > 0 && *inv_shift < w_bits);
                    let src_words = &regs[src.0];
                    let mut out = Vec::with_capacity(*dst_words);
                    for j in 0..*dst_words {
                        let lo = src_words[*word_offset + j] >> *shift;
                        let hi = if *word_offset + j + 1 < src_words.len() {
                            (src_words[*word_offset + j + 1] << *inv_shift) & w_mask
                        } else {
                            0
                        };
                        out.push(lo | hi);
                    }
                    if let Some(last) = out.last_mut() {
                        *last &= *tail_mask;
                    }
                    regs[dst.0] = out;
                }
                BaselineOp::WordExtractDynamic {
                    dst,
                    src,
                    start_reg,
                    src_width,
                    width,
                    tail_mask,
                } => {
                    let start_bv = unpack_words_to_bitvalue(
                        &regs[start_reg.0],
                        self.reg_widths[start_reg.0],
                        w_bits,
                    );
                    if *width > *src_width {
                        return Err(EvalError::DynamicSliceOutOfBounds {
                            input_width: *src_width,
                            slice_width: *width,
                            start_detail: start_bv.to_string(),
                        });
                    }
                    let max_start = u64::from(*src_width - *width);
                    let max_bv = BitValue::from_u64(start_bv.width().max(64), max_start)?;
                    if start_bv.cmp_unsigned(&max_bv) == Ordering::Greater {
                        return Err(EvalError::DynamicSliceOutOfBounds {
                            input_width: *src_width,
                            slice_width: *width,
                            start_detail: start_bv.to_string(),
                        });
                    }
                    if *width == 0 {
                        regs[dst.0] = Vec::new();
                    } else {
                        let start = start_bv.words().first().copied().unwrap_or(0) as u32;
                        let dst_words = width.div_ceil(w_bits) as usize;
                        let word_off = (start / w_bits) as usize;
                        let shift = start % w_bits;
                        let src_words = &regs[src.0];
                        let mut out = Vec::with_capacity(dst_words);
                        if shift == 0 {
                            out.extend_from_slice(&src_words[word_off..word_off + dst_words]);
                        } else {
                            let inv_shift = w_bits - shift;
                            for j in 0..dst_words {
                                let lo = src_words[word_off + j] >> shift;
                                let hi = if word_off + j + 1 < src_words.len() {
                                    (src_words[word_off + j + 1] << inv_shift) & w_mask
                                } else {
                                    0
                                };
                                out.push(lo | hi);
                            }
                        }
                        if let Some(last) = out.last_mut() {
                            *last &= *tail_mask;
                        }
                        regs[dst.0] = out;
                    }
                }
            }
        }

        run_boundary_cleanups(&self.boundary, &mut trace);
        let value = unpack_words_to_bitvalue(&regs[self.output.0], self.output_width, w_bits);
        Ok(EvalResult { value, trace })
    }
}

impl Display for BaselineProgram {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("(baseline-program ")?;
        write!(
            f,
            "(word-bits {}) (output {}) (output-width {}) (cost {}) (ops ",
            self.word_bits,
            self.output,
            self.output_width,
            self.cost()
        )?;
        write_list(f, &self.ops)?;
        write!(f, ") (boundary {}))", self.boundary)
    }
}

/// Lowers a validated [`Region`] to an `O(ceil(N / W))` machine-word [`BaselineProgram`].
///
/// Separates zero-width, word-aligned, single-word, cross-word, and dynamic
/// slices so no operation ever executes an undefined shift by `word_bits`.
pub fn lower_to_baseline(region: &Region, word_bits: u32) -> Result<BaselineProgram, EvalError> {
    if word_bits == 0 || word_bits > 64 {
        return Err(EvalError::UnsupportedWordWidth(word_bits));
    }
    region.validate()?;
    let widths = region.node_widths()?;
    let mut ops = Vec::with_capacity(region.nodes.len());
    let mut reg_widths = Vec::with_capacity(region.nodes.len());
    let mut node_to_reg = Vec::with_capacity(region.nodes.len());

    for (idx, node) in region.nodes.iter().enumerate() {
        let w = widths[idx];
        let dst = BaselineReg(reg_widths.len());
        reg_widths.push(w);
        node_to_reg.push(dst);

        if w == 0 {
            if let Node::Extract {
                input,
                start: ExtractStart::Dynamic(start_node),
                width: 0,
            } = node
            {
                ops.push(BaselineOp::WordExtractDynamic {
                    dst,
                    src: node_to_reg[input.0],
                    start_reg: node_to_reg[start_node.0],
                    src_width: widths[input.0],
                    width: 0,
                    tail_mask: 0,
                });
            } else {
                ops.push(BaselineOp::ZeroWidthEmpty { dst });
            }
            continue;
        }

        match node {
            Node::Input { width, identity } => {
                ops.push(BaselineOp::AcquireInput {
                    dst,
                    identity: identity.clone(),
                    width: *width,
                });
            }
            Node::Constant { width, bits } => {
                ops.push(BaselineOp::LoadConst {
                    dst,
                    width: *width,
                    words: pack_bitvalue_to_words(bits, word_bits),
                });
            }
            Node::Concat(hi, lo) => {
                let lw = widths[lo.0];
                let hw = widths[hi.0];
                let hi_reg = node_to_reg[hi.0];
                let lo_reg = node_to_reg[lo.0];
                if lw == 0 {
                    let dst_words = hw.div_ceil(word_bits) as usize;
                    ops.push(BaselineOp::WordZeroExtend {
                        dst,
                        src: hi_reg,
                        src_words: dst_words,
                        dst_words,
                    });
                } else if hw == 0 {
                    let dst_words = lw.div_ceil(word_bits) as usize;
                    ops.push(BaselineOp::WordZeroExtend {
                        dst,
                        src: lo_reg,
                        src_words: dst_words,
                        dst_words,
                    });
                } else {
                    let low_words = lw.div_ceil(word_bits) as usize;
                    let high_words = hw.div_ceil(word_bits) as usize;
                    let shift = lw % word_bits;
                    if shift == 0 {
                        ops.push(BaselineOp::WordConcatAligned {
                            dst,
                            high: hi_reg,
                            low: lo_reg,
                            low_words,
                            high_words,
                        });
                    } else {
                        let inv_shift = word_bits - shift;
                        let total_words = w.div_ceil(word_bits) as usize;
                        ops.push(BaselineOp::WordConcatShifted {
                            dst,
                            high: hi_reg,
                            low: lo_reg,
                            low_words,
                            high_words,
                            shift,
                            inv_shift,
                            total_words,
                            tail_mask: tail_mask_for_width(w, word_bits),
                        });
                    }
                }
            }
            Node::Extract {
                input,
                start,
                width,
            } => {
                let src_reg = node_to_reg[input.0];
                let src_width = widths[input.0];
                match start {
                    ExtractStart::Const(lo) => {
                        let dst_words = width.div_ceil(word_bits) as usize;
                        let tail_mask = tail_mask_for_width(*width, word_bits);
                        let word_offset = (*lo / word_bits) as usize;
                        let shift = *lo % word_bits;
                        if shift == 0 {
                            ops.push(BaselineOp::WordExtractAligned {
                                dst,
                                src: src_reg,
                                word_offset,
                                dst_words,
                                tail_mask,
                            });
                        } else {
                            let end_word = ((*lo + *width - 1) / word_bits) as usize;
                            if end_word == word_offset {
                                ops.push(BaselineOp::WordExtractSingleWord {
                                    dst,
                                    src: src_reg,
                                    word_idx: word_offset,
                                    bit_shift: shift,
                                    mask: tail_mask,
                                });
                            } else {
                                let inv_shift = word_bits - shift;
                                ops.push(BaselineOp::WordExtractCrossWord {
                                    dst,
                                    src: src_reg,
                                    word_offset,
                                    shift,
                                    inv_shift,
                                    dst_words,
                                    tail_mask,
                                });
                            }
                        }
                    }
                    ExtractStart::Dynamic(start_node) => {
                        ops.push(BaselineOp::WordExtractDynamic {
                            dst,
                            src: src_reg,
                            start_reg: node_to_reg[start_node.0],
                            src_width,
                            width: *width,
                            tail_mask: tail_mask_for_width(*width, word_bits),
                        });
                    }
                }
            }
            Node::And(l, r) => {
                let num_words = w.div_ceil(word_bits) as usize;
                ops.push(BaselineOp::WordAnd {
                    dst,
                    lhs: node_to_reg[l.0],
                    rhs: node_to_reg[r.0],
                    num_words,
                });
            }
            Node::Or(l, r) => {
                let num_words = w.div_ceil(word_bits) as usize;
                ops.push(BaselineOp::WordOr {
                    dst,
                    lhs: node_to_reg[l.0],
                    rhs: node_to_reg[r.0],
                    num_words,
                });
            }
            Node::Not(inner) => {
                let num_words = w.div_ceil(word_bits) as usize;
                ops.push(BaselineOp::WordNot {
                    dst,
                    src: node_to_reg[inner.0],
                    num_words,
                    tail_mask: tail_mask_for_width(w, word_bits),
                });
            }
            Node::Equal(l, r) => {
                let op_w = widths[l.0];
                if op_w == 0 {
                    ops.push(BaselineOp::LoadConst {
                        dst,
                        width: 1,
                        words: vec![1],
                    });
                } else {
                    let num_words = op_w.div_ceil(word_bits) as usize;
                    ops.push(BaselineOp::WordEqual {
                        dst,
                        lhs: node_to_reg[l.0],
                        rhs: node_to_reg[r.0],
                        num_words,
                    });
                }
            }
            Node::ZeroExtend { input, width } => {
                let src_words = widths[input.0].div_ceil(word_bits) as usize;
                let dst_words = width.div_ceil(word_bits) as usize;
                ops.push(BaselineOp::WordZeroExtend {
                    dst,
                    src: node_to_reg[input.0],
                    src_words,
                    dst_words,
                });
            }
        }
    }

    Ok(BaselineProgram {
        word_bits,
        declared_inputs: region.inputs.clone(),
        domain: region.domain.clone(),
        boundary: region.boundary.clone(),
        reg_widths,
        ops,
        output: node_to_reg[region.root.0],
        output_width: widths[region.root.0],
    })
}

pub(crate) fn word_mask(word_bits: u32) -> u64 {
    if word_bits >= 64 {
        u64::MAX
    } else {
        (1u64 << word_bits) - 1
    }
}

pub(crate) fn tail_mask_for_width(width: u32, word_bits: u32) -> u64 {
    if width == 0 {
        return 0;
    }
    let rem = width % word_bits;
    if rem == 0 {
        word_mask(word_bits)
    } else {
        (1u64 << rem) - 1
    }
}

pub(crate) fn pack_bitvalue_to_words(val: &BitValue, word_bits: u32) -> Vec<u64> {
    if val.width() == 0 {
        return Vec::new();
    }
    if word_bits == 64 {
        return val.words().to_vec();
    }
    let num_words = val.width().div_ceil(word_bits) as usize;
    let mut out = vec![0u64; num_words];
    for bit_idx in 0..val.width() {
        if val.bit(bit_idx).unwrap_or(false) {
            let w_idx = (bit_idx / word_bits) as usize;
            let b_idx = bit_idx % word_bits;
            out[w_idx] |= 1u64 << b_idx;
        }
    }
    out
}

pub(crate) fn unpack_words_to_bitvalue(words: &[u64], width: u32, word_bits: u32) -> BitValue {
    if width == 0 {
        return BitValue::empty();
    }
    if word_bits == 64 {
        let mut clean = words.to_vec();
        if let Some(last) = clean.last_mut() {
            *last &= top_word_mask(width);
        }
        return BitValue::from_words(width, clean).expect("valid 64-bit word packing");
    }
    let mut bits = Vec::with_capacity(width as usize);
    for bit_idx in 0..width {
        let w_idx = (bit_idx / word_bits) as usize;
        let b_idx = bit_idx % word_bits;
        let bit = words.get(w_idx).is_some_and(|&w| ((w >> b_idx) & 1) == 1);
        bits.push(bit);
    }
    BitValue::from_bits_lsb(&bits)
}
