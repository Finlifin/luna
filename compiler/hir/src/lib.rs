//! # Flurry HIR (High-level Intermediate Representation)
//!
//! ## Architecture (modeled after rustc)
//!
//! ```text
//!   Package<'hir>                                        ← top-level container
//!   ├── owners: IndexVec<LocalDefId, OwnerInfo<'hir>>    ← definition descriptors
//!   └── bodies: FxHashMap<BodyId, Body<'hir>>            ← heavy payloads
//!
//!   HirArena  (owned by CompilerInstance)                ← backing memory
//!     └── TypedArena<Expr>, TypedArena<Pattern>, ...
//! ```
//!
//! All `&'hir T` references point into the [`HirArena`]. The `'hir`
//! lifetime is the borrow lifetime of the arena.

use rustc_data_structures::fx::FxHashMap;

pub mod arena;
pub mod body;
pub mod clause;
pub mod common;
pub mod decl;
pub mod definitions;
mod display;
pub mod expr;
pub mod hir_id;
pub mod idx;
pub mod item;
pub mod node;
pub mod owner;
pub mod pattern;
pub mod trait_sources;

pub use arena::HirArena;
pub use body::{Body, BodyParam};
pub use clause::{ClauseConstraint, ClauseConstraintKind, ClauseParam, ClauseParamKind};
pub use common::{BinOp, BindingMode, Ident, Lit, LitKind, Path, Symbol, UnOp};
pub use decl::LetDecl;
pub use expr::{Block, CondictionArm, Expr, ExprKind, FieldExpr};
pub use hir_id::{BodyId, HirId, ItemLocalId, LocalDefId, OwnerId};
pub use idx::{Idx, IndexVec};
pub use item::{
    Configuration, DefKind, EnumDef, FieldDef, FnSig, ImplDef, ImplementationPhase, Item, ItemKind,
    ModDef, NFSig, StructDef, TraitDef, Variant, VariantKind,
};
pub use node::Node;
pub use owner::{OwnerInfo, OwnerNode, OwnerNodes, ParentedNode};
pub use pattern::{FieldPat, Pattern, PatternArm, PatternKind};

/// Distinguish an enclosing definition from a specific member within it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttributeTarget {
    Definition(OwnerId),
    Member(HirId),
}

/// A definition attribute awaiting elaboration and comptime evaluation.
/// Known names are resolved during lowering; unresolved names remain deferred.
/// The expression is also retained verbatim, without treating its
/// spelling as a compiler intrinsic. Entries for one target are stored in
/// application order (inner prefix first).
#[derive(Debug, Clone)]
pub struct DeferredDefinitionAttribute<'hir> {
    pub target: AttributeTarget,
    pub lexical_owner: OwnerId,
    pub expression: String,
    /// Structured expression in its original lexical context. Keeping source
    /// text as well supports diagnostics; evaluation must consume this tree.
    pub value: &'hir Expr<'hir>,
    /// Some for a configuration of the enclosing definition, None for a prefix Object.
    pub configuration_key: Option<String>,
    pub span: rustc_span::Span,
}

/// The top-level HIR container for a single Flurry package.
///
/// All `&'hir` references inside point into the [`HirArena`] owned by
/// `CompilerInstance`.
#[derive(Clone)]
pub struct Package<'hir> {
    owners: IndexVec<LocalDefId, Option<OwnerInfo<'hir>>>,
    bodies: FxHashMap<BodyId, Body<'hir>>,
    pub root_mod: OwnerId,
    /// Semantic information not yet represented by dedicated HIR nodes.
    pub pending_semantics: Vec<(rustc_span::Span, String)>,
    pub definition_attributes: Vec<DeferredDefinitionAttribute<'hir>>,
    /// Resolver-selected members whose parameterized base must survive lowering.
    pub member_projections: FxHashMap<HirId, symbol::DefId>,
    /// Original source definition for cloned syntax. Fresh lexical binding
    /// identities must not change which package supplies contextual build data.
    pub source_origins: FxHashMap<HirId, symbol::DefId>,
    /// Accessible static members for a base whose container is known only after
    /// elaboration. Each pair is (container, member), resolved at the use site.
    pub deferred_member_projections:
        FxHashMap<HirId, std::sync::Arc<[(symbol::DefId, symbol::DefId)]>>,
    /// Lexical trait sources and unresolved exports at each member use site.
    pub trait_sources: FxHashMap<HirId, std::sync::Arc<trait_sources::TraitSourcePlan>>,
    /// Use-site permissions for methods selected from a receiver's inferred type.
    pub member_access: FxHashMap<HirId, std::sync::Arc<[(symbol::DefId, symbol::DefId)]>>,
    definitions: FxHashMap<symbol::DefId, OwnerId>,
    owner_definitions: FxHashMap<OwnerId, symbol::DefId>,
    // Counts preserve occupancy when replacing owners from unchecked source HIR.
    clause_definitions: FxHashMap<symbol::DefId, usize>,
    next_definition_index: u32,
    /// Lexical definition containing a block-local declaration.
    pub lexical_parents: FxHashMap<OwnerId, OwnerId>,
    pub staged_parameter_definitions: FxHashMap<HirId, symbol::DefId>,
}

impl<'hir> Package<'hir> {
    pub fn new() -> Self {
        Package {
            owners: IndexVec::new(),
            bodies: FxHashMap::default(),
            root_mod: OwnerId::INVALID,
            pending_semantics: Vec::new(),
            definition_attributes: Vec::new(),
            member_projections: FxHashMap::default(),
            source_origins: FxHashMap::default(),
            deferred_member_projections: FxHashMap::default(),
            member_access: FxHashMap::default(),
            trait_sources: FxHashMap::default(),
            definitions: FxHashMap::default(),
            owner_definitions: FxHashMap::default(),
            clause_definitions: FxHashMap::default(),
            next_definition_index: 0,
            lexical_parents: FxHashMap::default(),
            staged_parameter_definitions: FxHashMap::default(),
        }
    }

    /// Reserve the resolver's full scan prefix, including declarations not yet
    /// lowered and clause identities that have no HIR owner. Never moves back.
    pub fn reserve_definition_prefix(&mut self, next: u32) {
        self.next_definition_index = self.next_definition_index.max(next);
    }

    pub fn next_definition_index(&self) -> u32 {
        self.next_definition_index
    }

    pub fn bind_definition(&mut self, definition: symbol::DefId, owner: OwnerId) {
        self.reserve_definition_prefix(definition.index.saturating_add(1));
        self.definitions.insert(definition, owner);
        self.owner_definitions.insert(owner, definition);
    }

    pub fn owner_definition(&self, owner: OwnerId) -> Option<symbol::DefId> {
        self.owner_definitions.get(&owner).copied()
    }

    /// Follow already recorded provenance when cloning a clone; otherwise the
    /// syntax still belongs to its original owner definition.
    pub fn source_origin(&self, expression: HirId) -> Option<symbol::DefId> {
        self.source_origins
            .get(&expression)
            .copied()
            .or_else(|| self.owner_definition(expression.owner))
    }

    pub fn definition_owner(&self, definition: symbol::DefId) -> Option<OwnerId> {
        self.definitions.get(&definition).copied()
    }

    pub fn alloc_owner_id(&mut self) -> OwnerId {
        let id = self.owners.push(None);
        OwnerId::new(id)
    }

    pub fn insert_owner(&mut self, owner_id: OwnerId, info: OwnerInfo<'hir>) {
        let def_id = owner_id.def_id;
        if let Some(previous) = self.item(owner_id) {
            for parameter in definitions::members::clauses(previous) {
                if let Some(identity) = parameter.definition {
                    let count = self
                        .clause_definitions
                        .get_mut(&identity)
                        .expect("indexed clause");
                    *count -= 1;
                    if *count == 0 {
                        self.clause_definitions.remove(&identity);
                    }
                }
            }
        }
        for parameter in definitions::members::clauses(info.node.expect_item()) {
            if let Some(identity) = parameter.definition {
                self.reserve_definition_prefix(identity.index.saturating_add(1));
                *self.clause_definitions.entry(identity).or_default() += 1;
            }
        }
        self.owners.ensure_contains(def_id);
        self.owners[def_id] = Some(info);
    }

    pub fn owner(&self, owner_id: OwnerId) -> Option<&OwnerInfo<'hir>> {
        self.owners.get(owner_id.def_id)?.as_ref()
    }

    pub fn item(&self, owner_id: OwnerId) -> Option<&'hir Item<'hir>> {
        self.owner(owner_id).map(|info| info.node.expect_item())
    }

    pub fn owners(&self) -> impl Iterator<Item = (OwnerId, &OwnerInfo<'hir>)> {
        self.owners
            .iter_enumerated()
            .filter_map(|(id, opt)| opt.as_ref().map(|info| (OwnerId::new(id), info)))
    }

    pub fn num_defs(&self) -> usize {
        self.owners.len()
    }

    pub fn insert_body(&mut self, body_id: BodyId, body: Body<'hir>) {
        self.bodies.insert(body_id, body);
    }

    pub fn body(&self, body_id: BodyId) -> Option<&Body<'hir>> {
        self.bodies.get(&body_id)
    }

    pub fn bodies(&self) -> impl Iterator<Item = (&BodyId, &Body<'hir>)> {
        self.bodies.iter()
    }

    pub fn num_bodies(&self) -> usize {
        self.bodies.len()
    }

    pub fn node(&self, hir_id: HirId) -> Option<&Node<'hir>> {
        let owner_info = self.owner(hir_id.owner)?;
        let parented = owner_info.nodes.get(hir_id.local_id)?;
        Some(&parented.node)
    }

    pub fn hir_id_allocator(&self, owner: OwnerId) -> HirIdAllocator {
        HirIdAllocator {
            owner,
            next_local: 1,
        }
    }
}

impl Default for Package<'_> {
    fn default() -> Self {
        Self::new()
    }
}

impl<'hir> Package<'hir> {
    /// Format the complete HIR as a deterministic Lisp-style S-expression.
    ///
    /// This is the same representation as [`std::fmt::Display`]. Owners and
    /// bodies occupy separate tables, and references retain both ID components.
    pub fn dump_to_lisp(&self) -> String {
        self.to_string()
    }
}

pub struct HirIdAllocator {
    owner: OwnerId,
    next_local: u32,
}

impl HirIdAllocator {
    pub fn owner(&self) -> OwnerId {
        self.owner
    }

    pub fn next_id(&mut self) -> HirId {
        let local = ItemLocalId::new(self.next_local);
        self.next_local += 1;
        HirId::new(self.owner, local)
    }

    pub fn owner_hir_id(&self) -> HirId {
        HirId::make_owner(self.owner)
    }
}
