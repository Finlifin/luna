//! Atomic installation of completed struct descriptions after checked publication.
use super::*;

struct Installation<'vm, 'world> {
    vm: &'vm mut Interpreter<'world>,
    definitions: &'world [nir::program::Struct],
    inserted: usize,
    committed: bool,
}
impl Drop for Installation<'_, '_> {
    fn drop(&mut self) {
        if !self.committed {
            for definition in &self.definitions[..self.inserted] {
                self.vm.structs.remove(&definition.declaration);
            }
        }
    }
}

impl<'world> Interpreter<'world> {
    /// Install a checked batch of concrete builder results into this comptime VM.
    /// The publisher reserves global identities and performs ordinary declaration
    /// checking first. This boundary additionally checks collisions, closed field
    /// descriptions and formation in the combined world before retaining it.
    ///
    /// Existing declarations, values, invocation state and caches are untouched.
    /// Failure/unwind removes the entire batch but does not refund work or roll
    /// back arbitrary host solver effects. The caller owns the immutable structs
    /// for the VM's lifetime. Generic declarations, functions and impls require a
    /// separate publication path; this is not a source builder dispatch hook.
    pub fn install_generated_structs(
        &mut self,
        definitions: &'world [nir::program::Struct],
    ) -> Result<(), Error> {
        if !self.comptime_only || self.graph.is_some() || self.cleanup_schedule.is_some() {
            return Err(self.failure(ErrorKind::PhaseMismatch, None));
        }
        if !self.abstract_type_parameters.is_empty() || !self.abstract_value_parameters.is_empty() {
            return Err(self.failure(ErrorKind::Type, None));
        }
        self.integer_operation(None, |budget| {
            budget.consume(definitions.len().saturating_add(1))
        })?;
        let mut identities = BTreeSet::new();
        for definition in definitions {
            if !definition.type_parameters.is_empty()
                || !definition.parameter_types.is_empty()
                || !identities.insert(definition.declaration)
                || self.known_definition(definition.declaration)?
            {
                return Err(self.failure(ErrorKind::InvalidCallPlan, None));
            }
            self.integer_operation(None, |budget| {
                budget.consume(
                    definition
                        .name
                        .len()
                        .saturating_add(definition.fields.len()),
                )
            })?;
            for requirement in &definition.requirements {
                let before = usize::try_from(self.budget.steps).unwrap_or(usize::MAX);
                let mut remaining = before;
                let checked = nir::copy_budget::charge_predicate_substitution(
                    requirement,
                    &[],
                    &[],
                    &mut remaining,
                );
                self.budget.steps -= (before - remaining) as u64;
                checked.map_err(|_| self.failure(ErrorKind::StepsExhausted, None))?;
            }
            for capability in &definition.capabilities {
                let before = usize::try_from(self.budget.steps).unwrap_or(usize::MAX);
                let mut remaining = before;
                let checked = (|| {
                    nir::type_computations::charge_term(&capability.subject, &mut remaining)?;
                    for argument in &capability.interface_arguments {
                        nir::type_computations::charge_term(argument, &mut remaining)?;
                    }
                    Ok::<_, nir::type_computations::Error>(())
                })();
                self.budget.steps -= (before - remaining) as u64;
                checked.map_err(|_| self.failure(ErrorKind::StepsExhausted, None))?;
            }
            let mut names = BTreeSet::new();
            for (name, ty) in &definition.fields {
                self.integer_operation(None, |budget| {
                    budget.consume(name.len().saturating_add(1))
                })?;
                if !names.insert(name) || ty.sort() != nir::Sort::Type {
                    return Err(self.failure(ErrorKind::Type, None));
                }
                let before = usize::try_from(self.budget.steps).unwrap_or(usize::MAX);
                let mut remaining = before;
                let checked = nir::type_computations::charge_term(ty, &mut remaining);
                self.budget.steps -= (before - remaining) as u64;
                checked.map_err(|_| self.failure(ErrorKind::StepsExhausted, None))?;
                ty.check_scope(&[])
                    .map_err(|_| self.failure(ErrorKind::Type, None))?;
            }
        }
        // Every identity was checked before mutation. Reserve insertion and
        // rollback work up front; unwinding must remove only this batch even if
        // admission consumes the rest of the invocation's execution allowance.
        self.integer_operation(None, |budget| {
            budget.consume(definitions.len().saturating_mul(2))
        })?;
        let mut installation = Installation {
            vm: self,
            definitions,
            inserted: 0,
            committed: false,
        };
        for definition in definitions {
            installation.inserted += 1;
            installation
                .vm
                .structs
                .insert(definition.declaration, definition);
        }
        for definition in definitions {
            let ty = Term::Nominal {
                declaration: definition.declaration,
                arguments: vec![],
            };
            installation.vm.admit_type(&ty, 0)?;
            for (_, field) in &definition.fields {
                installation.vm.admit_type(field, 0)?;
            }
        }
        installation.committed = true;
        Ok(())
    }

    fn known_definition(&mut self, id: DeclarationId) -> Result<bool, Error> {
        self.tick(None)?;
        if self.structs.contains_key(&id)
            || self.enums.contains_key(&id)
            || self.newtypes.contains_key(&id)
            || self.normal_forms.contains_key(&id)
            || self.functions.contains_key(&id)
            || self.runtime_function_identities.contains_key(&id)
            || self.alias_contracts.contains_key(&id)
            || self.effect_declarations.contains_key(&id)
            || self.declaration_parameter_sorts.contains_key(&id)
        {
            return Ok(true);
        }
        // Trait/impl and body-free telescope identities are not all represented
        // by executable function tables. They still occupy declaration space.
        let world = self
            .declaration_world
            .ok_or_else(|| self.failure(ErrorKind::PhaseMismatch, None))?;
        for interface in &world.traits {
            self.integer_operation(None, |budget| {
                budget.consume(
                    interface
                        .methods
                        .len()
                        .saturating_add(interface.associated.len())
                        .saturating_add(1),
                )
            })?;
            if interface.declaration == id
                || interface.self_parameter == id
                || interface
                    .methods
                    .iter()
                    .any(|method| method.declaration == id)
                || interface
                    .associated
                    .iter()
                    .any(|member| member.declaration == id)
            {
                return Ok(true);
            }
        }
        for implementation in &world.implementations {
            self.tick(None)?;
            if implementation.declaration == id {
                return Ok(true);
            }
        }
        for parameters in &world.function_parameters {
            self.tick(None)?;
            if parameters.declaration == id {
                return Ok(true);
            }
        }
        for template in &world.quote_templates {
            self.tick(None)?;
            if template.declaration == id {
                return Ok(true);
            }
        }
        Ok(false)
    }
}

#[cfg(test)]
mod tests;
