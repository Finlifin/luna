//! Explicit source expansion on private IIR candidates.
//! Implementations share the caller's fuel, fresh identities and provenance;
//! failures must preserve the expansion transaction's existing rollback rules.
use nir::program as n;
use std::collections::BTreeSet;

/// A publisher owns private checked receipts for one source invocation.
pub trait DefinitionPublisher: comptime_interpretor::DefinitionBuildService {
    fn finish(self: Box<Self>) -> (n::PublishedDefinitions, u32);
}

pub trait Expansion {
    /// The host owns the implementation of formation and representation checks.
    /// Providers with no declaration service can still expand ordinary code.
    fn definition_publisher<'a>(
        &self,
        _baseline: &'a n::Package,
        _owner: u64,
        _solver: &'a comptime_interpretor::BudgetedTypeSolver<'a>,
    ) -> Option<Box<dyn DefinitionPublisher + 'a>> {
        None
    }

    fn quotes(
        &self,
        package: &n::Package,
        expression: &mut n::Expr,
        caller_env_parameters: &[n::RuntimeEnvParameter],
        caller_runtime_deliveries: &std::collections::BTreeMap<nir::RuntimeId, n::Expr>,
        next: &mut u32,
        sources: &mut Vec<n::ExpansionSource>,
        enums: &mut Vec<n::Enum>,
        fuel: &mut usize,
    ) -> Result<(), String>;
    fn effects(
        &self,
        package: &n::Package,
        inline_effects: &BTreeSet<nir::DeclarationId>,
        expression: &mut n::Expr,
        next: &mut u32,
        sources: &mut Vec<n::ExpansionSource>,
        enums: &mut Vec<n::Enum>,
        fuel: &mut usize,
    ) -> Result<(usize, usize), String>;
    fn quote(
        &self,
        package: &n::Package,
        template: &n::Function,
        expression: &mut n::Expr,
        caller_env_parameters: &[n::RuntimeEnvParameter],
        caller_runtime_deliveries: &std::collections::BTreeMap<nir::RuntimeId, n::Expr>,
        in_closure: bool,
        next: &mut u32,
        sources: &mut Vec<n::ExpansionSource>,
        enums: &mut Vec<n::Enum>,
        fuel: &mut usize,
    ) -> Result<(), String>;
    fn inline(
        &self,
        package: &n::Package,
        template: &n::Function,
        expression: &mut n::Expr,
        caller_env_parameters: &[n::RuntimeEnvParameter],
        caller_runtime_deliveries: &std::collections::BTreeMap<nir::RuntimeId, n::Expr>,
        in_closure: bool,
        next: &mut u32,
        sources: &mut Vec<n::ExpansionSource>,
        enums: &mut Vec<n::Enum>,
        fuel: &mut usize,
    ) -> Result<bool, String>;
}
