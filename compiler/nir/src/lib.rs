//! Stable semantic terms for normalization.
//!
//! This crate deliberately has no inference-variable or solver-state variant.
//! Elaboration must discharge those before publishing these terms. Structural
//! equality here is an interning/cache operation, not a complete type solver.

pub mod types;
pub use types::*;

pub mod enum_encoding;
pub mod program;
mod trait_parameters;

pub mod phases;

pub mod normal_forms;

mod display;

pub mod contracts;

pub mod completed_values;

pub mod effects;

pub mod free_bindings;
pub mod preparation;
pub mod type_computations;
pub mod visit;

pub mod deferred_signatures;

pub mod inline_return;

pub mod newtypes;
