//! Preserve contribution environments until shared associated values agree.
use super::*;

impl Checker<'_, '_> {
    pub(super) fn prepare_complementary_associated(
        &mut self,
        contributions: &[n::Implementation],
        relationships: &BTreeMap<DeclarationId, DeclarationId>,
    ) -> Result<(), Error> {
        let definitions: BTreeMap<_, _> = contributions
            .iter()
            .map(|item| (item.declaration, item))
            .collect();
        let mut merged = BTreeMap::new();
        let mut alternatives = Vec::new();
        for ((own, member), (owner, parameters, arguments)) in &self.associated_initializers {
            let canonical = relationships.get(own).copied().unwrap_or(*own);
            let span = self.package.item(*owner).unwrap().span;
            self.required_steps = self
                .required_steps
                .checked_sub(1 + parameters.len() as u64 + arguments.len() as u64)
                .ok_or_else(|| error(span, "associated contribution merge budget exhausted"))?;
            for argument in arguments {
                let mut fuel = self.required_steps.min(100_000) as usize;
                let before = fuel;
                let result = nir::type_computations::charge_term(argument, &mut fuel);
                self.required_steps -= (before - fuel) as u64;
                result.map_err(|e| error(span, e.to_string()))?;
            }
            let mut parameters = parameters.clone();
            let mut arguments = arguments.clone();
            if canonical != *own {
                let original = definitions[own];
                let target = definitions[&canonical];
                let binders = original
                    .parameter_binders()
                    .map_err(|e| error(span, e.to_string()))?;
                let inputs = target
                    .parameter_binders()
                    .map_err(|e| error(span, e.to_string()))?
                    .iter()
                    .map(|p| Term::Rigid {
                        declaration: p.declaration,
                        sort: p.sort,
                    })
                    .collect::<Vec<_>>();
                if parameters.is_empty() {
                    parameters = original.type_parameters.clone();
                    arguments = inputs;
                } else {
                    for argument in &mut arguments {
                        let mut fuel = self.required_steps.min(100_000) as usize;
                        let before = fuel;
                        let result = nir::type_computations::charge_term(argument, &mut fuel);
                        self.required_steps -= (before - fuel) as u64;
                        result.map_err(|e| error(span, e.to_string()))?;
                        *argument = argument
                            .instantiate_parameters(&binders, &inputs)
                            .map_err(|e| error(span, e.to_string()))?;
                    }
                }
            }
            self.required_steps = self
                .required_steps
                .checked_sub(1 + parameters.len() as u64 + arguments.len() as u64)
                .ok_or_else(|| error(span, "associated contribution merge budget exhausted"))?;
            let key = (canonical, *member);
            let recipe = (*owner, parameters, arguments);
            if merged.contains_key(&key) {
                alternatives.push((key, recipe));
            } else {
                merged.insert(key, recipe);
            }
        }
        self.associated_initializers = merged;
        self.complementary_associated = alternatives;
        self.implementation_relationships = relationships.clone();
        Ok(())
    }

    pub(super) fn check_complementary_associated(&mut self) -> Result<(), Error> {
        /*
         * Re-evaluate each alternative in the canonical binder environment.
         * Restore the selected recipe and payload even on error. Other shared
         * members retain their selected values; each is independently checked.
         * This private checker world is never published after a failed check.
         */
        for (key, alternative) in std::mem::take(&mut self.complementary_associated) {
            let (implementation, member) = key;
            let span = self.package.item(alternative.0).unwrap().span;
            let slot = self
                .checked_implementations
                .iter()
                .position(|item| item.declaration == implementation)
                .unwrap();
            let expected = self.associated_comparison_payload(slot, member, span)?;
            let saved_value = self.checked_implementations[slot]
                .associated
                .iter()
                .position(|(id, _)| *id == member)
                .map(|position| {
                    self.checked_implementations[slot]
                        .associated
                        .remove(position)
                });
            let saved_initializer = self.checked_implementations[slot]
                .associated_initializers
                .iter()
                .position(|(id, _)| *id == member)
                .map(|position| {
                    self.checked_implementations[slot]
                        .associated_initializers
                        .remove(position)
                });
            let saved_recipe = self
                .associated_initializers
                .insert(key, alternative)
                .unwrap();
            let actual = self
                .selected_associated_value(implementation, member, span)
                .and_then(|_| self.associated_comparison_payload(slot, member, span));
            self.associated_initializers.insert(key, saved_recipe);
            self.checked_implementations[slot]
                .associated
                .retain(|(id, _)| *id != member);
            self.checked_implementations[slot]
                .associated_initializers
                .retain(|(id, _)| *id != member);
            if let Some(value) = saved_value {
                self.checked_implementations[slot].associated.push(value);
            }
            if let Some(initializer) = saved_initializer {
                self.checked_implementations[slot]
                    .associated_initializers
                    .push(initializer);
            }
            let actual = actual?;
            if expected != actual {
                return Err(error(
                    span,
                    "complementary associated values differ or their equality cannot be established",
                ));
            }
        }
        Ok(())
    }

    fn associated_comparison_payload(
        &mut self,
        slot: usize,
        member: DeclarationId,
        span: Span,
    ) -> Result<n::Expr, Error> {
        let implementation = &self.checked_implementations[slot];
        let value = implementation
            .associated
            .iter()
            .find(|(id, _)| *id == member)
            .map(|(_, value)| value)
            .or_else(|| {
                implementation
                    .associated_initializers
                    .iter()
                    .find(|(id, _)| *id == member)
                    .map(|(_, value)| value.expression())
            })
            .ok_or_else(|| error(span, "shared associated value has no checked payload"))?;
        constants::charge_copy(value, &mut self.required_steps, span)?;
        let mut value = value.clone();
        let mut fuel = self.required_steps.min(100_000) as usize;
        let before = fuel;
        let result = nir::visit::expression_mut(&mut value, &mut fuel, |node, _| {
            node.source = n::SourceRange { start: 0, end: 0 };
            Ok(())
        });
        self.required_steps -= (before - fuel) as u64;
        result.map_err(|e| error(span, e.to_string()))?;
        Ok(value)
    }
}
