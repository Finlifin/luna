//! Source expansion adapter for the standalone materialization engine.
use crate::ptc::Budget;
use nir::program as n;
use std::collections::BTreeSet;

pub use ::materialization::{
    dispatch_instances, dispatch_instances_for_invocation, dispatch_instances_for_roots,
    resolve_method_instance,
};
pub(crate) use ::materialization::{
    dispatch_instances_in_context, normalize_inline_annotations, resolve_quote_method_instance,
};

struct SourceExpansion;
impl ::materialization::Expansion for SourceExpansion {
    fn quotes(
        &self,
        package: &n::Package,
        expression: &mut n::Expr,
        next: &mut u32,
        sources: &mut Vec<n::ExpansionSource>,
        enums: &mut Vec<n::Enum>,
        fuel: &mut usize,
    ) -> Result<(), String> {
        crate::program::expand_materialized_quotes(package, expression, next, sources, enums, fuel)
    }
    fn effects(
        &self,
        package: &n::Package,
        inline_effects: &BTreeSet<nir::DeclarationId>,
        expression: &mut n::Expr,
        next: &mut u32,
        sources: &mut Vec<n::ExpansionSource>,
        enums: &mut Vec<n::Enum>,
        fuel: &mut usize,
    ) -> Result<(usize, usize), String> {
        crate::program::inlining::expand_materialized_effects(
            package,
            inline_effects,
            expression,
            next,
            sources,
            enums,
            fuel,
        )
    }
    fn quote(
        &self,
        package: &n::Package,
        template: &n::Function,
        expression: &mut n::Expr,
        next: &mut u32,
        sources: &mut Vec<n::ExpansionSource>,
        enums: &mut Vec<n::Enum>,
        fuel: &mut usize,
    ) -> Result<(), String> {
        crate::program::expand_bound_quote(
            package, template, expression, next, sources, enums, fuel,
        )
    }
    fn inline(
        &self,
        package: &n::Package,
        template: &n::Function,
        expression: &mut n::Expr,
        next: &mut u32,
        sources: &mut Vec<n::ExpansionSource>,
        enums: &mut Vec<n::Enum>,
        fuel: &mut usize,
    ) -> Result<bool, String> {
        crate::program::inlining::expand_bound_inline(
            package, template, expression, next, sources, enums, fuel,
        )
    }
}

pub fn instantiate(
    package: &n::Package,
    roots: &[iir::InstanceKey],
    runtime_bindings: BTreeSet<nir::RuntimeId>,
    budget: Budget,
) -> Result<iir::Package, String> {
    ::materialization::instantiate(package, roots, runtime_bindings, budget, &SourceExpansion)
}

pub fn instantiate_with_types(
    package: &n::Package,
    roots: &[iir::InstanceKey],
    types: &[nir::Term],
    runtime_bindings: BTreeSet<nir::RuntimeId>,
    budget: Budget,
) -> Result<iir::Package, String> {
    ::materialization::instantiate_with_types(
        package,
        roots,
        types,
        runtime_bindings,
        budget,
        &SourceExpansion,
    )
}
