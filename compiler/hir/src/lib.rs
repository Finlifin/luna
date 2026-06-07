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
pub mod expr;
pub mod hir_id;
pub mod idx;
pub mod item;
pub mod node;
pub mod owner;
pub mod pattern;

pub use arena::HirArena;
pub use body::{Body, Param};
pub use clause::{ClauseConstraint, ClauseConstraintKind, ClauseParam, ClauseParamKind};
pub use common::{BinOp, BindingMode, Ident, Lit, LitKind, Path, Symbol, UnOp};
pub use decl::LetDecl;
pub use expr::{Block, CondictionArm, Expr, ExprKind, FieldExpr};
pub use hir_id::{BodyId, HirId, ItemLocalId, LocalDefId, OwnerId};
pub use idx::{Idx, IndexVec};
pub use item::{
    DefKind, EnumDef, FieldDef, FnSig, ImplDef, Item, ItemKind, ModDef, NFSig, StructDef, TraitDef,
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
}

impl<'hir> Package<'hir> {
    pub fn new() -> Self {
        Package {
            owners: IndexVec::new(),
            bodies: FxHashMap::default(),
            root_mod: OwnerId::INVALID,
        }
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
    /// Serialize the HIR package as a Lisp-style s-expression string.
    ///
    /// Outputs one line per top-level item showing its kind and name.
    /// Function items also list their parameters (name: type) and
    /// their return type if present.  Bodies are printed on subsequent
    /// indented lines.
    ///
    /// # Format
    /// ```text
    /// (hir-package
    ///   (fn add (params (a _) (b _)) (ret _) (body <id>))
    ///   (struct Point (fields x y))
    ///   (mod some_module))
    /// ```
    pub fn dump_to_lisp(&self) -> String {
        use item::ItemKind;
        use std::fmt::Write as _;

        let mut out = String::new();
        writeln!(out, "(hir-package").unwrap();

        for (owner_id, _info) in self.owners() {
            let Some(item) = self.item(owner_id) else {
                continue;
            };
            match &item.kind {
                ItemKind::Fn(sig, body_id) => {
                    write!(out, "  (fn {}", item.ident.name).unwrap();
                    // Parameters
                    write!(out, " (params").unwrap();
                    for (ident, _tp) in sig.params {
                        write!(out, " {}", ident.name).unwrap();
                    }
                    write!(out, ")").unwrap();
                    // Return type placeholder
                    if sig.return_ty.is_some() {
                        write!(out, " (ret ...)").unwrap();
                    }
                    write!(out, " (body {})", body_id.hir_id.local_id.raw()).unwrap();

                    // Params from body, if available
                    if let Some(body) = self.body(*body_id) {
                        if !body.params.is_empty() {
                            write!(out, "\n    (body-params").unwrap();
                            for p in body.params {
                                write!(out, " {}", p.name.name).unwrap();
                            }
                            write!(out, ")").unwrap();
                        }
                    }
                    writeln!(out, ")").unwrap();
                }
                ItemKind::Struct(def) => {
                    write!(out, "  (struct {}", item.ident.name).unwrap();
                    write!(out, " (fields").unwrap();
                    for f in def.fields {
                        write!(out, " {}", f.ident.name).unwrap();
                    }
                    writeln!(out, "))").unwrap();
                }
                ItemKind::Enum(def) => {
                    write!(out, "  (enum {}", item.ident.name).unwrap();
                    write!(out, " (variants").unwrap();
                    for v in def.variants {
                        write!(out, " {}", v.ident.name).unwrap();
                    }
                    writeln!(out, "))").unwrap();
                }
                ItemKind::Mod(_) => {
                    writeln!(out, "  (mod {})", item.ident.name).unwrap();
                }
                ItemKind::Trait(_) => {
                    writeln!(out, "  (trait {})", item.ident.name).unwrap();
                }
                ItemKind::Impl(_) => {
                    writeln!(out, "  (impl {})", item.ident.name).unwrap();
                }
                ItemKind::TypeAlias(_) => {
                    writeln!(out, "  (type-alias {})", item.ident.name).unwrap();
                }
                ItemKind::Use(_) => {
                    writeln!(out, "  (use {})", item.ident.name).unwrap();
                }
                ItemKind::Const(_, _) => {
                    writeln!(out, "  (const {})", item.ident.name).unwrap();
                }
                ItemKind::Invalid => {
                    writeln!(out, "  (invalid {}))", item.ident.name).unwrap();
                }
            }
        }

        out.push(')');
        out
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
