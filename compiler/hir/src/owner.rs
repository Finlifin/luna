//! Owner information – per-definition data stored in the package.

use crate::hir_id::ItemLocalId;
use crate::idx::IndexVec;
use crate::item::Item;
use crate::node::Node;

// ── OwnerNode ────────────────────────────────────────────────────────────────

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

// ── OwnerInfo ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct OwnerInfo<'hir> {
    pub node: OwnerNode<'hir>,
    pub nodes: OwnerNodes<'hir>,
}

// ── OwnerNodes ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct OwnerNodes<'hir> {
    pub nodes: IndexVec<ItemLocalId, Option<ParentedNode<'hir>>>,
}

impl<'hir> OwnerNodes<'hir> {
    pub fn new() -> Self {
        OwnerNodes { nodes: IndexVec::new() }
    }

    pub fn get(&self, local_id: ItemLocalId) -> Option<&ParentedNode<'hir>> {
        self.nodes.get(local_id)?.as_ref()
    }

    pub fn insert(&mut self, local_id: ItemLocalId, parent: ItemLocalId, node: Node<'hir>) {
        self.nodes.insert(local_id, ParentedNode { parent, node });
    }

    pub fn len(&self) -> usize { self.nodes.len() }
    pub fn is_empty(&self) -> bool { self.nodes.is_empty() }
}

impl Default for OwnerNodes<'_> {
    fn default() -> Self { Self::new() }
}

// ── ParentedNode ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct ParentedNode<'hir> {
    pub parent: ItemLocalId,
    pub node: Node<'hir>,
}
