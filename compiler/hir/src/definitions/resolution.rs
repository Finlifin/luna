//! Resolver evidence travels with a generated definition transaction.
use super::*;
use crate::HirId;
use std::sync::Arc;

/// Resolution evidence for one cloned use site. Definition identities inside
/// candidate lists must already refer to the generated or external definitions.
#[derive(Clone, Debug, Default)]
pub struct ResolutionEvidence {
    /// Original definition, intentionally not remapped to a generated owner.
    pub source_origin: Option<DefId>,
    pub trait_sources: Option<Arc<crate::trait_sources::TraitSourcePlan>>,
    pub projection: Option<DefId>,
    pub deferred: Option<Arc<[(DefId, DefId)]>>,
    pub access: Option<Arc<[(DefId, DefId)]>>,
}

impl DefinitionBatch<'_, '_> {
    /// Replace the complete draft evidence. Replacing allows a failed draft to
    /// be repaired without changing any previously published use-site metadata.
    pub fn set_resolution(&mut self, evidence: Vec<(HirId, ResolutionEvidence)>) {
        self.resolution = evidence;
    }

    pub(super) fn validate_resolution(&self, fuel: &mut usize) -> Result<(), Error> {
        let mut seen = BTreeSet::new();
        for (id, evidence) in &self.resolution {
            charge(fuel)?;
            if self
                .slot(id.owner)
                .and_then(|i| self.definitions[i].as_ref())
                .is_none()
            {
                return Err(error(
                    "resolution target not defined in this batch",
                    id.owner,
                ));
            }
            if !seen.insert((id.owner, id.local_id)) {
                return Err(error("duplicate resolution target", id.owner));
            }
            if self.package.member_projections.contains_key(id)
                || self.package.deferred_member_projections.contains_key(id)
                || self.package.member_access.contains_key(id)
                || self.package.source_origins.contains_key(id)
                || self.package.trait_sources.contains_key(id)
            {
                return Err(error("resolution target already published", id.owner));
            }
            if let Some(plan) = &evidence.trait_sources {
                plan.charge(fuel)
                    .map_err(|message| error(message, id.owner))?;
                for name in &plan.names {
                    if matches!(name.fallback, Ok(Some(definition)) if !definition.is_valid()) {
                        return Err(error("invalid trait source definition", id.owner));
                    }
                }
            }
            if evidence.source_origin.is_some_and(|id| !id.is_valid()) {
                return Err(error("invalid source origin", id.owner));
            }
            if evidence.projection.is_some_and(|id| !id.is_valid()) {
                return Err(error("invalid resolution definition", id.owner));
            }
            for pairs in [&evidence.deferred, &evidence.access].into_iter().flatten() {
                for (container, member) in pairs.iter() {
                    charge(fuel)?;
                    if !container.is_valid() || !member.is_valid() {
                        return Err(error("invalid resolution candidate", id.owner));
                    }
                }
            }
        }
        Ok(())
    }

    pub(super) fn publish_resolution(&mut self) {
        for (id, evidence) in self.resolution.drain(..) {
            if let Some(plan) = evidence.trait_sources {
                self.package.trait_sources.insert(id, plan);
            }
            if let Some(origin) = evidence.source_origin {
                self.package.source_origins.insert(id, origin);
            }
            if let Some(value) = evidence.projection {
                self.package.member_projections.insert(id, value);
            }
            if let Some(value) = evidence.deferred {
                self.package.deferred_member_projections.insert(id, value);
            }
            if let Some(value) = evidence.access {
                self.package.member_access.insert(id, value);
            }
        }
    }
}

impl fmt::Display for ResolutionEvidence {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("(resolution-evidence (projection ")?;
        match self.projection {
            Some(id) => write!(f, "(def {} {})", id.pkg, id.index)?,
            None => f.write_str("none")?,
        }
        f.write_str(") (source-origin ")?;
        match self.source_origin {
            Some(id) => write!(f, "(def {} {})", id.pkg, id.index)?,
            None => f.write_str("none")?,
        }
        f.write_str(")")?;
        for (label, pairs) in [("deferred", &self.deferred), ("access", &self.access)] {
            write!(f, " ({label}")?;
            match pairs {
                None => f.write_str(" none")?,
                Some(pairs) => {
                    f.write_str(" (candidates")?;
                    for (container, member) in pairs.iter() {
                        write!(
                            f,
                            " (pair (def {} {}) (def {} {}))",
                            container.pkg, container.index, member.pkg, member.index
                        )?;
                    }
                    f.write_str(")")?;
                }
            }
            f.write_str(")")?;
        }
        if let Some(plan) = &self.trait_sources {
            write!(f, " (trait-sources {plan})")?;
        }
        f.write_str(")")
    }
}
