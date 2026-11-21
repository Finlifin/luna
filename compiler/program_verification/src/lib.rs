//! NIR program verification: control flow, memory views, ownership and effects.
//! Type propositions remain in PTC; program facts use verification_core.
pub mod behavior;
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

/// Reusable package-scoped verification caches retained across fixpoint rounds
/// whose type declarations remain unchanged.
#[derive(Default)]
pub struct SessionCaches {
    summaries: memory::SummaryCache,
    layouts: memory::TypeLayoutCache,
    move_layouts: moves::MoveLayoutCache,
    ownership_shapes: moves::OwnershipShapeCache,
    text_regions: memory::TextRegionCache,
    return_results: moves::ReturnResultCache,
    rejection: rejection::Memo,
}

impl SessionCaches {
    /// Retain entries unaffected by updates to `changed` runtime instances.
    /// Layout, text-region, rejection, and ownership-shape caches depend only
    /// on immutable declarations and bodies unless new instances arrive.
    pub fn retain_unaffected(&mut self, changed: &std::collections::HashSet<iir::InstanceKey>) {
        self.summaries.retain_unaffected(changed);
        if !changed.is_empty() {
            self.return_results.clear();
        }
    }

    /// Reset caches that inspect materialized `runtime_instances` after the
    /// graph grows monotonically without adding new nominal types.
    pub fn on_graph_extended(&mut self) {
        self.rejection.clear();
        self.return_results.clear();
    }
}

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
    pub(crate) return_results: moves::ReturnResultCache,
    pub(crate) layout_cache: memory::TypeLayoutCache,
    pub(crate) move_layouts: moves::MoveLayoutCache,
    pub(crate) ownership_shapes: moves::OwnershipShapeCache,
    pub(crate) text_regions: memory::TextRegionCache,
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
            return_results: moves::ReturnResultCache::default(),
            layout_cache: memory::TypeLayoutCache::default(),
            move_layouts: moves::MoveLayoutCache::default(),
            ownership_shapes: moves::OwnershipShapeCache::default(),
            text_regions: memory::TextRegionCache::default(),
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

    pub fn with_summaries(mut self, summaries: memory::SummaryCache) -> Self {
        self.summaries = summaries;
        self
    }

    pub fn into_summaries(self) -> memory::SummaryCache {
        self.summaries
    }

    pub fn with_caches(mut self, caches: SessionCaches) -> Self {
        self.summaries = caches.summaries;
        self.layout_cache = caches.layouts;
        self.move_layouts = caches.move_layouts;
        self.ownership_shapes = caches.ownership_shapes;
        self.text_regions = caches.text_regions;
        self.return_results = caches.return_results;
        self.rejection = caches.rejection;
        self
    }

    pub fn into_caches(self) -> SessionCaches {
        SessionCaches {
            summaries: self.summaries,
            layouts: self.layout_cache,
            move_layouts: self.move_layouts,
            ownership_shapes: self.ownership_shapes,
            text_regions: self.text_regions,
            return_results: self.return_results,
            rejection: self.rejection,
        }
    }

    pub fn pointer_input_layout(
        &mut self,
        ty: &nir::Term,
        provenance: memory::TextProvenance,
        fuel: &mut usize,
    ) -> Result<Vec<memory::PointerInputLeaf>, String> {
        self.layout_cache
            .pointer_input_layout_for(Some(self.package), ty, provenance, fuel)
    }

    pub fn pointer_representation(
        &mut self,
        ty: &nir::Term,
        fuel: &mut usize,
    ) -> Result<Option<(usize, nir::Term)>, String> {
        self.layout_cache
            .pointer_representation(Some(self.package), ty, fuel)
    }

    pub fn callable_parameter_paths(
        &mut self,
        signature: &nir::Term,
        fuel: &mut usize,
        source: nir::program::SourceRange,
    ) -> Result<Vec<(usize, Vec<usize>)>, moves::Error> {
        self.move_layouts
            .callable_parameter_paths(self.package, signature, fuel, source)
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
            Some(&mut self.layout_cache),
            Some(&mut self.text_regions),
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
            Some(&mut self.layout_cache),
            fuel,
        )
    }

    pub fn validate_residual_contracts(
        &self,
        function: &nir::program::Function,
        limits: &mut Limits,
    ) -> Result<(), flow::Error> {
        flow::validate_residual_contracts_with_declarations(
            self.package,
            self.declarations.clone(),
            function,
            limits,
        )
    }
}
