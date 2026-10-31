//! Immutable method binders with contract references resolved in the source slice.
use super::*;

pub(super) struct Entry {
    pub(super) binders: Vec<nir::DeclarationParameter>,
    pub(super) interface: usize,
    pub(super) member: usize,
}
pub(super) type Entries = HashMap<(DeclarationId, DeclarationId), Entry>;
#[derive(Default)]
pub(super) struct Contracts<'a> {
    pub(super) traits: &'a [nir::program::Trait],
    pub(super) entries: Arc<Entries>,
}
impl<'a> Contracts<'a> {
    pub(super) fn get(
        &self,
        key: &(DeclarationId, DeclarationId),
    ) -> Option<(
        &Vec<nir::DeclarationParameter>,
        &'a nir::program::MethodContract,
    )> {
        let entry = self.entries.get(key)?;
        Some((
            &entry.binders,
            &self.traits[entry.interface].methods[entry.member],
        ))
    }
}
