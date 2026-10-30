//! Stable semantic terms for normalization.
//!
//! This crate deliberately has no inference-variable or solver-state variant.
//! Elaboration must discharge those before publishing these terms. Structural
//! equality here is an interning/cache operation, not a complete type solver.

pub mod target;
pub mod types;
pub use types::*;

pub mod enum_encoding;
pub mod float_formation;
pub mod integer_formation;
mod nominal_parameters;
pub mod program;
mod trait_parameters;
pub use trait_parameters::{
    validate_record_index_with, validate_record_index_with_admission,
    validate_record_index_with_admission_in,
};

pub mod phases;

pub mod function_parameters;
pub mod normal_forms;

mod display;
pub use display::QuotedDisplay;

pub mod contracts;

pub mod associated_initializers;
pub mod completed_values;
pub mod comptime_values;
pub mod objects;

pub mod effects;
pub mod error_results;

pub mod free_bindings;
pub mod preparation;
pub mod type_computations;
pub mod visit;

pub mod deferred_signatures;

pub mod inline_return;

pub mod newtypes;

pub mod type_aliases;

pub mod code_interfaces;
pub mod code_values;

pub use nominal_parameters::{
    AdmissionError as NominalAdmissionError, AdmittedArguments, DeclarationSubstitution,
    admit_with_evidence_in as admit_declaration_arguments_with_evidence_in,
    binders as declaration_parameters, validate_completed as validate_declaration_arguments,
    validate_completed_in as validate_declaration_arguments_in,
    validate_completed_with_layouts as validate_declaration_arguments_with_layouts,
    validate_completed_with_layouts_in as validate_declaration_arguments_with_layouts_in,
    validate_with_evidence as validate_declaration_arguments_with_evidence,
    validate_with_evidence_in as validate_declaration_arguments_with_evidence_in,
};

pub mod copy_budget;

pub mod call_safety;

pub mod intrinsics;

pub mod type_budget;

pub mod freshening;
