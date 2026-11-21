//! HIR elaboration and language protocol coordination.

pub use type_resolution::{InferenceContext, InferenceError, InferenceTerm, MetaId};
pub use type_resolution::{associated, calls, capabilities, implementation, matching, obligations};

pub mod definitions;
pub mod materialization;
pub mod program;

pub mod derivation;

// Preserve downstream Rust paths while the solver has its own build unit.
pub use ::ptc;

pub(crate) mod automata;
mod coverage;

mod bit_coverage;
mod float_coverage;
mod interval_coverage;
mod machine_float_coverage;
mod symbol_coverage;

/// Program-state arithmetic reasoning, separate from the type proposition engine.
pub use ::program_verification as verification;

pub mod effects;

pub mod splice;

pub mod key_protocols;

pub mod arithmetic;

// The gradual compatibility/refinement model has no production consumer.
// Source checking owns its observations separately from completed Type data.
#[cfg(test)]
#[path = "../test_support/gradual.rs"]
#[allow(dead_code)]
mod gradual;

// The old inference-output scheduler has no frontend consumer. Its dependency
// and cancellation contracts remain exercised by shared test support only.
#[cfg(test)]
#[path = "../test_support/type_computations/mod.rs"]
#[allow(dead_code)]
mod type_computations;

pub mod indexing;
pub mod list_patterns;

pub mod literal_conversion;

/// Finite by-value storage checks shared by source and generated declarations.
pub mod representations;
