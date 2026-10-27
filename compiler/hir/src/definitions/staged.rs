//! Publish resolver identities for generated comptime parameter bindings.
use super::*;
use crate::HirId;
use std::collections::BTreeMap;

impl DefinitionBatch<'_, '_> {
    /// Replace draft registrations. Each entry must name a declared comptime
    /// parameter and carry a fresh identity in the ordinary definition namespace.
    pub fn set_staged_parameters(&mut self, parameters: Vec<(HirId, DefId)>) {
        for (_, identity) in &parameters {
            if identity.is_valid() {
                self.identity_next = self.identity_next.max(identity.index.saturating_add(1));
            }
        }
        self.staged_parameters = parameters;
    }

    pub(super) fn validate_staged_parameters(
        &self,
        fuel: &mut usize,
        identities: &mut BTreeSet<DefId>,
    ) -> Result<(), Error> {
        if self.definitions.is_empty() && self.staged_parameters.is_empty() {
            return Ok(());
        }
        // Source HIR exposes this table to lowering. Scan it once per publication
        // rather than treating its values as available definition identities.
        for identity in self.package.staged_parameter_definitions.values() {
            charge(fuel)?;
            identities.insert(*identity);
        }
        if self.staged_parameters.is_empty() {
            return Ok(());
        }
        let mut eligible = BTreeMap::new();
        for definition in self.definitions.iter().flatten() {
            charge(fuel)?;
            let parameters = match &definition.info.node.expect_item().kind {
                ItemKind::Fn(signature, _)
                | ItemKind::FnDeclaration(signature)
                | ItemKind::Effect(signature, _) => signature.params,
                ItemKind::Associated(definition) => definition.parameters,
                ItemKind::Deriver(definition) => definition.parameters,
                _ => &[],
            };
            for parameter in parameters {
                charge(fuel)?;
                eligible.insert(
                    (parameter.hir_id.owner, parameter.hir_id.local_id),
                    parameter.ty.is_comptime(),
                );
            }
        }
        let mut bindings = BTreeSet::new();
        for (binding, identity) in &self.staged_parameters {
            charge(fuel)?;
            let key = (binding.owner, binding.local_id);
            if eligible.get(&key) != Some(&true) {
                return Err(error(
                    "staged identity target is not a draft comptime parameter",
                    binding.owner,
                ));
            }
            if !bindings.insert(key)
                || self
                    .package
                    .staged_parameter_definitions
                    .contains_key(binding)
            {
                return Err(error("duplicate staged parameter binding", binding.owner));
            }
            if !identity.is_valid()
                || self.package.definition_owner(*identity).is_some()
                || self.package.clause_definitions.contains_key(identity)
                || !identities.insert(*identity)
            {
                return Err(error(
                    "invalid or duplicate staged parameter identity",
                    binding.owner,
                ));
            }
        }
        Ok(())
    }

    pub(super) fn publish_staged_parameters(&mut self) {
        for (binding, identity) in self.staged_parameters.drain(..) {
            self.package
                .reserve_definition_prefix(identity.index.saturating_add(1));
            self.package
                .staged_parameter_definitions
                .insert(binding, identity);
        }
    }
}
