//! NIR program verification: control flow, memory views, ownership and effects.
//! Type propositions remain in PTC; program facts use verification_core.
pub mod contracts;
pub mod control;
pub mod copy;
mod declarations;
pub mod drop;
mod effect_transfer;
pub mod effect_usage;
pub mod flow;
pub mod memory;
pub mod moves;
mod observation_region;
pub mod optimization;
mod pattern_facts;
pub mod purity;
mod rejection;
pub use verification_core::*;

use type_resolution::{associated, capabilities, implementation, ptc};

/// Shared package-scoped verification caches for declaration metadata,
/// dispatch preparation, method selections, memory summaries, and rejection
/// analysis across a single immutable package pass.
pub struct PackageSession<'a> {
    pub(crate) package: &'a nir::program::Package,
    pub(crate) declarations: std::rc::Rc<declarations::Metadata<'a>>,
    pub(crate) source: Option<::materialization::DeclarationSource<'a>>,
    pub(crate) methods: memory::MethodCallees,
    pub(crate) summaries: memory::SummaryCache,
    pub(crate) rejection: rejection::Memo,
}

impl<'a> PackageSession<'a> {
    pub fn new(package: &'a nir::program::Package) -> Self {
        Self {
            package,
            declarations: std::rc::Rc::new(declarations::Metadata::new(Some(package))),
            source: None,
            methods: memory::MethodCallees::default(),
            summaries: memory::SummaryCache::default(),
            rejection: rejection::Memo::default(),
        }
    }

    pub fn package(&self) -> &'a nir::program::Package {
        self.package
    }

    pub fn with_source(mut self, source: ::materialization::DeclarationSource<'a>) -> Self {
        self.source = Some(source);
        self
    }

    pub fn with_dispatch(mut self, dispatch: &[nir::program::DispatchInstance]) -> Self {
        self.methods = memory::MethodCallees::with_dispatch(dispatch);
        self
    }

    pub fn cleanup_session<'w>(
        &self,
        world: &'a capabilities::World<'w>,
        protocol: nir::DeclarationId,
        budget: ptc::Budget,
    ) -> drop::CleanupSession<'a, 'w> {
        drop::CleanupSession::with_context(
            self.declarations.clone(),
            self.source,
            self.methods.with_preselected(),
            world,
            protocol,
            budget,
        )
    }

    pub fn infer_text_regions(
        &mut self,
        declaration: nir::DeclarationId,
        arguments: &[nir::Term],
        fuel: &mut usize,
    ) -> Result<memory::TextRegionSummary, String> {
        memory::infer_text_regions_with_context(
            self.package,
            self.source,
            self.declarations.as_ref(),
            &mut self.methods,
            declaration,
            arguments,
            fuel,
        )
    }

    pub fn infer_memory(
        &mut self,
        declaration: nir::DeclarationId,
        arguments: &[nir::Term],
        text_provenance: memory::TextProvenance,
        fuel: &mut usize,
    ) -> Result<memory::Summary, String> {
        memory::infer_in_package_with_full_context(
            self.package,
            declaration,
            arguments,
            text_provenance,
            self.source,
            Some(&mut self.methods),
            Some(self.declarations.as_ref()),
            &mut self.summaries,
            fuel,
        )
    }
}
