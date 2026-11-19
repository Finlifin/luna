//! Stable semantic terms for normalization.
//!
//! This crate deliberately has no inference-variable or solver-state variant.
//! Elaboration must discharge those before publishing these terms. Structural
//! equality here is an interning/cache operation, not a complete type solver.

pub mod target;
mod type_children;
pub mod types;
pub use types::*;

pub mod enum_encoding;
pub mod float_formation;
pub mod integer_formation;
mod nominal_parameters;
pub mod pattern_structure;
pub mod program;
mod trait_parameters;
pub use trait_parameters::{
    validate_record_index_with, validate_record_index_with_admission,
    validate_record_index_with_admission_in,
};

pub mod member_types;
pub mod phases;

pub mod callable_types;
pub mod function_parameters;
pub mod normal_forms;

mod display;
pub use display::QuotedDisplay;

pub mod callable_source;
pub mod contracts;

pub mod associated_initializers;
pub mod completed_values;
pub mod comptime_values;
pub mod objects;
pub mod scalar_construction;
pub mod value_admission;

pub mod effects;
pub mod error_results;

pub mod free_bindings;
pub mod preparation;
pub mod type_computations;
pub mod visit;

pub mod deferred_signatures;

pub mod inline_return;

pub mod newtypes;

pub mod described_applications;
pub mod refinements;
pub mod type_aliases;

pub mod code_interfaces;
pub mod code_values;

pub use nominal_parameters::{
    AdmissionError as NominalAdmissionError, AdmittedArguments, DeclarationAdmissionFailure,
    DeclarationInputScope, DeclarationSubstitution, ExpressionInstantiationError,
    InputAdmissionStage, admit_in_order as admit_declaration_inputs,
    admit_in_scope as admit_declaration_arguments_in_scope,
    admit_projected_in_scope as admit_projected_declaration_arguments_in_scope,
    admit_with_collections_in as admit_declaration_arguments_with_collections_in,
    admit_with_evidence_in as admit_declaration_arguments_with_evidence_in,
    admit_with_queries_in as admit_declaration_arguments_with_queries_in,
    binders as declaration_parameters, check_observed_inputs, instantiate_observed_domain,
    validate_completed as validate_declaration_arguments,
    validate_completed_in as validate_declaration_arguments_in,
    validate_completed_with_layouts as validate_declaration_arguments_with_layouts,
    validate_completed_with_layouts_in as validate_declaration_arguments_with_layouts_in,
    validate_with_collections_in as validate_declaration_arguments_with_collections_in,
    validate_with_evidence as validate_declaration_arguments_with_evidence,
    validate_with_evidence_in as validate_declaration_arguments_with_evidence_in,
};

pub mod copy_budget;

pub mod call_safety;

pub mod intrinsics;

pub mod layout;
pub use layout::{
    LayoutError, LayoutWorld, NominalLayout, TypeLayout, TypeLayoutQuery, compute_layout,
};

pub mod type_budget;

pub mod freshening;

mod method_identity;
pub use method_identity::{CallableValueRef, TraitMethodIdentity};

pub mod declaration_fields;
