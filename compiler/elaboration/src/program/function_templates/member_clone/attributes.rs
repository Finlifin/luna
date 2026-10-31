//! Attribute expressions retain their lexical lookup context.
use super::*;
use hir::{AttributeTarget as Target, DeferredDefinitionAttribute};

impl<'h> Rebind<'_, 'h> {
    pub(super) fn attributes(
        &mut self,
        member: OwnerId,
    ) -> Result<Vec<DeferredDefinitionAttribute<'h>>, Error> {
        let mut attributes = Vec::new();
        for attribute in &self.source.definition_attributes {
            self.charge(attribute.span, 0)?;
            let target = match attribute.target {
                Target::Definition(owner) if owner == member => Target::Definition(self.namespace),
                Target::Member(id) if id.owner == member => {
                    let mapped = self.bindings.get(&id).copied().ok_or_else(|| {
                        error(attribute.span, "generated attribute target was not cloned")
                    })?;
                    Target::Member(mapped)
                }
                _ => continue,
            };
            // Prefix expressions belong to the surrounding lexical scope.
            // Definition-local configurations instead follow the cloned owner.
            let (lexical_owner, value) = if attribute.lexical_owner == member {
                (self.namespace, self.expr(attribute.value, 0)?)
            } else {
                (attribute.lexical_owner, attribute.value)
            };
            let bytes = attribute
                .expression
                .len()
                .checked_add(attribute.configuration_key.as_ref().map_or(0, String::len))
                .ok_or_else(|| error(attribute.span, "generated attribute size overflow"))?;
            self.budget = self
                .budget
                .checked_sub(bytes)
                .ok_or_else(|| error(attribute.span, "generated attribute budget exhausted"))?;
            attributes.push(DeferredDefinitionAttribute {
                target,
                lexical_owner,
                value,
                expression: attribute.expression.clone(),
                configuration_key: attribute.configuration_key.clone(),
                span: attribute.span,
            });
        }
        Ok(attributes)
    }
}
