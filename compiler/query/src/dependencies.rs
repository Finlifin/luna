//! Invocation-local dependency identities, independent of diagnostic key text.
use crate::DepNode;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct NodeId(u64);
impl NodeId {
    pub(crate) fn fresh() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        Self(
            NEXT.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_add(1))
                .expect("query identity space exhausted"),
        )
    }
}

#[derive(Default)]
struct Node {
    label: Option<DepNode>,
    reads: BTreeSet<NodeId>,
    users: BTreeSet<NodeId>,
    dirty: bool,
}
#[derive(Default)]
pub(crate) struct Dependencies {
    nodes: BTreeMap<NodeId, Node>,
    active: Vec<(NodeId, BTreeSet<NodeId>)>,
}
impl Dependencies {
    pub(crate) fn read(&mut self, id: NodeId, label: DepNode) -> bool {
        if let Some((_, reads)) = self.active.last_mut() {
            reads.insert(id);
        }
        let node = self.nodes.entry(id).or_default();
        node.label = Some(label);
        std::mem::take(&mut node.dirty)
    }
    pub(crate) fn begin(&mut self, id: NodeId) {
        self.active.push((id, BTreeSet::new()));
    }
    pub(crate) fn finish(&mut self) {
        let (id, reads) = self.active.pop().expect("active dependency frame");
        let old = std::mem::replace(&mut self.nodes.entry(id).or_default().reads, reads.clone());
        for dependency in old {
            self.nodes
                .get_mut(&dependency)
                .expect("dependency node")
                .users
                .remove(&id);
        }
        for dependency in reads {
            self.nodes.entry(dependency).or_default().users.insert(id);
        }
    }
    pub(crate) fn invalidate(&mut self, root: NodeId) {
        let mut pending = vec![root];
        let mut visited = BTreeSet::new();
        while let Some(id) = pending.pop() {
            if !visited.insert(id) {
                continue;
            }
            let node = self.nodes.entry(id).or_default();
            node.dirty = true;
            pending.extend(node.users.iter().copied());
        }
    }
}
impl std::fmt::Display for Dependencies {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("(query-dependencies")?;
        for (id, node) in &self.nodes {
            write!(f, " (node {}", id.0)?;
            if let Some(label) = &node.label {
                write!(f, " (query {:?}) (key {:?})", label.query, label.key)?;
            }
            write!(f, " (dirty {}) (reads", node.dirty)?;
            for dependency in &node.reads {
                write!(f, " {}", dependency.0)?;
            }
            f.write_str("))")?;
        }
        f.write_str(")")
    }
}
