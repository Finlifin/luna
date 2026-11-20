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

const CONST_GLOBAL_OWNER_TAG: u64 = 1u64 << 62;
const CONST_GLOBAL_HASHED_OWNER_TAG: u64 = (1u64 << 62) | (1u64 << 61);

pub(crate) fn const_global_place(declaration: nir::DeclarationId) -> PlaceId {
    let owner = if declaration.package <= u64::from(u32::MAX) {
        CONST_GLOBAL_OWNER_TAG | declaration.package
    } else {
        // Keep bits 63..62 as `01` so const globals never overlap source
        // locals (<= u32::MAX) or synthetic MAX..MAX-6 roots.
        let folded = (declaration.package ^ (declaration.package >> 32)) & 0xFFFF_FFFF;
        CONST_GLOBAL_HASHED_OWNER_TAG | folded
    };
    PlaceId {
        owner,
        local: declaration.local,
    }
}

pub(crate) fn is_const_global_place(id: PlaceId) -> bool {
    (id.owner & (0b11u64 << 62)) == CONST_GLOBAL_OWNER_TAG
}

use type_resolution::{associated, capabilities, implementation, ptc};

/// Shared package-scoped verification caches for declaration metadata,
/// dispatch preparation, method selections, memory summaries, and rejection
/// analysis across a single immutable package pass.
pub struct PackageSession<'a> {
    pub(crate) package: &'a nir::program::Package,
    pub(crate) global_runtime_deliveries:
        Option<&'a std::collections::BTreeMap<nir::RuntimeId, nir::program::Expr>>,
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
            global_runtime_deliveries: None,
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

    pub fn with_global_runtime_deliveries(
        mut self,
        deliveries: &'a std::collections::BTreeMap<nir::RuntimeId, nir::program::Expr>,
    ) -> Self {
        self.global_runtime_deliveries = Some(deliveries);
        self
    }

    pub fn set_global_runtime_deliveries(
        &mut self,
        deliveries: &'a std::collections::BTreeMap<nir::RuntimeId, nir::program::Expr>,
    ) {
        self.global_runtime_deliveries = Some(deliveries);
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
