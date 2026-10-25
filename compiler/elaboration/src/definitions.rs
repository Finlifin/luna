//! Semantic admission of generated definition batches.
//!
//! This boundary runs the ordinary package elaborator in fresh candidate state.
//! It does not yet connect a source-level builder or resume a suspended VM.
use crate::program::{self, Protocols};
use hir::definitions::{DefinitionBatch, PublicationError};
use std::collections::BTreeMap;

/// Publish only after the complete candidate passes ordinary elaboration with
/// the caller's package identities, protocols and target. Return the checked
/// NIR together with the installed owner IDs, so callers need not re-elaborate.
/// Existing checked output is never mutated on rejection.
pub fn publish(
    batch: &mut DefinitionBatch<'_, '_>,
    structural_fuel: &mut usize,
    package_identities: &BTreeMap<u32, u64>,
    protocols: &Protocols,
    target: Option<nir::target::TargetData>,
) -> Result<(Vec<hir::OwnerId>, nir::program::Package), PublicationError<program::Error>> {
    publish_with_invocation_budget(
        batch,
        structural_fuel,
        &mut 1_000_000,
        package_identities,
        protocols,
        target,
    )
}

/// Reuse the caller's required invocation budget across publication attempts.
/// Structural rejection does not enter elaboration or consume invocation fuel.
/// Semantic errors retain consumption while preserving the draft and package.
/// Other analyses retain their own limits; this does not resume a live VM.
pub fn publish_with_invocation_budget(
    batch: &mut DefinitionBatch<'_, '_>,
    structural_fuel: &mut usize,
    invocation_steps: &mut u64,
    package_identities: &BTreeMap<u32, u64>,
    protocols: &Protocols,
    target: Option<nir::target::TargetData>,
) -> Result<(Vec<hir::OwnerId>, nir::program::Package), PublicationError<program::Error>> {
    batch.publish_checked(structural_fuel, |candidate| {
        program::elaborate_packages_with_invocation_budget(
            candidate,
            package_identities,
            protocols,
            target,
            invocation_steps,
        )
    })
}
