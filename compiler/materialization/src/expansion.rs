//! Explicit source expansion on private IIR candidates.
//! Implementations share the caller's fuel, fresh identities and provenance;
//! failures must preserve the expansion transaction's existing rollback rules.
use nir::program as n;
use std::collections::BTreeSet;

pub trait Expansion {
    fn quotes(
        &self,
        package: &n::Package,
        expression: &mut n::Expr,
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
        next: &mut u32,
        sources: &mut Vec<n::ExpansionSource>,
        enums: &mut Vec<n::Enum>,
        fuel: &mut usize,
    ) -> Result<bool, String>;
}
