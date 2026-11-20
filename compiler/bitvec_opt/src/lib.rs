//! Standalone BitVec algebra lowering, algebraic rewrite, bit-blasting SAT
//! verification, and target-aware CEGIS superoptimization crate.
//!
//! Strictly implements the compiler & synthesis architecture specified in
//! `docs/program-synthesis/bitvec-algebra.md`:
//! - [`ir`]: Pure finite-DAG BitVec IR, arbitrary-width [`BitValue`](ir::BitValue),
//!   [`DomainConstraint`](ir::DomainConstraint) (`P(x)`), and
//!   [`BehaviorBoundary`](ir::BehaviorBoundary) preserving evaluation order,
//!   memory versions, side effects, dynamic bounds checks, and cleanups.
//! - [`zero_width`]: `BitVec<0>` elimination pass preserving boundary contracts.
//! - [`baseline`]: Exact bit-level evaluator (`eval_region`) and `O(ceil(N/W))`
//!   machine-word baseline lowering (`BaselineProgram`) free of shift-by-`W` UB.
//! - [`rewrites`]: Verified algebraic rewrite rules and idiom recognizers with
//!   explicit [`Justification`](rewrites::Justification) certificates.
//! - [`smt`]: Self-contained Tseitin CNF bit-blaster and bounded CDCL SAT
//!   equivalence/definedness verifier.
//! - [`synth`]: Target-aware CEGIS + rewrite synthesizer, fallback to verified
//!   [`BaselineProgram`](baseline::BaselineProgram) on `Unknown(Budget)`, and
//!   [`SynthesisCache`](synth::SynthesisCache).

pub mod baseline;
pub mod ir;
pub mod rewrites;
pub mod sexpr;
pub mod smt;
pub mod synth;
pub mod zero_width;

pub use baseline::{
    BaselineOp, BaselineProgram, EvalError, EvalResult, TraceEvent, eval_region, eval_region_value,
    lower_to_baseline,
};
pub use ir::{
    BehaviorBoundary, BitValue, BoundaryCheck, BoundaryInput, CanonicalRegion, CleanupAction,
    CmpOp, DomainConstraint, DomainTerm, ExtractStart, InputEffect, InputIdentity, IrError, Node,
    NodeId, OutputKind, Region, RegionBuilder, top_word_mask,
};
pub use rewrites::{
    Justification, PatternKind, RecognizedPattern, RewriteResult, RewriteStep, recognize_patterns,
    simplify_region,
};
pub use smt::{
    Budget, Counterexample, CounterexampleReason, ProofCertificate, ProofOutcome, VerifyError,
    find_satisfying_inputs, half_open_to_inclusive_extract, verify_region_equivalence,
    verify_target_program,
};
pub use synth::{
    CacheKey, CachedEntry, ChosenImplementation, CostModel, FallbackReason, ProofStatus,
    SynthError, SynthesisCache, SynthesisOutcome, SynthesisStats, TargetInst, TargetOp,
    TargetProfile, TargetProgram, TargetReg, VERIFIER_VERSION, synthesize, synthesize_with_budget,
};
pub use zero_width::{ZeroWidthAction, ZeroWidthResult, eliminate_zero_width};
