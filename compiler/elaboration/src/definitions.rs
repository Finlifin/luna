//! Semantic admission of generated definition batches.
//!
//! HIR batches use ordinary elaboration in fresh candidate state. Concrete,
//! body-free structs can instead use completed-Type and representation checks
//! against a checked snapshot. Active source-checker registration is separate.
use crate::program::{self, Protocols};
use hir::definitions::{DefinitionBatch, PreparedPublication, PublicationError};
use std::collections::BTreeMap;
mod allocated_struct_publisher;
mod checked_struct_publisher;
mod deferred_struct_publisher;
mod struct_publisher;
pub use allocated_struct_publisher::AllocatedStructPublisher;
pub use checked_struct_publisher::CheckedStructPublisher;
pub use deferred_struct_publisher::{DeferredStructPublisher, StructBuildService};
pub use struct_publisher::StructPublisher;

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
    Ok(prepare_with_invocation_budget(
        batch,
        structural_fuel,
        invocation_steps,
        package_identities,
        protocols,
        target,
    )?
    .commit())
}

/// Prepare a semantic candidate once, retaining the original draft until the
/// caller coordinates external admission and explicitly commits the result.
pub fn prepare_with_invocation_budget<'batch, 'package, 'hir>(
    batch: &'batch mut DefinitionBatch<'package, 'hir>,
    structural_fuel: &mut usize,
    invocation_steps: &mut u64,
    package_identities: &BTreeMap<u32, u64>,
    protocols: &Protocols,
    target: Option<nir::target::TargetData>,
) -> Result<
    PreparedPublication<'batch, 'package, 'hir, nir::program::Package>,
    PublicationError<program::Error>,
> {
    batch.prepare_checked(structural_fuel, |candidate| {
        program::elaborate_packages_with_invocation_budget(
            candidate,
            package_identities,
            protocols,
            target,
            invocation_steps,
        )
    })
}
