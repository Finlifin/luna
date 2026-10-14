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
mod display;
pub mod expr;
pub mod hir_id;
pub mod idx;
pub mod item;
pub mod node;
pub mod owner;
pub mod pattern;

pub use arena::HirArena;
pub use body::{Body, BodyParam};
pub use clause::{ClauseConstraint, ClauseConstraintKind, ClauseParam, ClauseParamKind};
pub use common::{BinOp, BindingMode, Ident, Lit, LitKind, Path, Symbol, UnOp};
pub use decl::LetDecl;
pub use expr::{Block, CondictionArm, Expr, ExprKind, FieldExpr};
pub use hir_id::{BodyId, HirId, ItemLocalId, LocalDefId, OwnerId};
pub use idx::{Idx, IndexVec};
pub use item::{
    Configuration, DefKind, EnumDef, FieldDef, FnSig, ImplDef, Item, ItemKind, ModDef, NFSig, StructDef, TraitDef,
    Variant, VariantKind,
};
pub use node::Node;
pub use owner::{OwnerInfo, OwnerNode, OwnerNodes, ParentedNode};
pub use pattern::{FieldPat, Pattern, PatternArm, PatternKind};

/// The top-level HIR container for a single Flurry package.
///
/// All `&'hir` references inside point into the [`HirArena`] owned by
/// `CompilerInstance`.
pub struct Package<'hir> {
    owners: IndexVec<LocalDefId, Option<OwnerInfo<'hir>>>,
    bodies: FxHashMap<BodyId, Body<'hir>>,
    pub root_mod: OwnerId,
    /// Semantic information not yet represented by dedicated HIR nodes.
    pub pending_semantics: Vec<(rustc_span::Span, String)>,
    definitions: FxHashMap<symbol::DefId, OwnerId>,
    owner_definitions: FxHashMap<OwnerId, symbol::DefId>,
}

impl<'hir> Package<'hir> {
    pub fn new() -> Self {
        Package {
            owners: IndexVec::new(),
            bodies: FxHashMap::default(),
            root_mod: OwnerId::INVALID,
            pending_semantics: Vec::new(),
            definitions: FxHashMap::default(),
            owner_definitions: FxHashMap::default(),
        }
    }

    pub fn bind_definition(&mut self, definition: symbol::DefId, owner: OwnerId) {
        self.definitions.insert(definition, owner);
        self.owner_definitions.insert(owner, definition);
    }

    pub fn owner_definition(&self, owner: OwnerId) -> Option<symbol::DefId> {
        self.owner_definitions.get(&owner).copied()
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
