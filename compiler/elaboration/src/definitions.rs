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
    batch.publish_checked(structural_fuel, |candidate| {
        program::elaborate_packages_for_target(candidate, package_identities, protocols, target)
    })
}
