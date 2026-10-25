//! Owner-local node construction shared by generated definition kinds.
use super::{Error, charge, error};
use crate::{HirId, Item, ItemLocalId, Node, OwnerId, OwnerInfo, OwnerNode, OwnerNodes};
use std::fmt;

/// Allocate identities before constructing arena nodes, then register each node
/// exactly once. Parents must already be reserved, which prevents parent cycles.
/// Finishing transfers the index into OwnerInfo; semantic publication remains a
/// separate DefinitionBatch operation. No package table is mutated here.
pub struct DefinitionNodes<'hir> {
    owner: OwnerId,
    parents: Vec<ItemLocalId>,
    nodes: OwnerNodes<'hir>,
    pending: usize,
    finished: bool,
}
impl<'hir> DefinitionNodes<'hir> {
    pub fn new(owner: OwnerId) -> Result<Self, Error> {
        if owner == OwnerId::INVALID {
            return Err(error("invalid node draft owner", owner));
        }
        Ok(Self {
            owner,
            parents: vec![ItemLocalId::new(0)],
            nodes: OwnerNodes::new(),
            pending: 0,
            finished: false,
        })
    }
    pub fn root(&self) -> HirId {
        HirId::new(self.owner, ItemLocalId::new(0))
    }
    fn open(&self) -> Result<(), Error> {
        if self.finished {
            Err(error("node draft already finished", self.owner))
        } else {
            Ok(())
        }
    }
    pub fn reserve(&mut self, parent: HirId, fuel: &mut usize) -> Result<HirId, Error> {
        self.open()?;
        if parent.owner != self.owner || parent.local_id.raw() as usize >= self.parents.len() {
            return Err(error("node parent not reserved in this draft", self.owner));
        }
        let next = u32::try_from(self.parents.len())
            .ok()
            .filter(|id| *id != u32::MAX)
            .ok_or_else(|| error("local node identity exhausted", self.owner))?;
        charge(fuel)?;
        self.parents.push(parent.local_id);
        self.pending += 1;
        Ok(HirId::new(self.owner, ItemLocalId::new(next)))
    }
    pub fn register(&mut self, node: Node<'hir>, fuel: &mut usize) -> Result<(), Error> {
        self.open()?;
        let id = node.hir_id();
        if id.owner != self.owner
            || id.local_id.raw() == 0
            || id.local_id.raw() as usize >= self.parents.len()
        {
            return Err(error(
                "node identity not reserved in this draft",
                self.owner,
            ));
        }
        if self.nodes.get(id.local_id).is_some() {
            return Err(error("node identity already registered", self.owner));
        }
        charge(fuel)?;
        self.nodes
            .insert(id.local_id, self.parents[id.local_id.raw() as usize], node);
        self.pending -= 1;
        Ok(())
    }
    /// Errors leave the draft editable. Success seals this draft and transfers
    /// its table without cloning arena nodes; the canonical Item becomes root.
    pub fn finish(
        &mut self,
        item: &'hir Item<'hir>,
        fuel: &mut usize,
    ) -> Result<OwnerInfo<'hir>, Error> {
        self.open()?;
        if item.owner_id != self.owner {
            return Err(error(
                "definition belongs to another node draft",
                self.owner,
            ));
        }
        if self.pending != 0 {
            return Err(error("unfinished node reservations", self.owner));
        }
        charge(fuel)?;
        self.nodes
            .insert(ItemLocalId::new(0), ItemLocalId::new(0), Node::Item(item));
        self.finished = true;
        self.parents.clear();
        Ok(OwnerInfo {
            node: OwnerNode::Item(item),
            nodes: std::mem::take(&mut self.nodes),
        })
    }
}
impl fmt::Display for DefinitionNodes<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(definition-nodes (owner {}) (finished {}) (pending {}) (reservations",
            self.owner, self.finished, self.pending
        )?;
        for (local, parent) in self.parents.iter().enumerate().skip(1) {
            write!(f, " (node (local {local}) (parent {parent}) (kind")?;
            if let Some(entry) = self.nodes.get(ItemLocalId::new(local as u32)) {
                write!(f, " {:?}", entry.node.kind_name())?;
            }
            f.write_str("))")?;
        }
        f.write_str("))")
    }
}
