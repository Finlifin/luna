//! HIR identifier types.
//!
//! The HIR uses a two-level identification scheme modeled after rustc:
//!
//! ```text
//!   HirId  =  OwnerId  +  ItemLocalId
//!             ────────     ───────────
//!             "which       "which node
//!              definition"  inside it"
//! ```
//!
//! - [`LocalDefId`] is the raw index into the package's definition table.
//! - [`OwnerId`] wraps a `LocalDefId` to mark a *definition owner*.
//! - [`ItemLocalId`] identifies a node *within* an owner.
//! - [`HirId`] combines the two for a package-wide unique ID.
//! - [`BodyId`] references a separately-stored [`Body`](super::body::Body).

use std::fmt;

use crate::idx::Idx;

// ── LocalDefId ───────────────────────────────────────────────────────────────

/// Index of a definition within a package's definition table.
///
/// Every "owner" – function, struct, enum, module, impl, trait, closure, …
/// – is allocated a unique `LocalDefId`. The ID is a simple `u32` index
/// into a flat vector.
///
/// Analogous to rustc's `LocalDefId`.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LocalDefId(u32);

impl LocalDefId {
    pub const INVALID: Self = LocalDefId(u32::MAX);

    #[inline]
    pub fn new(raw: u32) -> Self {
        LocalDefId(raw)
    }

    #[inline]
    pub fn index(self) -> usize {
        self.0 as usize
    }

    #[inline]
    pub fn raw(self) -> u32 {
        self.0
    }

    #[inline]
    pub fn is_valid(self) -> bool {
        self != Self::INVALID
    }
}

impl fmt::Debug for LocalDefId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "LocalDefId({})", self.0)
    }
}

impl fmt::Display for LocalDefId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl Idx for LocalDefId {
    #[inline]
    fn new(raw: u32) -> Self {
        LocalDefId(raw)
    }
    #[inline]
    fn index(self) -> usize {
        self.0 as usize
    }
}

// ── OwnerId ──────────────────────────────────────────────────────────────────

/// Identifies a *definition owner* within a package.
///
/// An "owner" is a definition that can contain other HIR nodes: functions,
/// structs, enums, modules, impl blocks, closures, etc. Owners form the
/// top-level partitioning of a package's HIR – every [`HirId`] references
/// exactly one owner.
///
/// Analogous to rustc's `OwnerId`.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct OwnerId {
    pub def_id: LocalDefId,
}

impl OwnerId {
    pub const INVALID: Self = OwnerId {
        def_id: LocalDefId::INVALID,
    };

    #[inline]
    pub fn new(def_id: LocalDefId) -> Self {
        OwnerId { def_id }
    }

    #[inline]
    pub fn is_valid(self) -> bool {
        self.def_id.is_valid()
    }
}

impl From<LocalDefId> for OwnerId {
    fn from(def_id: LocalDefId) -> Self {
        OwnerId { def_id }
    }
}

impl fmt::Debug for OwnerId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "OwnerId({})", self.def_id)
    }
}

impl fmt::Display for OwnerId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.def_id)
    }
}

// ── ItemLocalId ──────────────────────────────────────────────────────────────

/// Index of an HIR node *within* its owner.
///
/// Every HIR node that belongs to an owner is assigned a unique
/// `ItemLocalId` within that owner's scope. By convention,
/// `ItemLocalId(0)` refers to the owner node itself.
///
/// Analogous to rustc's `ItemLocalId`.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ItemLocalId(u32);

impl ItemLocalId {
    /// The owner node itself.
    pub const ZERO: Self = ItemLocalId(0);
    pub const INVALID: Self = ItemLocalId(u32::MAX);

    #[inline]
    pub fn new(raw: u32) -> Self {
        ItemLocalId(raw)
    }

    #[inline]
    pub fn index(self) -> usize {
        self.0 as usize
    }

    #[inline]
    pub fn raw(self) -> u32 {
        self.0
    }

    #[inline]
    pub fn is_valid(self) -> bool {
        self != Self::INVALID
    }
}

impl fmt::Debug for ItemLocalId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl fmt::Display for ItemLocalId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl Idx for ItemLocalId {
    #[inline]
    fn new(raw: u32) -> Self {
        ItemLocalId(raw)
    }
    #[inline]
    fn index(self) -> usize {
        self.0 as usize
    }
}

// ── HirId ────────────────────────────────────────────────────────────────────

/// Uniquely identifies any HIR node within a package.
///
/// Composed of:
/// - [`OwnerId`] – which definition (owner) contains this node.
/// - [`ItemLocalId`] – which node within that owner.
///
/// This two-level scheme allows the compiler to load/process owners
/// independently (e.g. for incremental compilation).
///
/// Analogous to rustc's `HirId`.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct HirId {
    pub owner: OwnerId,
    pub local_id: ItemLocalId,
}

impl HirId {
    pub const INVALID: Self = HirId {
        owner: OwnerId::INVALID,
        local_id: ItemLocalId::INVALID,
    };

    #[inline]
    pub fn new(owner: OwnerId, local_id: ItemLocalId) -> Self {
        HirId { owner, local_id }
    }

    /// Create a `HirId` that refers to the owner node itself (`local_id = 0`).
    #[inline]
    pub fn make_owner(owner: OwnerId) -> Self {
        HirId {
            owner,
            local_id: ItemLocalId::ZERO,
        }
    }

    #[inline]
    pub fn is_valid(self) -> bool {
        self.owner.is_valid()
    }
}

impl fmt::Debug for HirId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "({:?}:{:?})", self.owner, self.local_id)
    }
}

impl fmt::Display for HirId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.owner, self.local_id)
    }
}

// ── BodyId ───────────────────────────────────────────────────────────────────

/// Identifies a [`Body`](super::body::Body) stored in the package's body table.
///
/// Bodies (function/closure bodies) are stored *separately* from definition
/// descriptors so that the compiler can work with lightweight definition
/// metadata without loading full body ASTs.
///
/// The `hir_id` field points to the HIR node that *owns* the body – for
/// a function, this is the function item itself; for a closure, the
/// closure expression.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct BodyId {
    pub hir_id: HirId,
}

impl BodyId {
    #[inline]
    pub fn new(hir_id: HirId) -> Self {
        BodyId { hir_id }
    }
}

impl fmt::Debug for BodyId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "BodyId({:?})", self.hir_id)
    }
}
