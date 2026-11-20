//! Target-aware CEGIS + rewrite BitVec synthesizer and verification cache.
//!
//! Lowers regions to a verified [`BaselineProgram`](crate::baseline::BaselineProgram)
//! first, searches lower-cost target programs via algebraic rewrites and
//! counterexample-guided inductive synthesis (CEGIS), formally verifies every
//! candidate with [`smt`](crate::smt), and unconditionally falls back to the
//! verified baseline on `Unknown(Budget)` or when no cheaper verified candidate
//! exists.

use std::collections::BTreeMap;
use std::fmt::{self, Display, Write};

use crate::baseline::{
    BaselineProgram, BitValue, EvalError, EvalResult, eval_region, lower_to_baseline,
    run_boundary_cleanups, run_boundary_prefix,
};
use crate::ir::{
    BehaviorBoundary, CanonicalRegion, DomainConstraint, ExtractStart, InputIdentity, IrError,
    Node, NodeId, Region,
};
use crate::rewrites::{Justification, PatternKind, RecognizedPattern, simplify_region};
use crate::sexpr::{Quoted, write_list};
use crate::smt::{
    Budget, ProofCertificate, ProofOutcome, VerifyError, find_satisfying_inputs,
    verify_target_program,
};
use crate::zero_width::eliminate_zero_width;

/// Version tag mixed into [`SynthesisCache`] keys to invalidate stale proofs.
pub const VERIFIER_VERSION: &str = "bitvec-cdcl-tseitin-v1";

/// Target ISA profile governing legal instructions and cost weights.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum TargetProfile {
    Generic,
    X86_64BmiBswap,
    Aarch64,
}

impl TargetProfile {
    /// Returns `true` if `op` is legally supported on this target profile.
    pub fn supports_op(self, op: &TargetOp) -> bool {
        match op {
            TargetOp::ZeroWidthEmpty { .. }
            | TargetOp::Const { .. }
            | TargetOp::Input { .. }
            | TargetOp::Copy { .. }
            | TargetOp::And { .. }
            | TargetOp::Or { .. }
            | TargetOp::Xor { .. }
            | TargetOp::Not { .. }
            | TargetOp::Equal { .. }
            | TargetOp::Concat { .. }
            | TargetOp::ExtractConst { .. }
            | TargetOp::ExtractDynamic { .. }
            | TargetOp::ZeroExtend { .. }
            | TargetOp::ShlConst { .. }
            | TargetOp::LshrConst { .. }
            | TargetOp::Shl { .. }
            | TargetOp::Lshr { .. } => true,
            TargetOp::AndNot { .. } => matches!(self, Self::X86_64BmiBswap | Self::Aarch64),
            TargetOp::OrNot { .. } => matches!(self, Self::Aarch64),
            TargetOp::Bswap16 { .. }
            | TargetOp::Bswap32 { .. }
            | TargetOp::Bswap64 { .. }
            | TargetOp::RotrConst { .. }
            | TargetOp::RotlConst { .. }
            | TargetOp::Rotr { .. }
            | TargetOp::Rotl { .. } => matches!(self, Self::X86_64BmiBswap | Self::Aarch64),
        }
    }

    /// Default target instruction cost model for this profile.
    pub fn default_cost_model(self) -> CostModel {
        CostModel::for_target(self)
    }
}

impl Display for TargetProfile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Generic => "generic",
            Self::X86_64BmiBswap => "x86_64-bmi-bswap",
            Self::Aarch64 => "aarch64",
        })
    }
}

/// Deterministic target instruction cost model.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CostModel {
    pub const_cost: u64,
    pub bitwise_cost: u64,
    pub xor_cost: u64,
    pub and_not_cost: u64,
    pub or_not_cost: u64,
    pub equal_cost: u64,
    pub concat_cost: u64,
    pub extract_cost: u64,
    pub dynamic_extract_cost: u64,
    pub zext_cost: u64,
    pub shift_cost: u64,
    pub rotate_cost: u64,
    pub bswap_cost: u64,
}

impl CostModel {
    /// Returns calibrated instruction costs for `profile`.
    pub fn for_target(profile: TargetProfile) -> Self {
        match profile {
            TargetProfile::Generic => Self {
                const_cost: 1,
                bitwise_cost: 1,
                xor_cost: 1,
                and_not_cost: 100,
                or_not_cost: 100,
                equal_cost: 2,
                concat_cost: 2,
                extract_cost: 1,
                dynamic_extract_cost: 4,
                zext_cost: 1,
                shift_cost: 2,
                rotate_cost: 100,
                bswap_cost: 100,
            },
            TargetProfile::X86_64BmiBswap => Self {
                const_cost: 1,
                bitwise_cost: 1,
                xor_cost: 1,
                and_not_cost: 1,
                or_not_cost: 100,
                equal_cost: 2,
                concat_cost: 2,
                extract_cost: 1,
                dynamic_extract_cost: 4,
                zext_cost: 1,
                shift_cost: 1,
                rotate_cost: 1,
                bswap_cost: 1,
            },
            TargetProfile::Aarch64 => Self {
                const_cost: 1,
                bitwise_cost: 1,
                xor_cost: 1,
                and_not_cost: 1,
                or_not_cost: 1,
                equal_cost: 2,
                concat_cost: 2,
                extract_cost: 1,
                dynamic_extract_cost: 4,
                zext_cost: 1,
                shift_cost: 1,
                rotate_cost: 1,
                bswap_cost: 1,
            },
        }
    }

    /// Computes cost of a single target instruction, scaling multi-word operations by 64-bit chunks.
    pub fn op_cost(&self, op: &TargetOp, output_width: u32) -> u64 {
        let word_scale = output_width.div_ceil(64).max(1) as u64;
        match op {
            TargetOp::ZeroWidthEmpty { .. } | TargetOp::Input { .. } | TargetOp::Copy { .. } => 0,
            TargetOp::Const { .. } => self.const_cost,
            TargetOp::And { .. } | TargetOp::Or { .. } | TargetOp::Not { .. } => {
                self.bitwise_cost * word_scale
            }
            TargetOp::Xor { .. } => self.xor_cost * word_scale,
            TargetOp::AndNot { .. } => self.and_not_cost * word_scale,
            TargetOp::OrNot { .. } => self.or_not_cost * word_scale,
            TargetOp::Equal { operand_width, .. } => {
                self.equal_cost * (operand_width.div_ceil(64).max(1) as u64)
            }
            TargetOp::Concat { .. } => self.concat_cost * word_scale,
            TargetOp::ExtractConst { .. } => self.extract_cost * word_scale,
            TargetOp::ExtractDynamic { .. } => self.dynamic_extract_cost * word_scale,
            TargetOp::ZeroExtend { .. } => self.zext_cost * word_scale,
            TargetOp::ShlConst { .. }
            | TargetOp::LshrConst { .. }
            | TargetOp::Shl { .. }
            | TargetOp::Lshr { .. } => self.shift_cost * word_scale,
            TargetOp::RotrConst { .. }
            | TargetOp::RotlConst { .. }
            | TargetOp::Rotr { .. }
            | TargetOp::Rotl { .. } => self.rotate_cost * word_scale,
            TargetOp::Bswap16 { .. } | TargetOp::Bswap32 { .. } | TargetOp::Bswap64 { .. } => {
                self.bswap_cost
            }
        }
    }
}

impl Display for CostModel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(cost-model (const {}) (bitwise {}) (xor {}) (andn {}) (orn {}) (eq {}) (concat {}) (extract {}) (dyn-extract {}) (zext {}) (shift {}) (rotate {}) (bswap {}))",
            self.const_cost,
            self.bitwise_cost,
            self.xor_cost,
            self.and_not_cost,
            self.or_not_cost,
            self.equal_cost,
            self.concat_cost,
            self.extract_cost,
            self.dynamic_extract_cost,
            self.zext_cost,
            self.shift_cost,
            self.rotate_cost,
            self.bswap_cost
        )
    }
}

/// Virtual register index in a [`TargetProgram`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TargetReg(pub usize);

impl Display for TargetReg {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "%t{}", self.0)
    }
}

/// Target instruction opcodes used by synthesized BitVec programs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TargetOp {
    ZeroWidthEmpty {
        width: u32,
    },
    Const {
        width: u32,
        bits: BitValue,
    },
    Input {
        width: u32,
        identity: InputIdentity,
    },
    Copy {
        width: u32,
        src: TargetReg,
    },
    And {
        width: u32,
        lhs: TargetReg,
        rhs: TargetReg,
    },
    Or {
        width: u32,
        lhs: TargetReg,
        rhs: TargetReg,
    },
    Xor {
        width: u32,
        lhs: TargetReg,
        rhs: TargetReg,
    },
    Not {
        width: u32,
        src: TargetReg,
    },
    AndNot {
        width: u32,
        lhs: TargetReg,
        inverted: TargetReg,
    },
    OrNot {
        width: u32,
        lhs: TargetReg,
        inverted: TargetReg,
    },
    Equal {
        operand_width: u32,
        lhs: TargetReg,
        rhs: TargetReg,
    },
    Concat {
        high: TargetReg,
        high_width: u32,
        low: TargetReg,
        low_width: u32,
    },
    ExtractConst {
        input: TargetReg,
        start: u32,
        width: u32,
    },
    ExtractDynamic {
        input: TargetReg,
        input_width: u32,
        start: TargetReg,
        width: u32,
    },
    ZeroExtend {
        input: TargetReg,
        target_width: u32,
    },
    Bswap16 {
        input: TargetReg,
    },
    Bswap32 {
        input: TargetReg,
    },
    Bswap64 {
        input: TargetReg,
    },
    RotrConst {
        input: TargetReg,
        width: u32,
        amount: u32,
    },
    RotlConst {
        input: TargetReg,
        width: u32,
        amount: u32,
    },
    ShlConst {
        input: TargetReg,
        width: u32,
        amount: u32,
    },
    LshrConst {
        input: TargetReg,
        width: u32,
        amount: u32,
    },
    Shl {
        input: TargetReg,
        width: u32,
        shift: TargetReg,
    },
    Lshr {
        input: TargetReg,
        width: u32,
        shift: TargetReg,
    },
    Rotr {
        input: TargetReg,
        width: u32,
        shift: TargetReg,
    },
    Rotl {
        input: TargetReg,
        width: u32,
        shift: TargetReg,
    },
}

impl Display for TargetOp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ZeroWidthEmpty { width } => write!(f, "(t-zw-empty (width {width}))"),
            Self::Const { width, bits } => write!(f, "(t-const (width {width}) {bits})"),
            Self::Input { width, identity } => {
                write!(f, "(t-input (width {width}) {identity})")
            }
            Self::Copy { width, src } => write!(f, "(t-copy {src} (width {width}))"),
            Self::And { width, lhs, rhs } => write!(f, "(t-and {lhs} {rhs} (width {width}))"),
            Self::Or { width, lhs, rhs } => write!(f, "(t-or {lhs} {rhs} (width {width}))"),
            Self::Xor { width, lhs, rhs } => write!(f, "(t-xor {lhs} {rhs} (width {width}))"),
            Self::Not { width, src } => write!(f, "(t-not {src} (width {width}))"),
            Self::AndNot {
                width,
                lhs,
                inverted,
            } => write!(f, "(t-andn {lhs} {inverted} (width {width}))"),
            Self::OrNot {
                width,
                lhs,
                inverted,
            } => write!(f, "(t-orn {lhs} {inverted} (width {width}))"),
            Self::Equal {
                operand_width,
                lhs,
                rhs,
            } => write!(f, "(t-equal {lhs} {rhs} (op-width {operand_width}))"),
            Self::Concat {
                high,
                high_width,
                low,
                low_width,
            } => write!(
                f,
                "(t-concat {high} (hi-w {high_width}) {low} (lo-w {low_width}))"
            ),
            Self::ExtractConst {
                input,
                start,
                width,
            } => write!(f, "(t-extract {input} (start {start}) (width {width}))"),
            Self::ExtractDynamic {
                input,
                input_width,
                start,
                width,
            } => write!(
                f,
                "(t-extract-dyn {input} (in-w {input_width}) {start} (width {width}))"
            ),
            Self::ZeroExtend {
                input,
                target_width,
            } => write!(f, "(t-zext {input} (width {target_width}))"),
            Self::Bswap16 { input } => write!(f, "(t-bswap16 {input})"),
            Self::Bswap32 { input } => write!(f, "(t-bswap32 {input})"),
            Self::Bswap64 { input } => write!(f, "(t-bswap64 {input})"),
            Self::RotrConst {
                input,
                width,
                amount,
            } => write!(
                f,
                "(t-rotr-const {input} (width {width}) (amount {amount}))"
            ),
            Self::RotlConst {
                input,
                width,
                amount,
            } => write!(
                f,
                "(t-rotl-const {input} (width {width}) (amount {amount}))"
            ),
            Self::ShlConst {
                input,
                width,
                amount,
            } => write!(f, "(t-shl-const {input} (width {width}) (amount {amount}))"),
            Self::LshrConst {
                input,
                width,
                amount,
            } => write!(
                f,
                "(t-lshr-const {input} (width {width}) (amount {amount}))"
            ),
            Self::Shl {
                input,
                width,
                shift,
            } => write!(f, "(t-shl {input} {shift} (width {width}))"),
            Self::Lshr {
                input,
                width,
                shift,
            } => write!(f, "(t-lshr {input} {shift} (width {width}))"),
            Self::Rotr {
                input,
                width,
                shift,
            } => write!(f, "(t-rotr {input} {shift} (width {width}))"),
            Self::Rotl {
                input,
                width,
                shift,
            } => write!(f, "(t-rotl {input} {shift} (width {width}))"),
        }
    }
}

/// Single SSA target instruction assigning `dst`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TargetInst {
    pub dst: TargetReg,
    pub width: u32,
    pub op: TargetOp,
}

impl Display for TargetInst {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(inst (dst {}) (width {}) {})",
            self.dst, self.width, self.op
        )
    }
}

/// Complete target-ISA BitVec program preserving domain and boundary contracts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TargetProgram {
    pub profile: TargetProfile,
    pub declared_inputs: BTreeMap<InputIdentity, u32>,
    pub domain: DomainConstraint,
    pub boundary: BehaviorBoundary,
    pub instructions: Vec<TargetInst>,
    pub root: TargetReg,
    pub output_width: u32,
    pub justifications: Vec<Justification>,
}

impl TargetProgram {
    /// Validates SSA register indices, bit widths, and target instruction invariants.
    pub fn validate(&self) -> Result<(), IrError> {
        let mut widths = Vec::with_capacity(self.instructions.len());
        for (idx, inst) in self.instructions.iter().enumerate() {
            if inst.dst.0 != idx {
                return Err(IrError::InvalidNodeId(NodeId(inst.dst.0)));
            }
            let expected_w = match &inst.op {
                TargetOp::ZeroWidthEmpty { width } => {
                    if *width != 0 {
                        return Err(IrError::WidthMismatch {
                            op: "target-zw-empty",
                            left: *width,
                            right: 0,
                        });
                    }
                    0
                }
                TargetOp::Const { width, bits } => {
                    if bits.width() != *width {
                        return Err(IrError::WidthMismatch {
                            op: "target-const",
                            left: *width,
                            right: bits.width(),
                        });
                    }
                    *width
                }
                TargetOp::Input { width, identity } => {
                    if let Some(&declared) = self.declared_inputs.get(identity)
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
                TargetOp::Copy { width, src } | TargetOp::Not { width, src } => {
                    let sw = checked_reg_width(&widths, idx, *src)?;
                    if sw != *width {
                        return Err(IrError::WidthMismatch {
                            op: "target-unary",
                            left: *width,
                            right: sw,
                        });
                    }
                    *width
                }
                TargetOp::And { width, lhs, rhs }
                | TargetOp::Or { width, lhs, rhs }
                | TargetOp::Xor { width, lhs, rhs } => {
                    let lw = checked_reg_width(&widths, idx, *lhs)?;
                    let rw = checked_reg_width(&widths, idx, *rhs)?;
                    if lw != *width || rw != *width {
                        return Err(IrError::WidthMismatch {
                            op: "target-binary",
                            left: lw,
                            right: rw,
                        });
                    }
                    *width
                }
                TargetOp::AndNot {
                    width,
                    lhs,
                    inverted,
                }
                | TargetOp::OrNot {
                    width,
                    lhs,
                    inverted,
                } => {
                    let lw = checked_reg_width(&widths, idx, *lhs)?;
                    let rw = checked_reg_width(&widths, idx, *inverted)?;
                    if lw != *width || rw != *width {
                        return Err(IrError::WidthMismatch {
                            op: "target-binary-not",
                            left: lw,
                            right: rw,
                        });
                    }
                    *width
                }
                TargetOp::Equal {
                    operand_width,
                    lhs,
                    rhs,
                } => {
                    let lw = checked_reg_width(&widths, idx, *lhs)?;
                    let rw = checked_reg_width(&widths, idx, *rhs)?;
                    if lw != *operand_width || rw != *operand_width {
                        return Err(IrError::WidthMismatch {
                            op: "target-equal",
                            left: lw,
                            right: rw,
                        });
                    }
                    1
                }
                TargetOp::Concat {
                    high,
                    high_width,
                    low,
                    low_width,
                } => {
                    let hw = checked_reg_width(&widths, idx, *high)?;
                    let lw = checked_reg_width(&widths, idx, *low)?;
                    if hw != *high_width || lw != *low_width {
                        return Err(IrError::WidthMismatch {
                            op: "target-concat",
                            left: hw,
                            right: *high_width,
                        });
                    }
                    hw.checked_add(lw).ok_or(IrError::WidthOverflow {
                        left: hw,
                        right: lw,
                    })?
                }
                TargetOp::ExtractConst {
                    input,
                    start,
                    width,
                } => {
                    let iw = checked_reg_width(&widths, idx, *input)?;
                    let end = start.checked_add(*width).ok_or(IrError::SliceOutOfBounds {
                        input_width: iw,
                        start: *start,
                        slice_width: *width,
                    })?;
                    if end > iw {
                        return Err(IrError::SliceOutOfBounds {
                            input_width: iw,
                            start: *start,
                            slice_width: *width,
                        });
                    }
                    *width
                }
                TargetOp::ExtractDynamic {
                    input,
                    input_width,
                    start,
                    width,
                } => {
                    let iw = checked_reg_width(&widths, idx, *input)?;
                    let sw = checked_reg_width(&widths, idx, *start)?;
                    if iw != *input_width {
                        return Err(IrError::WidthMismatch {
                            op: "target-extract-dyn",
                            left: iw,
                            right: *input_width,
                        });
                    }
                    if sw == 0 {
                        return Err(IrError::ZeroWidthDynamicIndex);
                    }
                    *width
                }
                TargetOp::ZeroExtend {
                    input,
                    target_width,
                } => {
                    let iw = checked_reg_width(&widths, idx, *input)?;
                    if *target_width < iw {
                        return Err(IrError::InvalidZeroExtend {
                            input_width: iw,
                            target_width: *target_width,
                        });
                    }
                    *target_width
                }
                TargetOp::Bswap16 { input } => {
                    let iw = checked_reg_width(&widths, idx, *input)?;
                    if iw != 16 {
                        return Err(IrError::InvalidByteSwapWidth(iw));
                    }
                    16
                }
                TargetOp::Bswap32 { input } => {
                    let iw = checked_reg_width(&widths, idx, *input)?;
                    if iw != 32 {
                        return Err(IrError::InvalidByteSwapWidth(iw));
                    }
                    32
                }
                TargetOp::Bswap64 { input } => {
                    let iw = checked_reg_width(&widths, idx, *input)?;
                    if iw != 64 {
                        return Err(IrError::InvalidByteSwapWidth(iw));
                    }
                    64
                }
                TargetOp::RotrConst { input, width, .. }
                | TargetOp::RotlConst { input, width, .. }
                | TargetOp::ShlConst { input, width, .. }
                | TargetOp::LshrConst { input, width, .. } => {
                    let iw = checked_reg_width(&widths, idx, *input)?;
                    if iw != *width {
                        return Err(IrError::WidthMismatch {
                            op: "target-const-shift-rot",
                            left: iw,
                            right: *width,
                        });
                    }
                    *width
                }
                TargetOp::Shl {
                    input,
                    width,
                    shift,
                }
                | TargetOp::Lshr {
                    input,
                    width,
                    shift,
                }
                | TargetOp::Rotr {
                    input,
                    width,
                    shift,
                }
                | TargetOp::Rotl {
                    input,
                    width,
                    shift,
                } => {
                    let iw = checked_reg_width(&widths, idx, *input)?;
                    let sw = checked_reg_width(&widths, idx, *shift)?;
                    if iw != *width {
                        return Err(IrError::WidthMismatch {
                            op: "target-shift-rot",
                            left: iw,
                            right: *width,
                        });
                    }
                    if sw == 0 {
                        return Err(IrError::ZeroWidthDynamicIndex);
                    }
                    *width
                }
            };
            if inst.width != expected_w {
                return Err(IrError::WidthMismatch {
                    op: "target-inst-width",
                    left: inst.width,
                    right: expected_w,
                });
            }
            widths.push(inst.width);
        }
        let rw = widths
            .get(self.root.0)
            .copied()
            .ok_or(IrError::InvalidNodeId(NodeId(self.root.0)))?;
        if rw != self.output_width {
            return Err(IrError::WidthMismatch {
                op: "target-root-width",
                left: rw,
                right: self.output_width,
            });
        }
        Ok(())
    }

    /// Computes total program cost under `model`.
    pub fn cost(&self, model: &CostModel) -> u64 {
        self.instructions
            .iter()
            .map(|inst| model.op_cost(&inst.op, inst.width))
            .sum()
    }

    /// Returns `true` if all instructions in this program are supported by `self.profile`.
    pub fn is_legal_for_profile(&self) -> bool {
        self.instructions
            .iter()
            .all(|inst| self.profile.supports_op(&inst.op))
    }

    /// Evaluates the target program on concrete inputs, enforcing shift/slice definedness
    /// and preserving [`BehaviorBoundary`] execution order.
    pub fn eval(
        &self,
        inputs: &BTreeMap<InputIdentity, BitValue>,
    ) -> Result<EvalResult, EvalError> {
        self.validate()?;
        let mut trace = run_boundary_prefix(&self.boundary, &self.declared_inputs, inputs)?;
        if !self.domain.eval(inputs)? {
            return Err(EvalError::DomainViolation);
        }

        let mut regs: Vec<BitValue> = vec![BitValue::empty(); self.instructions.len()];
        for inst in &self.instructions {
            let val = match &inst.op {
                TargetOp::ZeroWidthEmpty { .. } => BitValue::empty(),
                TargetOp::Const { bits, .. } => bits.clone(),
                TargetOp::Input { width, identity } => {
                    let v = inputs
                        .get(identity)
                        .ok_or_else(|| IrError::MissingInput(identity.clone()))?;
                    if v.width() != *width {
                        return Err(IrError::InputWidthMismatch {
                            identity: identity.clone(),
                            expected: *width,
                            actual: v.width(),
                        }
                        .into());
                    }
                    v.clone()
                }
                TargetOp::Copy { src, .. } => regs[src.0].clone(),
                TargetOp::And { lhs, rhs, .. } => regs[lhs.0].bit_and(&regs[rhs.0])?,
                TargetOp::Or { lhs, rhs, .. } => regs[lhs.0].bit_or(&regs[rhs.0])?,
                TargetOp::Xor { lhs, rhs, .. } => regs[lhs.0].bit_xor(&regs[rhs.0])?,
                TargetOp::Not { src, .. } => regs[src.0].bit_not(),
                TargetOp::AndNot { lhs, inverted, .. } => {
                    regs[lhs.0].bit_and(&regs[inverted.0].bit_not())?
                }
                TargetOp::OrNot { lhs, inverted, .. } => {
                    regs[lhs.0].bit_or(&regs[inverted.0].bit_not())?
                }
                TargetOp::Equal { lhs, rhs, .. } => BitValue::from_bool(regs[lhs.0] == regs[rhs.0]),
                TargetOp::Concat { high, low, .. } => regs[high.0].concat(&regs[low.0])?,
                TargetOp::ExtractConst {
                    input,
                    start,
                    width,
                } => regs[input.0].extract(*start, *width)?,
                TargetOp::ExtractDynamic {
                    input,
                    input_width,
                    start,
                    width,
                } => {
                    let start_bv = &regs[start.0];
                    if *width > *input_width {
                        return Err(EvalError::DynamicSliceOutOfBounds {
                            input_width: *input_width,
                            slice_width: *width,
                            start_detail: start_bv.to_string(),
                        });
                    }
                    let max_bv = BitValue::from_u64(
                        start_bv.width().max(64),
                        u64::from(*input_width - *width),
                    )?;
                    if start_bv.cmp_unsigned(&max_bv) == std::cmp::Ordering::Greater {
                        return Err(EvalError::DynamicSliceOutOfBounds {
                            input_width: *input_width,
                            slice_width: *width,
                            start_detail: start_bv.to_string(),
                        });
                    }
                    let start_u32 = start_bv.words().first().copied().unwrap_or(0) as u32;
                    regs[input.0].extract(start_u32, *width)?
                }
                TargetOp::ZeroExtend {
                    input,
                    target_width,
                } => regs[input.0].zero_extend(*target_width)?,
                TargetOp::Bswap16 { input }
                | TargetOp::Bswap32 { input }
                | TargetOp::Bswap64 { input } => regs[input.0].bswap_bytes()?,
                TargetOp::RotrConst { input, amount, .. } => regs[input.0].rotr_const(*amount),
                TargetOp::RotlConst { input, amount, .. } => regs[input.0].rotl_const(*amount),
                TargetOp::ShlConst {
                    input,
                    width,
                    amount,
                } => {
                    if *width == 0 || *amount >= *width {
                        return Err(EvalError::UndefinedTargetOperation(format!(
                            "shl_const amount {amount} >= width {width}"
                        )));
                    }
                    regs[input.0].shl_const(*amount)?
                }
                TargetOp::LshrConst {
                    input,
                    width,
                    amount,
                } => {
                    if *width == 0 || *amount >= *width {
                        return Err(EvalError::UndefinedTargetOperation(format!(
                            "lshr_const amount {amount} >= width {width}"
                        )));
                    }
                    regs[input.0].lshr_const(*amount)?
                }
                TargetOp::Shl {
                    input,
                    width,
                    shift,
                } => {
                    let s = checked_shift_amount(&regs[shift.0], *width, "shl")?;
                    regs[input.0].shl_const(s)?
                }
                TargetOp::Lshr {
                    input,
                    width,
                    shift,
                } => {
                    let s = checked_shift_amount(&regs[shift.0], *width, "lshr")?;
                    regs[input.0].lshr_const(s)?
                }
                TargetOp::Rotr {
                    input,
                    width,
                    shift,
                } => {
                    let s = checked_shift_amount(&regs[shift.0], *width, "rotr")?;
                    regs[input.0].rotr_const(s)
                }
                TargetOp::Rotl {
                    input,
                    width,
                    shift,
                } => {
                    let s = checked_shift_amount(&regs[shift.0], *width, "rotl")?;
                    regs[input.0].rotl_const(s)
                }
            };
            regs[inst.dst.0] = val;
        }

        run_boundary_cleanups(&self.boundary, &mut trace);
        Ok(EvalResult {
            value: regs[self.root.0].clone(),
            trace,
        })
    }

    /// Rebinds canonicalized inputs/domain/boundary back onto a concrete [`Region`].
    pub fn rebind(&self, rename: &BTreeMap<InputIdentity, InputIdentity>, region: &Region) -> Self {
        let instructions = self
            .instructions
            .iter()
            .map(|inst| TargetInst {
                dst: inst.dst,
                width: inst.width,
                op: match &inst.op {
                    TargetOp::Input { width, identity } => TargetOp::Input {
                        width: *width,
                        identity: rename
                            .get(identity)
                            .cloned()
                            .unwrap_or_else(|| identity.clone()),
                    },
                    other => other.clone(),
                },
            })
            .collect();
        Self {
            profile: self.profile,
            declared_inputs: region.inputs.clone(),
            domain: region.domain.clone(),
            boundary: region.boundary.clone(),
            instructions,
            root: self.root,
            output_width: self.output_width,
            justifications: self.justifications.clone(),
        }
    }
}

impl Display for TargetProgram {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(target-program (profile {}) (root {}) (output-width {}) (instructions ",
            self.profile, self.root, self.output_width
        )?;
        write_list(f, &self.instructions)?;
        f.write_str(") (justifications ")?;
        write_list(f, &self.justifications)?;
        write!(f, ") (boundary {}))", self.boundary)
    }
}

fn checked_reg_width(widths: &[u32], current_idx: usize, reg: TargetReg) -> Result<u32, IrError> {
    if reg.0 >= current_idx {
        return Err(IrError::CyclicDag(NodeId(reg.0)));
    }
    widths
        .get(reg.0)
        .copied()
        .ok_or(IrError::InvalidNodeId(NodeId(reg.0)))
}

fn checked_shift_amount(shift_bv: &BitValue, width: u32, op_name: &str) -> Result<u32, EvalError> {
    if width == 0 {
        return Err(EvalError::UndefinedTargetOperation(format!(
            "{op_name} on zero-width bitvec"
        )));
    }
    let max_bv = BitValue::from_u64(shift_bv.width().max(64), u64::from(width))?;
    if shift_bv.cmp_unsigned(&max_bv) != std::cmp::Ordering::Less {
        return Err(EvalError::UndefinedTargetOperation(format!(
            "{op_name} shift amount {shift_bv} >= width {width}"
        )));
    }
    Ok(shift_bv.words().first().copied().unwrap_or(0) as u32)
}

/// Selected implementation delivered by the BitVec synthesizer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ChosenImplementation {
    Baseline(BaselineProgram),
    Target(TargetProgram),
}

impl ChosenImplementation {
    /// Evaluates the chosen implementation on concrete inputs.
    pub fn eval(
        &self,
        inputs: &BTreeMap<InputIdentity, BitValue>,
    ) -> Result<EvalResult, EvalError> {
        match self {
            Self::Baseline(prog) => prog.execute(inputs),
            Self::Target(prog) => prog.eval(inputs),
        }
    }

    /// Cost of the chosen implementation under `model`.
    pub fn cost(&self, model: &CostModel) -> u64 {
        match self {
            Self::Baseline(prog) => prog.cost(),
            Self::Target(prog) => prog.cost(model),
        }
    }

    /// Behavior boundary preserved by the chosen implementation.
    pub fn boundary(&self) -> &BehaviorBoundary {
        match self {
            Self::Baseline(prog) => &prog.boundary,
            Self::Target(prog) => &prog.boundary,
        }
    }
}

impl Display for ChosenImplementation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Baseline(prog) => write!(f, "(chosen-baseline {prog})"),
            Self::Target(prog) => write!(f, "(chosen-target {prog})"),
        }
    }
}

/// Explains why the synthesizer kept the verified baseline instead of a target candidate.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FallbackReason {
    SolverUnknown(Budget),
    NoCheaperVerifiedCandidate,
}

impl Display for FallbackReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SolverUnknown(budget) => write!(f, "(fallback-unknown {budget})"),
            Self::NoCheaperVerifiedCandidate => f.write_str("(fallback-no-cheaper-candidate)"),
        }
    }
}

/// Verification status of the delivered implementation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProofStatus {
    Verified(ProofCertificate),
    BaselineFallback(FallbackReason),
}

impl Display for ProofStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Verified(cert) => write!(f, "(status-verified {cert})"),
            Self::BaselineFallback(reason) => write!(f, "(status-baseline-fallback {reason})"),
        }
    }
}

/// Counters summarizing synthesis and verification effort.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SynthesisStats {
    pub candidates_considered: u64,
    pub counterexamples_pruned: u64,
    pub smt_queries: u64,
    pub cache_hit: bool,
}

impl Display for SynthesisStats {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(synth-stats (candidates {}) (cex-pruned {}) (smt-queries {}) (cache-hit {}))",
            self.candidates_considered,
            self.counterexamples_pruned,
            self.smt_queries,
            self.cache_hit
        )
    }
}

/// Result returned by [`synthesize`] / [`synthesize_with_budget`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SynthesisOutcome {
    pub implementation: ChosenImplementation,
    pub cost: u64,
    pub status: ProofStatus,
    pub stats: SynthesisStats,
}

impl Display for SynthesisOutcome {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(synthesis-outcome (cost {}) (status {}) (stats {}) (impl {}))",
            self.cost, self.status, self.stats, self.implementation
        )
    }
}

/// Errors encountered during synthesis or lowering.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SynthError {
    Ir(IrError),
    Eval(EvalError),
    Verify(VerifyError),
}

impl From<IrError> for SynthError {
    fn from(value: IrError) -> Self {
        Self::Ir(value)
    }
}

impl From<EvalError> for SynthError {
    fn from(value: EvalError) -> Self {
        Self::Eval(value)
    }
}

impl From<VerifyError> for SynthError {
    fn from(value: VerifyError) -> Self {
        Self::Verify(value)
    }
}

impl Display for SynthError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Ir(err) => write!(f, "(synth-error ir {err})"),
            Self::Eval(err) => write!(f, "(synth-error eval {err})"),
            Self::Verify(err) => write!(f, "(synth-error verify {err})"),
        }
    }
}

impl std::error::Error for SynthError {}

/// Cache key isolating synthesis proofs by canonical mapping, widths, domain,
/// target profile, cost model, and verifier version.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CacheKey {
    pub canonical_region: CanonicalRegion,
    pub input_widths: Vec<(InputIdentity, u32)>,
    pub output_width: u32,
    pub domain: DomainConstraint,
    pub target_profile: TargetProfile,
    pub cost_model: CostModel,
    pub verifier_version: &'static str,
}

impl CacheKey {
    /// Builds a deterministic cache key from a canonicalized region and target configuration.
    pub fn from_canonical(
        canonical_region: CanonicalRegion,
        target_profile: TargetProfile,
        cost_model: CostModel,
    ) -> Self {
        let input_widths = canonical_region
            .region
            .inputs
            .iter()
            .map(|(k, &v)| (k.clone(), v))
            .collect();
        let output_width = canonical_region.region.output_kind.bit_width();
        let domain = canonical_region.region.domain.clone();
        Self {
            canonical_region,
            input_widths,
            output_width,
            domain,
            target_profile,
            cost_model,
            verifier_version: VERIFIER_VERSION,
        }
    }
}

impl Display for CacheKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(cache-key (target {}) (output-width {}) (verifier {}) (cost-model {}) (domain {}) (region {}))",
            self.target_profile,
            self.output_width,
            Quoted(self.verifier_version),
            self.cost_model,
            self.domain,
            self.canonical_region
        )
    }
}

/// Cached verified synthesis entry stored in canonical input coordinates.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CachedEntry {
    pub canonical_program: TargetProgram,
    pub certificate: ProofCertificate,
    pub cost: u64,
}

impl Display for CachedEntry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(cached-entry (cost {}) (cert {}) (program {}))",
            self.cost, self.certificate, self.canonical_program
        )
    }
}

/// Deterministic synthesis proof cache backed by `BTreeMap`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SynthesisCache {
    entries: BTreeMap<CacheKey, CachedEntry>,
}

impl SynthesisCache {
    /// Creates an empty synthesis cache.
    pub fn new() -> Self {
        Self::default()
    }

    /// Looks up a cached verified entry.
    pub fn get(&self, key: &CacheKey) -> Option<&CachedEntry> {
        self.entries.get(key)
    }

    /// Inserts a verified entry into the cache.
    pub fn insert(&mut self, key: CacheKey, entry: CachedEntry) {
        self.entries.insert(key, entry);
    }

    /// Number of cached entries.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Returns `true` if the cache is empty.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Clears all cached entries.
    pub fn clear(&mut self) {
        self.entries.clear();
    }
}

impl Display for SynthesisCache {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("(synthesis-cache (")?;
        for (idx, (key, entry)) in self.entries.iter().enumerate() {
            if idx != 0 {
                f.write_char(' ')?;
            }
            write!(f, "(entry {key} {entry})")?;
        }
        f.write_str("))")
    }
}

/// Runs zero-width elimination, baseline lowering, rewrite/CEGIS synthesis, and
/// SAT verification using default budget and cost model.
pub fn synthesize(
    region: &Region,
    profile: TargetProfile,
    cache: &mut SynthesisCache,
) -> Result<SynthesisOutcome, SynthError> {
    synthesize_with_budget(
        region,
        profile,
        profile.default_cost_model(),
        Budget::default(),
        cache,
    )
}

/// Runs full BitVec optimization and formal verification within `budget`.
///
/// Unconditionally preserves correctness by falling back to the verified
/// [`BaselineProgram`] whenever SAT returns `Unknown(Budget)` or no cheaper
/// verified target candidate exists.
pub fn synthesize_with_budget(
    region: &Region,
    profile: TargetProfile,
    cost_model: CostModel,
    mut budget: Budget,
    cache: &mut SynthesisCache,
) -> Result<SynthesisOutcome, SynthError> {
    region.validate()?;
    let zw = eliminate_zero_width(region)?;
    let zw_region = zw.region;
    let baseline = lower_to_baseline(&zw_region, 64)?;
    let baseline_cost = baseline.cost();

    let (canon_region, canon_to_orig) = zw_region.canonicalize_with_map()?;
    let cache_key = CacheKey::from_canonical(canon_region.clone(), profile, cost_model);

    if let Some(cached) = cache.get(&cache_key) {
        let rebound = cached.canonical_program.rebind(&canon_to_orig, &zw_region);
        return Ok(SynthesisOutcome {
            implementation: ChosenImplementation::Target(rebound),
            cost: cached.cost,
            status: ProofStatus::Verified(cached.certificate.clone()),
            stats: SynthesisStats {
                candidates_considered: 0,
                counterexamples_pruned: 0,
                smt_queries: 0,
                cache_hit: true,
            },
        });
    }

    let mut best_target: Option<(TargetProgram, ProofCertificate, u64)> = None;
    let mut best_cost = baseline_cost;
    let mut stats = SynthesisStats::default();
    let mut counterexamples = seed_valid_examples(&zw_region, &mut budget)?;
    let mut hit_unknown: Option<Budget> = None;

    // Stage 1: Verified algebraic simplification + pattern-guided target lowering.
    let simplified = simplify_region(&zw_region)?;
    let mut candidates = Vec::new();
    candidates.push(lower_region_to_target(
        &simplified.region,
        &[],
        profile,
        &cost_model,
    )?);
    if !simplified.patterns.is_empty() {
        candidates.push(lower_region_to_target(
            &simplified.region,
            &simplified.patterns,
            profile,
            &cost_model,
        )?);
    }
    candidates.extend(generate_dynamic_slice_shift_candidates(
        &simplified.region,
        profile,
    )?);

    for cand in candidates {
        if !cand.is_legal_for_profile() {
            continue;
        }
        let cand_cost = cand.cost(&cost_model);
        if cand_cost >= best_cost {
            continue;
        }
        stats.candidates_considered += 1;
        if !passes_counterexamples(&cand, &counterexamples) {
            stats.counterexamples_pruned += 1;
            continue;
        }
        stats.smt_queries += 1;
        match verify_target_program(&zw_region, &cand, &mut budget)? {
            ProofOutcome::Equivalent(cert) => {
                best_cost = cand_cost;
                best_target = Some((cand, cert, cand_cost));
            }
            ProofOutcome::Counterexample(cex) => {
                record_counterexample(&zw_region, cex.inputs, &mut counterexamples);
            }
            ProofOutcome::Unknown(b) => {
                hit_unknown = Some(b);
                break;
            }
        }
    }

    // Stage 2: Enumerative CEGIS search for smaller target instruction sequences.
    if hit_unknown.is_none() && best_cost > 1 {
        let enum_candidates =
            enumerate_target_candidates(&zw_region, profile, &cost_model, best_cost);
        for cand in enum_candidates {
            let cand_cost = cand.cost(&cost_model);
            if cand_cost >= best_cost {
                continue;
            }
            stats.candidates_considered += 1;
            if !passes_counterexamples(&cand, &counterexamples) {
                stats.counterexamples_pruned += 1;
                continue;
            }
            stats.smt_queries += 1;
            match verify_target_program(&zw_region, &cand, &mut budget)? {
                ProofOutcome::Equivalent(cert) => {
                    best_cost = cand_cost;
                    best_target = Some((cand, cert, cand_cost));
                    if best_cost <= 1 {
                        break;
                    }
                }
                ProofOutcome::Counterexample(cex) => {
                    record_counterexample(&zw_region, cex.inputs, &mut counterexamples);
                }
                ProofOutcome::Unknown(b) => {
                    hit_unknown = Some(b);
                    break;
                }
            }
        }
    }

    if let Some((winning_prog, cert, winning_cost)) = best_target {
        // Store canonicalized verified winner in cache only if verified (never cache Unknown).
        let orig_to_canon: BTreeMap<InputIdentity, InputIdentity> = canon_to_orig
            .iter()
            .map(|(canon_id, orig_id)| (orig_id.clone(), canon_id.clone()))
            .collect();
        let canonical_program = winning_prog.rebind(&orig_to_canon, &canon_region.region);
        cache.insert(
            cache_key,
            CachedEntry {
                canonical_program,
                certificate: cert.clone(),
                cost: winning_cost,
            },
        );
        return Ok(SynthesisOutcome {
            implementation: ChosenImplementation::Target(winning_prog),
            cost: winning_cost,
            status: ProofStatus::Verified(cert),
            stats,
        });
    }

    let reason = match hit_unknown {
        Some(b) => FallbackReason::SolverUnknown(b),
        None => FallbackReason::NoCheaperVerifiedCandidate,
    };
    Ok(SynthesisOutcome {
        implementation: ChosenImplementation::Baseline(baseline),
        cost: baseline_cost,
        status: ProofStatus::BaselineFallback(reason),
        stats,
    })
}

#[derive(Clone)]
struct CexEntry {
    inputs: BTreeMap<InputIdentity, BitValue>,
    expected: BitValue,
}

fn seed_valid_examples(region: &Region, budget: &mut Budget) -> Result<Vec<CexEntry>, SynthError> {
    let mut examples = Vec::new();
    let mut candidates = Vec::new();

    let mut all_zero = BTreeMap::new();
    let mut all_ones = BTreeMap::new();
    let mut alt_55 = BTreeMap::new();
    for (id, &w) in &region.inputs {
        all_zero.insert(id.clone(), BitValue::zeros(w));
        all_ones.insert(id.clone(), BitValue::ones(w));
        let bits: Vec<bool> = (0..w).map(|i| i % 2 == 0).collect();
        alt_55.insert(id.clone(), BitValue::from_bits_lsb(&bits));
    }
    candidates.push(all_zero);
    candidates.push(all_ones);
    candidates.push(alt_55);

    for env in candidates {
        if region.domain.eval(&env).unwrap_or(false)
            && let Ok(res) = eval_region(region, &env)
        {
            examples.push(CexEntry {
                inputs: env,
                expected: res.value,
            });
        }
    }

    if examples.is_empty()
        && let Some(sat_env) = find_satisfying_inputs(region, budget)?
        && let Ok(res) = eval_region(region, &sat_env)
    {
        examples.push(CexEntry {
            inputs: sat_env,
            expected: res.value,
        });
    }

    Ok(examples)
}

fn record_counterexample(
    region: &Region,
    inputs: BTreeMap<InputIdentity, BitValue>,
    examples: &mut Vec<CexEntry>,
) {
    if inputs.len() < region.inputs.len() {
        return;
    }
    if region.domain.eval(&inputs).unwrap_or(false)
        && let Ok(res) = eval_region(region, &inputs)
    {
        examples.push(CexEntry {
            inputs,
            expected: res.value,
        });
    }
}

fn passes_counterexamples(cand: &TargetProgram, examples: &[CexEntry]) -> bool {
    for cex in examples {
        match cand.eval(&cex.inputs) {
            Ok(res) if res.value == cex.expected => continue,
            _ => return false,
        }
    }
    true
}

fn lower_region_to_target(
    region: &Region,
    patterns: &[RecognizedPattern],
    profile: TargetProfile,
    model: &CostModel,
) -> Result<TargetProgram, IrError> {
    let widths = region.node_widths()?;
    let mut best_pattern: BTreeMap<NodeId, &RecognizedPattern> = BTreeMap::new();
    for pat in patterns {
        if !pattern_supported_by_profile(&pat.kind, profile) {
            continue;
        }
        let replace = match best_pattern.get(&pat.node) {
            None => true,
            Some(prev) => {
                pattern_cost(&pat.kind, widths[pat.node.0], model)
                    < pattern_cost(&prev.kind, widths[pat.node.0], model)
            }
        };
        if replace {
            best_pattern.insert(pat.node, pat);
        }
    }

    let mut insts = Vec::new();
    let mut node_to_reg: Vec<Option<TargetReg>> = vec![None; region.nodes.len()];
    let mut justifications = Vec::new();

    let root_reg = emit_node_to_target(
        region,
        &widths,
        region.root,
        &best_pattern,
        &mut node_to_reg,
        &mut insts,
        &mut justifications,
    );

    Ok(TargetProgram {
        profile,
        declared_inputs: region.inputs.clone(),
        domain: region.domain.clone(),
        boundary: region.boundary.clone(),
        instructions: insts,
        root: root_reg,
        output_width: region.output_kind.bit_width(),
        justifications,
    })
}

fn emit_node_to_target(
    region: &Region,
    widths: &[u32],
    id: NodeId,
    best_pattern: &BTreeMap<NodeId, &RecognizedPattern>,
    memo: &mut [Option<TargetReg>],
    insts: &mut Vec<TargetInst>,
    justifications: &mut Vec<Justification>,
) -> TargetReg {
    if let Some(reg) = memo[id.0] {
        return reg;
    }
    let w = widths[id.0];

    if let Some(pat) = best_pattern.get(&id) {
        let op = match &pat.kind {
            PatternKind::Xor { lhs, rhs, width } => {
                let l = emit_node_to_target(
                    region,
                    widths,
                    *lhs,
                    best_pattern,
                    memo,
                    insts,
                    justifications,
                );
                let r = emit_node_to_target(
                    region,
                    widths,
                    *rhs,
                    best_pattern,
                    memo,
                    insts,
                    justifications,
                );
                TargetOp::Xor {
                    width: *width,
                    lhs: l,
                    rhs: r,
                }
            }
            PatternKind::Bswap16 { input } => {
                let src = emit_node_to_target(
                    region,
                    widths,
                    *input,
                    best_pattern,
                    memo,
                    insts,
                    justifications,
                );
                TargetOp::Bswap16 { input: src }
            }
            PatternKind::Bswap32 { input } => {
                let src = emit_node_to_target(
                    region,
                    widths,
                    *input,
                    best_pattern,
                    memo,
                    insts,
                    justifications,
                );
                TargetOp::Bswap32 { input: src }
            }
            PatternKind::Bswap64 { input } => {
                let src = emit_node_to_target(
                    region,
                    widths,
                    *input,
                    best_pattern,
                    memo,
                    insts,
                    justifications,
                );
                TargetOp::Bswap64 { input: src }
            }
            PatternKind::RotateRight {
                input,
                rotr_amount,
                width,
                ..
            } => {
                let src = emit_node_to_target(
                    region,
                    widths,
                    *input,
                    best_pattern,
                    memo,
                    insts,
                    justifications,
                );
                TargetOp::RotrConst {
                    input: src,
                    width: *width,
                    amount: *rotr_amount,
                }
            }
            PatternKind::ShlConst {
                input,
                amount,
                width,
            } => {
                let src = emit_node_to_target(
                    region,
                    widths,
                    *input,
                    best_pattern,
                    memo,
                    insts,
                    justifications,
                );
                TargetOp::ShlConst {
                    input: src,
                    width: *width,
                    amount: *amount,
                }
            }
            PatternKind::LshrConst {
                input,
                amount,
                width,
            } => {
                let src = emit_node_to_target(
                    region,
                    widths,
                    *input,
                    best_pattern,
                    memo,
                    insts,
                    justifications,
                );
                TargetOp::LshrConst {
                    input: src,
                    width: *width,
                    amount: *amount,
                }
            }
            PatternKind::AndNot {
                lhs,
                inverted,
                width,
            } => {
                let l = emit_node_to_target(
                    region,
                    widths,
                    *lhs,
                    best_pattern,
                    memo,
                    insts,
                    justifications,
                );
                let inv = emit_node_to_target(
                    region,
                    widths,
                    *inverted,
                    best_pattern,
                    memo,
                    insts,
                    justifications,
                );
                TargetOp::AndNot {
                    width: *width,
                    lhs: l,
                    inverted: inv,
                }
            }
            PatternKind::OrNot {
                lhs,
                inverted,
                width,
            } => {
                let l = emit_node_to_target(
                    region,
                    widths,
                    *lhs,
                    best_pattern,
                    memo,
                    insts,
                    justifications,
                );
                let inv = emit_node_to_target(
                    region,
                    widths,
                    *inverted,
                    best_pattern,
                    memo,
                    insts,
                    justifications,
                );
                TargetOp::OrNot {
                    width: *width,
                    lhs: l,
                    inverted: inv,
                }
            }
        };
        justifications.push(pat.justification.clone());
        let reg = TargetReg(insts.len());
        insts.push(TargetInst {
            dst: reg,
            width: w,
            op,
        });
        memo[id.0] = Some(reg);
        return reg;
    }

    let op = match &region.nodes[id.0] {
        Node::Input { width, identity } => TargetOp::Input {
            width: *width,
            identity: identity.clone(),
        },
        Node::Constant { width, bits } => {
            if *width == 0 {
                TargetOp::ZeroWidthEmpty { width: 0 }
            } else {
                TargetOp::Const {
                    width: *width,
                    bits: bits.clone(),
                }
            }
        }
        Node::Concat(hi, lo) => {
            let hr = emit_node_to_target(
                region,
                widths,
                *hi,
                best_pattern,
                memo,
                insts,
                justifications,
            );
            let lr = emit_node_to_target(
                region,
                widths,
                *lo,
                best_pattern,
                memo,
                insts,
                justifications,
            );
            TargetOp::Concat {
                high: hr,
                high_width: widths[hi.0],
                low: lr,
                low_width: widths[lo.0],
            }
        }
        Node::Extract {
            input,
            start,
            width,
        } => {
            let ir = emit_node_to_target(
                region,
                widths,
                *input,
                best_pattern,
                memo,
                insts,
                justifications,
            );
            match start {
                ExtractStart::Const(lo) => TargetOp::ExtractConst {
                    input: ir,
                    start: *lo,
                    width: *width,
                },
                ExtractStart::Dynamic(d) => {
                    let dr = emit_node_to_target(
                        region,
                        widths,
                        *d,
                        best_pattern,
                        memo,
                        insts,
                        justifications,
                    );
                    TargetOp::ExtractDynamic {
                        input: ir,
                        input_width: widths[input.0],
                        start: dr,
                        width: *width,
                    }
                }
            }
        }
        Node::And(l, r) => {
            let lr = emit_node_to_target(
                region,
                widths,
                *l,
                best_pattern,
                memo,
                insts,
                justifications,
            );
            let rr = emit_node_to_target(
                region,
                widths,
                *r,
                best_pattern,
                memo,
                insts,
                justifications,
            );
            TargetOp::And {
                width: w,
                lhs: lr,
                rhs: rr,
            }
        }
        Node::Or(l, r) => {
            let lr = emit_node_to_target(
                region,
                widths,
                *l,
                best_pattern,
                memo,
                insts,
                justifications,
            );
            let rr = emit_node_to_target(
                region,
                widths,
                *r,
                best_pattern,
                memo,
                insts,
                justifications,
            );
            TargetOp::Or {
                width: w,
                lhs: lr,
                rhs: rr,
            }
        }
        Node::Not(inner) => {
            let ir = emit_node_to_target(
                region,
                widths,
                *inner,
                best_pattern,
                memo,
                insts,
                justifications,
            );
            TargetOp::Not { width: w, src: ir }
        }
        Node::Equal(l, r) => {
            let lr = emit_node_to_target(
                region,
                widths,
                *l,
                best_pattern,
                memo,
                insts,
                justifications,
            );
            let rr = emit_node_to_target(
                region,
                widths,
                *r,
                best_pattern,
                memo,
                insts,
                justifications,
            );
            TargetOp::Equal {
                operand_width: widths[l.0],
                lhs: lr,
                rhs: rr,
            }
        }
        Node::ZeroExtend { input, width } => {
            let ir = emit_node_to_target(
                region,
                widths,
                *input,
                best_pattern,
                memo,
                insts,
                justifications,
            );
            TargetOp::ZeroExtend {
                input: ir,
                target_width: *width,
            }
        }
    };

    let reg = TargetReg(insts.len());
    insts.push(TargetInst {
        dst: reg,
        width: w,
        op,
    });
    memo[id.0] = Some(reg);
    reg
}

fn generate_dynamic_slice_shift_candidates(
    region: &Region,
    profile: TargetProfile,
) -> Result<Vec<TargetProgram>, IrError> {
    let mut out = Vec::new();
    let widths = region.node_widths()?;
    if let Node::Extract {
        input,
        start: ExtractStart::Dynamic(start_node),
        width,
    } = region.nodes[region.root.0]
        && let (
            Node::Input {
                width: in_w,
                identity: in_id,
            },
            Node::Input {
                width: st_w,
                identity: st_id,
            },
        ) = (&region.nodes[input.0], &region.nodes[start_node.0])
    {
        if width > 0 && *in_w >= width {
            let insts = vec![
                TargetInst {
                    dst: TargetReg(0),
                    width: *in_w,
                    op: TargetOp::Input {
                        width: *in_w,
                        identity: in_id.clone(),
                    },
                },
                TargetInst {
                    dst: TargetReg(1),
                    width: *st_w,
                    op: TargetOp::Input {
                        width: *st_w,
                        identity: st_id.clone(),
                    },
                },
                TargetInst {
                    dst: TargetReg(2),
                    width: *in_w,
                    op: TargetOp::Lshr {
                        input: TargetReg(0),
                        width: *in_w,
                        shift: TargetReg(1),
                    },
                },
                TargetInst {
                    dst: TargetReg(3),
                    width,
                    op: TargetOp::ExtractConst {
                        input: TargetReg(2),
                        start: 0,
                        width,
                    },
                },
            ];
            out.push(TargetProgram {
                profile,
                declared_inputs: region.inputs.clone(),
                domain: region.domain.clone(),
                boundary: region.boundary.clone(),
                instructions: insts,
                root: TargetReg(3),
                output_width: width,
                justifications: vec![Justification::SmtVerified {
                    solver: VERIFIER_VERSION,
                    summary: String::from("dynamic slice lowered to guarded lshr + low extract"),
                }],
            });
        }
    }
    let _ = widths;
    Ok(out)
}

fn enumerate_target_candidates(
    region: &Region,
    profile: TargetProfile,
    model: &CostModel,
    max_cost: u64,
) -> Vec<TargetProgram> {
    const MAX_CANDIDATES: usize = 384;
    let out_w = region.output_kind.bit_width();
    let mut candidates = Vec::new();
    let mut base_insts = Vec::new();
    let mut input_regs: Vec<(TargetReg, u32)> = Vec::new();

    for (id, &w) in &region.inputs {
        let reg = TargetReg(base_insts.len());
        base_insts.push(TargetInst {
            dst: reg,
            width: w,
            op: TargetOp::Input {
                width: w,
                identity: id.clone(),
            },
        });
        input_regs.push((reg, w));
    }

    let make_prog = |mut insts: Vec<TargetInst>, root: TargetReg, note: &str| TargetProgram {
        profile,
        declared_inputs: region.inputs.clone(),
        domain: region.domain.clone(),
        boundary: region.boundary.clone(),
        instructions: {
            let _ = &mut insts;
            insts
        },
        root,
        output_width: out_w,
        justifications: vec![Justification::SmtVerified {
            solver: VERIFIER_VERSION,
            summary: String::from(note),
        }],
    };

    if out_w == 0 {
        return candidates;
    }

    // Depth-0: single constant (0 or all-ones) or direct input pass-through (cost 0..=1).
    for c_val in [BitValue::zeros(out_w), BitValue::ones(out_w)] {
        let mut insts = base_insts.clone();
        let r = TargetReg(insts.len());
        insts.push(TargetInst {
            dst: r,
            width: out_w,
            op: TargetOp::Const {
                width: out_w,
                bits: c_val,
            },
        });
        let prog = make_prog(insts, r, "cegis constant synthesis");
        if prog.cost(model) < max_cost {
            candidates.push(prog);
        }
    }

    let same_w_regs: Vec<TargetReg> = input_regs
        .iter()
        .filter(|&&(_, w)| w == out_w)
        .map(|&(r, _)| r)
        .collect();

    for &r in &same_w_regs {
        let mut insts = base_insts.clone();
        let dst = TargetReg(insts.len());
        insts.push(TargetInst {
            dst,
            width: out_w,
            op: TargetOp::Copy {
                width: out_w,
                src: r,
            },
        });
        let prog = make_prog(insts, dst, "cegis identity projection");
        if prog.cost(model) < max_cost {
            candidates.push(prog);
        }
    }

    // Depth-1: unary & binary target operations over same-width inputs.
    for &r in &same_w_regs {
        let unary_ops: Vec<(TargetOp, &str)> = vec![
            (
                TargetOp::Not {
                    width: out_w,
                    src: r,
                },
                "cegis unary not",
            ),
            (TargetOp::Bswap16 { input: r }, "cegis bswap16"),
            (TargetOp::Bswap32 { input: r }, "cegis bswap32"),
            (TargetOp::Bswap64 { input: r }, "cegis bswap64"),
        ];
        for (op, note) in unary_ops {
            if !profile.supports_op(&op) {
                continue;
            }
            if matches!(op, TargetOp::Bswap16 { .. }) && out_w != 16 {
                continue;
            }
            if matches!(op, TargetOp::Bswap32 { .. }) && out_w != 32 {
                continue;
            }
            if matches!(op, TargetOp::Bswap64 { .. }) && out_w != 64 {
                continue;
            }
            let mut insts = base_insts.clone();
            let dst = TargetReg(insts.len());
            insts.push(TargetInst {
                dst,
                width: out_w,
                op,
            });
            let prog = make_prog(insts, dst, note);
            if prog.cost(model) < max_cost {
                candidates.push(prog);
            }
        }

        // Constant rotates and shifts.
        let mut amounts = Vec::new();
        for k in [1u32, 2, 4, 8, 12, 16, 24, 32] {
            if k < out_w {
                amounts.push(k);
            }
        }
        if out_w > 8 && out_w < 64 {
            for k in 1..out_w {
                if !amounts.contains(&k) && amounts.len() < 12 {
                    amounts.push(k);
                }
            }
        }
        for amt in amounts {
            for (op, note) in [
                (
                    TargetOp::RotrConst {
                        input: r,
                        width: out_w,
                        amount: amt,
                    },
                    "cegis rotr const",
                ),
                (
                    TargetOp::RotlConst {
                        input: r,
                        width: out_w,
                        amount: amt,
                    },
                    "cegis rotl const",
                ),
                (
                    TargetOp::ShlConst {
                        input: r,
                        width: out_w,
                        amount: amt,
                    },
                    "cegis shl const",
                ),
                (
                    TargetOp::LshrConst {
                        input: r,
                        width: out_w,
                        amount: amt,
                    },
                    "cegis lshr const",
                ),
            ] {
                if !profile.supports_op(&op) {
                    continue;
                }
                let mut insts = base_insts.clone();
                let dst = TargetReg(insts.len());
                insts.push(TargetInst {
                    dst,
                    width: out_w,
                    op,
                });
                let prog = make_prog(insts, dst, note);
                if prog.cost(model) < max_cost {
                    candidates.push(prog);
                }
            }
            if candidates.len() >= MAX_CANDIDATES {
                return candidates;
            }
        }
    }

    for (i, &lhs) in same_w_regs.iter().enumerate() {
        for (j, &rhs) in same_w_regs.iter().enumerate() {
            let mut ops: Vec<(TargetOp, &str)> = vec![
                (
                    TargetOp::AndNot {
                        width: out_w,
                        lhs,
                        inverted: rhs,
                    },
                    "cegis and-not",
                ),
                (
                    TargetOp::OrNot {
                        width: out_w,
                        lhs,
                        inverted: rhs,
                    },
                    "cegis or-not",
                ),
                (
                    TargetOp::Shl {
                        input: lhs,
                        width: out_w,
                        shift: rhs,
                    },
                    "cegis shl",
                ),
                (
                    TargetOp::Lshr {
                        input: lhs,
                        width: out_w,
                        shift: rhs,
                    },
                    "cegis lshr",
                ),
            ];
            if i < j {
                ops.push((
                    TargetOp::Xor {
                        width: out_w,
                        lhs,
                        rhs,
                    },
                    "cegis xor",
                ));
                ops.push((
                    TargetOp::And {
                        width: out_w,
                        lhs,
                        rhs,
                    },
                    "cegis and",
                ));
                ops.push((
                    TargetOp::Or {
                        width: out_w,
                        lhs,
                        rhs,
                    },
                    "cegis or",
                ));
            }
            for (op, note) in ops {
                if !profile.supports_op(&op) {
                    continue;
                }
                let mut insts = base_insts.clone();
                let dst = TargetReg(insts.len());
                insts.push(TargetInst {
                    dst,
                    width: out_w,
                    op,
                });
                let prog = make_prog(insts, dst, note);
                if prog.cost(model) < max_cost {
                    candidates.push(prog);
                }
                if candidates.len() >= MAX_CANDIDATES {
                    return candidates;
                }
            }
        }
    }

    candidates
}

fn pattern_supported_by_profile(kind: &PatternKind, profile: TargetProfile) -> bool {
    match kind {
        PatternKind::Xor { .. } | PatternKind::ShlConst { .. } | PatternKind::LshrConst { .. } => {
            true
        }
        PatternKind::AndNot { .. } => matches!(
            profile,
            TargetProfile::X86_64BmiBswap | TargetProfile::Aarch64
        ),
        PatternKind::OrNot { .. } => matches!(profile, TargetProfile::Aarch64),
        PatternKind::Bswap16 { .. }
        | PatternKind::Bswap32 { .. }
        | PatternKind::Bswap64 { .. }
        | PatternKind::RotateRight { .. } => {
            matches!(
                profile,
                TargetProfile::X86_64BmiBswap | TargetProfile::Aarch64
            )
        }
    }
}

fn pattern_cost(kind: &PatternKind, width: u32, model: &CostModel) -> u64 {
    let scale = width.div_ceil(64).max(1) as u64;
    match kind {
        PatternKind::Xor { .. } => model.xor_cost * scale,
        PatternKind::AndNot { .. } => model.and_not_cost * scale,
        PatternKind::OrNot { .. } => model.or_not_cost * scale,
        PatternKind::Bswap16 { .. } | PatternKind::Bswap32 { .. } | PatternKind::Bswap64 { .. } => {
            model.bswap_cost
        }
        PatternKind::RotateRight { .. } => model.rotate_cost * scale,
        PatternKind::ShlConst { .. } | PatternKind::LshrConst { .. } => model.shift_cost * scale,
    }
}
