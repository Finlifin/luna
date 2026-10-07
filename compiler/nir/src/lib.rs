//! Stable semantic terms for normalization.
//!
//! This crate deliberately has no inference-variable or solver-state variant.
//! Elaboration must discharge those before publishing these terms. Structural
//! equality here is an interning/cache operation, not a complete type solver.

pub mod types;
pub use types::*;
