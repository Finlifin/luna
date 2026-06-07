//! Owner information – per-definition data stored in the package.
//!
//! Every owner (function, struct, enum, …) is represented by an
//! [`OwnerInfo`] which pairs the top-level [`OwnerNode`] with the flat
//! [`OwnerNodes`] table that indexes every sub-node inside that owner.

use crate::hir_id::ItemLocalId;
use crate::idx::IndexVec;
use crate::item::Item;
use crate::node::Node;

/// The top-level HIR node of a definition owner.
///
/// Currently only function / struct / … [`Item`]s are owners, but the enum
/// is kept open for future extension (e.g. free closures).
#[derive(Debug, Clone)]
pub enum OwnerNode<'hir> {
    Item(&'hir Item<'hir>),
}

impl<'hir> OwnerNode<'hir> {
    pub fn as_item(&self) -> Option<&'hir Item<'hir>> {
        match self {
            OwnerNode::Item(item) => Some(item),
        }
    }

    pub fn expect_item(&self) -> &'hir Item<'hir> {
        self.as_item().expect("expected OwnerNode::Item")
    }
}

/// All data stored for one definition owner.
#[derive(Debug, Clone)]
pub struct OwnerInfo<'hir> {
    pub node: OwnerNode<'hir>,
    pub nodes: OwnerNodes<'hir>,
}

/// Flat table of every HIR node inside an owner, indexed by [`ItemLocalId`].
///
/// Each entry records the node and its parent, enabling parent-traversal
/// without requiring parent pointers on every node type.
#[derive(Debug, Clone)]
pub struct OwnerNodes<'hir> {
    pub nodes: IndexVec<ItemLocalId, Option<ParentedNode<'hir>>>,
}

impl<'hir> OwnerNodes<'hir> {
    pub fn new() -> Self {
        OwnerNodes {
            nodes: IndexVec::new(),
        }
    }

    pub fn get(&self, local_id: ItemLocalId) -> Option<&ParentedNode<'hir>> {
        self.nodes.get(local_id)?.as_ref()
    }

    pub fn insert(&mut self, local_id: ItemLocalId, parent: ItemLocalId, node: Node<'hir>) {
        self.nodes.insert(local_id, ParentedNode { parent, node });
    }

    pub fn len(&self) -> usize {
        self.nodes.len()
    }
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }
}

impl Default for OwnerNodes<'_> {
    fn default() -> Self {
        Self::new()
    }
}

/// A HIR node together with its parent's [`ItemLocalId`].
#[derive(Debug, Clone)]
pub struct ParentedNode<'hir> {
    pub parent: ItemLocalId,
    pub node: Node<'hir>,
}
