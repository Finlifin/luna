//! Concrete newtype requests for checked host publication.
use super::*;

#[derive(Debug)]
pub struct NewtypeBuildRequest {
    pub name: String,
    pub representation: Term,
    pub source: SourceRange,
}
impl NewtypeBuildRequest {
    /// Reserve descriptor assembly before copying its name or representation.
    pub fn charge_assembly(&self, fuel: &mut usize) -> Result<(), ()> {
        *fuel = fuel
            .checked_sub(self.name.len().saturating_add(1))
            .ok_or(())?;
        nir::type_computations::charge_term(&self.representation, fuel).map_err(|_| ())
    }
}
impl fmt::Display for NewtypeBuildRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(newtype-build (name {}) (representation {}))",
            nir::QuotedDisplay(&self.name),
            self.representation
        )
    }
}

/// The publisher owns identity allocation and ordinary declaration checking.
/// Commit must be transactional; dropping an uncommitted candidate publishes
/// nothing. VM admission separately checks the exact retained descriptors.
pub struct PreparedNewtypeBuild<'a> {
    pub definitions: Arc<[nir::newtypes::Newtype]>,
    pub result: DeclarationId,
    pub commit: Box<dyn FnOnce() -> Result<(), Error> + 'a>,
}
