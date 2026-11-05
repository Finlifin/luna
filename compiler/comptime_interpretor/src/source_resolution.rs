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
        if !package.dispatch_instances.is_empty() {
            return Err(Error {
                kind: ErrorKind::InvalidCallPlan,
                source: None,
            });
        }
        let mut phase_allowance = reserved;
        invocation_budget::charge_world_tables(package, &mut phase_allowance)?;
        nir::phases::validate(package).map_err(|error| Error {
            source: Some(error.source),
            kind: ErrorKind::InvalidPhase(error),
        })?;
        let mut interpreter = Self::with_assumptions_and_index(
            package,
            assumptions,
            reserved,
            None,
            Some(source_state::InvocationScope {
                types: parameters,
                values: value_parameters,
            }),
            SourceDispatch::Resolving(resolvers),
        )?;
        interpreter.comptime_only = true;
        budget.steps = interpreter.remaining_steps();
        Ok(interpreter)
    }
}
