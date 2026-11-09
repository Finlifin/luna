//! Source-world selection is demanded by execution, separate from closed tables.
use super::*;

pub type BudgetedTypeSolver<'a> =
    dyn Fn(&nir::program::TypePredicate, &mut u64) -> ContractOutcome + 'a;

/// Services for one immutable source world. Supplying all three lets execution
/// demand evidence and code without preselecting unrelated implementations.
pub struct SourceResolvers<'a> {
    pub types: &'a BudgetedTypeSolver<'a>,
    pub methods: &'a MethodResolver<'a>,
    pub associated: &'a AssociatedResolver<'a>,
}

pub(super) enum SourceDispatch<'a> {
    Closed,
    Resolving(SourceResolvers<'a>),
    // Private to non-executing descriptor admission. This grants no method
    // identity or proof authority merely because an implementation is declared.
    Admission,
}

impl<'a> Interpreter<'a> {
    /// Execute checked source with demand-driven selection in its exact world.
    /// No preselected dispatch table is accepted: each new member demand goes
    /// through the supplied services. Declaration shape and phase validation
    /// remain mandatory; source bodies are neither traversed for dependencies
    /// nor executed during construction. Failed construction reserves its fuel.
    pub fn for_comptime_resolving_in_budget(
        package: &'a Package,
        assumptions: &[nir::program::TraitRequirement],
        parameters: &[DeclarationId],
        value_parameters: &[(DeclarationId, Term)],
        resolvers: SourceResolvers<'a>,
        budget: &mut Budget,
    ) -> Result<Self, Error> {
        let reserved = *budget;
        budget.steps = 0;
        let mut interpreter = Self::prepare_resolving_source(package, resolvers, reserved)?;
        interpreter.admit_resolving_scope(assumptions, parameters, value_parameters)?;
        budget.steps = interpreter.remaining_steps();
        Ok(interpreter)
    }

    /// Source declarations carry no caller authority. This mode rejects closed
    /// dispatch tables, so their admission cannot depend on invocation premises.
    pub(super) fn prepare_resolving_source(
        package: &'a Package,
        resolvers: SourceResolvers<'a>,
        mut reserved: Budget,
    ) -> Result<Self, Error> {
        if !package.dispatch_instances.is_empty() {
            return Err(Error {
                kind: ErrorKind::InvalidCallPlan,
                source: None,
            });
        }
        let inspection = invocation_budget::prepare_phase_inspection(package, &mut reserved)?;
        let mut interpreter = Self::with_assumptions_and_index(
            package,
            &[],
            reserved,
            None,
            None,
            SourceDispatch::Resolving(resolvers),
        )?;
        interpreter.phase_inspection = Some(inspection);
        interpreter.comptime_only = true;
        Ok(interpreter)
    }

    /// Rebuild authority for each fresh VM, even when declaration validation
    /// and code are shared. A failed constructor never publishes this VM.
    pub(super) fn admit_resolving_scope(
        &mut self,
        assumptions: &[nir::program::TraitRequirement],
        parameters: &[DeclarationId],
        value_parameters: &[(DeclarationId, Term)],
    ) -> Result<(), Error> {
        let before = usize::try_from(self.budget.steps).unwrap_or(usize::MAX);
        let mut remaining = before;
        let premises =
            constructor_premises::expand(assumptions, &self.trait_declarations, &mut remaining);
        self.budget.steps -= (before - remaining) as u64;
        self.assumed_interfaces = premises?;
        self.authorize_abstract_type_parameters(parameters)?;
        self.authorize_abstract_value_parameters(value_parameters)
    }
}
