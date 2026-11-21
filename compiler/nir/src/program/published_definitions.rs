use super::{Enum, Package, Struct};
use std::sync::Arc;

/// Concrete declarations admitted by a builder host, grouped by nominal kind.
#[derive(Default)]
pub struct PublishedDefinitions {
    pub structs: Vec<Arc<[Struct]>>,
    pub enums: Vec<Arc<[Enum]>>,
    pub newtypes: Vec<Arc<[crate::newtypes::Newtype]>>,
}

impl PublishedDefinitions {
    pub fn is_empty(&self) -> bool {
        self.structs.is_empty() && self.enums.is_empty() && self.newtypes.is_empty()
    }

    pub fn declarations(&self) -> impl Iterator<Item = crate::DeclarationId> + '_ {
        self.structs
            .iter()
            .flat_map(|b| b.iter().map(|d| d.declaration))
            .chain(
                self.enums
                    .iter()
                    .flat_map(|b| b.iter().map(|d| d.declaration)),
            )
            .chain(
                self.newtypes
                    .iter()
                    .flat_map(|b| b.iter().map(|d| d.declaration)),
            )
    }

    /// Preflight the owned copies needed to install these descriptions.
    pub fn charge_copy(
        &self,
        fuel: &mut usize,
    ) -> Result<(), crate::copy_budget::CopyBudgetExhausted> {
        *fuel = fuel
            .checked_sub(self.structs.len())
            .and_then(|f| f.checked_sub(self.enums.len()))
            .and_then(|f| f.checked_sub(self.newtypes.len()))
            .ok_or(crate::copy_budget::CopyBudgetExhausted::Steps)?;
        crate::copy_budget::charge_representation_metadata(
            self.structs.iter().flat_map(|b| b.iter()),
            self.newtypes.iter().flat_map(|b| b.iter()),
            self.enums.iter().flat_map(|b| b.iter()),
            fuel,
        )
    }

    /// Transfer completed builder products to the package returned by an
    /// invocation. Source checking has a separate transactional receiver.
    pub fn append_to(self, package: &mut Package) {
        use crate::normal_forms::NominalValueDeclaration as D;
        for batch in self.structs {
            for def in batch.iter() {
                package
                    .formed_nullary_nominals
                    .admit_checked(D::Struct(def));
            }
            package.structs.extend(batch.iter().cloned());
        }
        for batch in self.enums {
            for def in batch.iter() {
                package.formed_nullary_nominals.admit_checked(D::Enum(def));
            }
            package.enums.extend(batch.iter().cloned());
        }
        for batch in self.newtypes {
            for def in batch.iter() {
                package
                    .formed_nullary_nominals
                    .admit_checked(D::Newtype(def));
            }
            package.newtypes.extend(batch.iter().cloned());
        }
    }
}

impl std::fmt::Display for PublishedDefinitions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("(published-definitions (structs")?;
        for item in self.structs.iter().flat_map(|batch| batch.iter()) {
            write!(f, " {item}")?;
        }
        f.write_str(") (enums")?;
        for item in self.enums.iter().flat_map(|batch| batch.iter()) {
            write!(f, " {item}")?;
        }
        f.write_str(") (newtypes")?;
        for item in self.newtypes.iter().flat_map(|batch| batch.iter()) {
            write!(f, " {item}")?;
        }
        f.write_str("))")
    }
}
