//! Concrete enum construction requests for checked host publication.
use super::*;

/// Ordinary concrete variants preserve their source shape and payload order.
/// Generic and encoded enum construction require separate publication contracts.
#[derive(Debug)]
pub struct EnumBuildRequest {
    pub name: String,
    pub groups: Vec<String>,
    pub variants: Vec<nir::program::Variant>,
    pub source: SourceRange,
}
impl EnumBuildRequest {
    /// Charge descriptor assembly before cloning names or nested payload types.
    pub fn charge_assembly(&self, fuel: &mut usize) -> Result<(), ()> {
        let charge = |fuel: &mut usize, work: usize| {
            *fuel = fuel.checked_sub(work).ok_or(())?;
            Ok::<_, ()>(())
        };
        charge(fuel, self.name.len().saturating_add(1))?;
        for group in &self.groups {
            charge(fuel, group.len().saturating_add(1))?;
        }
        for variant in &self.variants {
            charge(fuel, variant.name.len().saturating_add(1))?;
            if let nir::program::VariantShape::Named(names) = &variant.shape {
                for name in names {
                    charge(fuel, name.len().saturating_add(1))?;
                }
            }
            for field in &variant.fields {
                nir::type_computations::charge_term(field, fuel).map_err(|_| ())?;
            }
        }
        Ok(())
    }
}
impl fmt::Display for EnumBuildRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(enum-build (name {}) (groups",
            nir::QuotedDisplay(&self.name)
        )?;
        for group in &self.groups {
            write!(f, " {}", nir::QuotedDisplay(group))?;
        }
        f.write_str(") (variants")?;
        for variant in &self.variants {
            write!(
                f,
                " (variant (name {}) (shape ",
                nir::QuotedDisplay(&variant.name)
            )?;
            match &variant.shape {
                nir::program::VariantShape::Unit => f.write_str("unit")?,
                nir::program::VariantShape::Tuple => f.write_str("tuple")?,
                nir::program::VariantShape::Named(names) => {
                    f.write_str("(named")?;
                    for name in names {
                        write!(f, " {}", nir::QuotedDisplay(name))?;
                    }
                    f.write_str(")")?;
                }
            }
            f.write_str(") (fields")?;
            for field in &variant.fields {
                write!(f, " {field}")?;
            }
            f.write_str("))")?;
        }
        f.write_str("))")
    }
}

/// The enclosing host owns identity reservation and checked publication. The
/// live VM must admit this exact batch before calling commit. Dropping abandons
/// the request; commit errors must leave the host publication unchanged.
pub struct PreparedEnumBuild<'a> {
    pub definitions: Arc<[nir::program::Enum]>,
    pub result: DeclarationId,
    pub commit: Box<dyn FnOnce() -> Result<(), Error> + 'a>,
}
