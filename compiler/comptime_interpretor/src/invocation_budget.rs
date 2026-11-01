//! Scope a checked comptime world and synchronize its execution allowance.
use crate::{Budget, Error, ErrorKind, Interpreter};
use nir::program::{Package, TraitRequirement};

/// Reserve top-level world traversal and indexing before allocating VM tables.
/// Nested type, contract and substitution work keeps its separate existing cost.
pub(super) fn charge_world_tables(package: &Package, budget: &mut Budget) -> Result<(), Error> {
    let Package {
        builtin_implementation_packages,
        target: _,
        tests,
        function_parameters,
        runtime_instances,
        quote_templates,
        quote_identity_cursor: _,
        definition_identity_cursor: _,
        internal_return_packets,
        internal_return_empty_replacements,
        expansion_sources,
        effects,
        runtime_function_identities,
        normal_forms,
        dispatch_instances,
        traits,
        implementations,
        functions,
        structs,
        newtypes,
        alias_contracts,
        enums,
    } = package;
    let cost = [
        builtin_implementation_packages.len(),
        tests.len(),
        function_parameters.len(),
        runtime_instances.len(),
        quote_templates.len(),
        internal_return_packets.len(),
        internal_return_empty_replacements.len(),
        expansion_sources.len(),
        effects.len(),
        runtime_function_identities.len(),
        normal_forms.len(),
        dispatch_instances.len(),
        traits.len(),
        implementations.len(),
        functions.len(),
        structs.len(),
        newtypes.len(),
        alias_contracts.len(),
        enums.len(),
    ]
    .into_iter()
    .try_fold(1u64, |sum, count| {
        sum.checked_add(u64::try_from(count).ok()?)
    });
    budget.steps = cost
        .and_then(|cost| budget.steps.checked_sub(cost))
        .ok_or(Error {
            kind: ErrorKind::StepsExhausted,
            source: None,
        })?;
    Ok(())
}

struct Scope<'program, 'budget> {
    interpreter: Interpreter<'program>,
    budget: &'budget mut Budget,
}
impl Drop for Scope<'_, '_> {
    fn drop(&mut self) {
        // Keep the construction reservation even if host code replaced the VM
        // with another one. A scope must never replenish its caller's allowance.
        self.budget.steps = self.budget.steps.min(self.interpreter.remaining_steps());
    }
}

impl<'program> Interpreter<'program> {
    /// Prepare a fresh checked world, execute a host operation, and synchronize
    /// remaining steps on success, error, or unwinding. Construction failure
    /// retains the conservative reservation of for_comptime_in_budget.
    ///
    /// Consecutive scopes can use newly published packages without carrying
    /// function instances or evidence caches from the old world. Inputs and
    /// returned values retain ordinary invocation admission. This does not resume
    /// an active call stack or transfer its local storage/cleanup responsibilities.
    pub fn with_comptime_in_budget<T>(
        package: &'program Package,
        assumptions: &[TraitRequirement],
        budget: &mut Budget,
        operation: impl FnOnce(&mut Self) -> Result<T, Error>,
    ) -> Result<T, Error> {
        let interpreter = Self::for_comptime_in_budget(package, assumptions, budget)?;
        let mut scope = Scope {
            interpreter,
            budget,
        };
        operation(&mut scope.interpreter)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nir::{DeclarationId, Primitive, Term};

    #[test]
    fn declaration_domain_budget_failure_is_not_an_invalid_call_plan() {
        let parameter = nir::function_parameters::Declaration {
            declaration: DeclarationId {
                package: 1,
                local: 1,
            },
            parameters: vec![DeclarationId {
                package: 1,
                local: 2,
            }],
            domains: vec![Term::Optional(Box::new(Term::Primitive(Primitive::Bool)))],
        };
        let mut package = Package {
            function_parameters: vec![parameter.clone()],
            ..Default::default()
        };
        let mut succeeded = false;
        for steps in 0..256 {
            match Interpreter::for_comptime(&package, Budget { steps, depth: 128 }) {
                Ok(_) => {
                    succeeded = true;
                    break;
                }
                Err(error) => assert_eq!(error.kind, ErrorKind::StepsExhausted, "budget {steps}"),
            }
        }
        assert!(
            succeeded,
            "valid metadata must succeed with sufficient fuel"
        );
        package.function_parameters.push(parameter);
        assert_eq!(
            Interpreter::new(&package, Budget::default())
                .err()
                .unwrap()
                .kind,
            ErrorKind::InvalidCallPlan
        );
    }
}
