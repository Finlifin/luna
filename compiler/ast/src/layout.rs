//! Storage arity validation before consumers index node slots.
use crate::{Ast, NodeIndex, NodeKind, NodeType};
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LayoutError {
    pub node: NodeIndex,
    pub expected: usize,
    pub actual: Option<usize>,
}
impl fmt::Display for LayoutError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(ast-layout-error (node {}) (expected-slots {}) (actual-slots ",
            self.node, self.expected
        )?;
        match self.actual {
            Some(actual) => write!(f, "{actual}")?,
            None => f.write_str("unavailable")?,
        }
        f.write_str("))")
    }
}
impl std::error::Error for LayoutError {}

impl NodeType {
    /// Number of direct slots, including list handles and raw modifier flags.
    fn structural_slots(self) -> usize {
        use NodeType::*;
        match self {
            NoChild => 0,
            SingleChild | MultiChildren => 1,
            DoubleChildren | SingleWithMultiChildren | FnTypeChildren => 2,
            TripleChildren | DoubleWithMultiChildren | TypeDefChildren | TypeAliasChildren => 3,
            QuadrupleChildren
            | TripleWithMultiChildren
            | TraitDefChildren
            | ImplTraitDefChildren
            | ExtendTraitDefChildren => 4,
            FunctionDefChildren
            | NormalFormHeadDefChildren
            | AlgebraicEffectChildren
            | AssocDeclChildren => 5,
        }
    }
}
impl NodeKind {
    pub fn storage_slots(self) -> usize {
        match self {
            // These are leaves for visitors, but retain token identity data.
            Self::Id => 2,
            Self::Symbol => 1,
            _ => self.node_type().structural_slots(),
        }
    }
}
impl Ast {
    /// Check node-local storage, including nodes retained during parser
    /// backtracking. This does not establish graph acyclicity or name validity.
    pub fn validate_node_layouts(&self) -> Result<(), LayoutError> {
        for (index, kind) in self.nodes.iter().enumerate().skip(1) {
            let node = index as NodeIndex;
            let expected = kind.storage_slots();
            let actual = self.get_node(node).map(|(_, _, children)| children.len());
            if actual != Some(expected) {
                return Err(LayoutError {
                    node,
                    expected,
                    actual,
                });
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StructureError {
    Layout(LayoutError),
    Reference {
        node: NodeIndex,
        slot: usize,
        element: Option<usize>,
        target: NodeIndex,
        list: bool,
    },
}
impl StructureError {
    pub fn node(&self) -> NodeIndex {
        match self {
            Self::Layout(error) => error.node,
            Self::Reference { node, .. } => *node,
        }
    }
}
impl fmt::Display for StructureError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Layout(error) => error.fmt(f),
            Self::Reference {
                node,
                slot,
                element,
                target,
                list,
            } => {
                write!(
                    f,
                    "(ast-reference-error (node {node}) (slot {slot}) (element "
                )?;
                match element {
                    Some(index) => write!(f, "{index}")?,
                    None => f.write_str("none")?,
                }
                write!(
                    f,
                    ") (target {target}) (kind {}))",
                    if *list { "list" } else { "node" }
                )
            }
        }
    }
}
impl std::error::Error for StructureError {}

impl NodeType {
    fn list_slots(self) -> &'static [usize] {
        use NodeType::*;
        match self {
            MultiChildren => &[0],
            SingleWithMultiChildren | TypeDefChildren | TypeAliasChildren | FnTypeChildren => &[1],
            DoubleWithMultiChildren
            | TraitDefChildren
            | ImplTraitDefChildren
            | ExtendTraitDefChildren => &[2],
            TripleWithMultiChildren => &[3],
            FunctionDefChildren | NormalFormHeadDefChildren | AlgebraicEffectChildren => &[1, 3],
            AssocDeclChildren => &[1, 4],
            _ => &[],
        }
    }
}
impl Ast {
    /// Validate storage and referenced handles without following graph edges.
    /// Zero direct-node slots denote absent optional syntax. List elements
    /// must be actual nodes. Raw symbol words and function flags are not IDs.
    /// Cycles and language-specific required/optional positions are separate.
    pub fn validate_structure(&self) -> Result<(), StructureError> {
        self.validate_node_layouts()
            .map_err(StructureError::Layout)?;
        for (index, kind) in self.nodes.iter().enumerate().skip(1) {
            let node = index as NodeIndex;
            let lists = kind.node_type().list_slots();
            for (slot, &target) in self.get_children(node).iter().enumerate() {
                if *kind == NodeKind::Id || (*kind == NodeKind::FnType && slot == 0) {
                    continue;
                }
                if lists.contains(&slot) {
                    let values =
                        self.get_multi_child_slice(target)
                            .ok_or(StructureError::Reference {
                                node,
                                slot,
                                element: None,
                                target,
                                list: true,
                            })?;
                    for (element, &target) in values.iter().enumerate() {
                        if self.get_node_kind(target).is_none() {
                            return Err(StructureError::Reference {
                                node,
                                slot,
                                element: Some(element),
                                target,
                                list: false,
                            });
                        }
                    }
                } else if target != 0 && self.get_node_kind(target).is_none() {
                    return Err(StructureError::Reference {
                        node,
                        slot,
                        element: None,
                        target,
                        list: false,
                    });
                }
            }
        }
        Ok(())
    }
}
