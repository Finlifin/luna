//! Semantic declaration lookup bound to one immutable source package view.
use super::*;

pub(super) struct SourceDeclarations<'a, 'h> {
    package: &'a hir::Package<'h>,
    owners: BTreeMap<DeclarationId, OwnerId>,
    pub(super) nominal_context: crate::ptc::Context,
}

impl<'a, 'h> SourceDeclarations<'a, 'h> {
    pub(super) fn new(
        package: &'a hir::Package<'h>,
        identities: &BTreeMap<u32, u64>,
        fuel: &mut u64,
    ) -> Result<Self, Error> {
        // owners() also walks vacant dense slots. Account for the full source
        // view before iterating or allocating any lookup entries.
        *fuel = fuel.checked_sub(package.num_defs() as u64).ok_or_else(|| {
            error(
                rustc_span::DUMMY_SP,
                "source declaration index budget exhausted",
            )
        })?;
        let mut owners = BTreeMap::new();
        let mut nominal_context = crate::ptc::Context::default();
        for (owner, info) in package.owners() {
            let item = info.node.expect_item();
            // Imports have syntax owners, not a new declaration identity. Their
            // dense indices can coincide with resolver declaration indices.
            if matches!(item.kind, ItemKind::Use(_)) {
                continue;
            }
            let identity = declaration(package, identities, owner);
            if matches!(
                item.kind,
                ItemKind::Struct(_) | ItemKind::Enum(_) | ItemKind::Newtype(..)
            ) {
                nominal_context.admit_nominal_constructor(identity);
            }
            if owners.insert(identity, owner).is_some() {
                return Err(error(
                    item.span,
                    format!("duplicate source declaration identity {identity}"),
                ));
            }
        }
        Ok(Self {
            package,
            owners,
            nominal_context,
        })
    }

    pub(super) fn owner(&self, identity: DeclarationId) -> Option<OwnerId> {
        self.owners.get(&identity).copied()
    }

    pub(super) fn item(&self, identity: DeclarationId) -> Option<&'h hir::Item<'h>> {
        self.package.item(self.owner(identity)?)
    }
}

impl std::fmt::Display for SourceDeclarations<'_, '_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("(source-declarations")?;
        for (identity, owner) in &self.owners {
            write!(
                f,
                " (declaration {identity} (owner {}))",
                owner.def_id.raw()
            )?;
        }
        f.write_str(")")
    }
}

#[cfg(test)]
mod tests;
