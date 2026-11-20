//! Zero-width (`BitVec<0>`) elimination pass.
//!
//! Normalizes empty bit-vector operations (`empty ++ x -> x`, `x ++ empty -> x`,
//! `not empty -> empty`, `empty == empty -> true`, `x[k..k] -> empty`, and
//! `zero_extend(empty, w) -> 0`) so downstream fixed-width SMT solvers only see
//! positive-width pure nodes whenever possible, while strictly preserving all
//! boundary inputs, side-effecting calls, dynamic bounds checks, and cleanups.

use std::collections::BTreeMap;
use std::fmt::{self, Display};

use crate::ir::{
    BitValue, BoundaryCheck, DomainTerm, ExtractStart, InputIdentity, IrError, Node, NodeId, Region,
};
use crate::sexpr::write_list;

/// Specific zero-width rewrite applied to a DAG node.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ZeroWidthAction {
    ZeroWidthInput {
        node: NodeId,
        identity: InputIdentity,
    },
    ConcatBothEmpty {
        node: NodeId,
    },
    ConcatLeftEmpty {
        node: NodeId,
        kept: NodeId,
    },
    ConcatRightEmpty {
        node: NodeId,
        kept: NodeId,
    },
    SliceEmpty {
        node: NodeId,
        dynamic_bounds_preserved: bool,
    },
    AndEmpty {
        node: NodeId,
    },
    OrEmpty {
        node: NodeId,
    },
    NotEmpty {
        node: NodeId,
    },
    EqualEmpty {
        node: NodeId,
    },
    ZeroExtendEmpty {
        node: NodeId,
        target_width: u32,
    },
}

impl Display for ZeroWidthAction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ZeroWidthInput { node, identity } => {
                write!(f, "(zw-input {node} {identity})")
            }
            Self::ConcatBothEmpty { node } => write!(f, "(zw-concat-both-empty {node})"),
            Self::ConcatLeftEmpty { node, kept } => {
                write!(f, "(zw-concat-left-empty {node} (kept {kept}))")
            }
            Self::ConcatRightEmpty { node, kept } => {
                write!(f, "(zw-concat-right-empty {node} (kept {kept}))")
            }
            Self::SliceEmpty {
                node,
                dynamic_bounds_preserved,
            } => write!(
                f,
                "(zw-slice-empty {node} (dynamic-bounds-preserved {dynamic_bounds_preserved}))"
            ),
            Self::AndEmpty { node } => write!(f, "(zw-and-empty {node})"),
            Self::OrEmpty { node } => write!(f, "(zw-or-empty {node})"),
            Self::NotEmpty { node } => write!(f, "(zw-not-empty {node})"),
            Self::EqualEmpty { node } => write!(f, "(zw-equal-empty {node})"),
            Self::ZeroExtendEmpty { node, target_width } => {
                write!(f, "(zw-zext-empty {node} (target-width {target_width}))")
            }
        }
    }
}

/// Output of the zero-width elimination pass.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ZeroWidthResult {
    pub region: Region,
    pub actions: Vec<ZeroWidthAction>,
}

impl Display for ZeroWidthResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("(zero-width-result (actions ")?;
        write_list(f, &self.actions)?;
        write!(f, ") (region {}))", self.region)
    }
}

/// Eliminates zero-width pure operations while keeping `BehaviorBoundary` intact.
///
/// Never drops boundary inputs, side-effect calls, dynamic bounds checks, or
/// resource cleanups—even when the resulting value is empty or constant zero.
pub fn eliminate_zero_width(region: &Region) -> Result<ZeroWidthResult, IrError> {
    region.validate()?;
    let orig_widths = region.node_widths()?;
    let mut builder = ZwBuilder::default();
    let mut remap: Vec<NodeId> = vec![NodeId(0); region.nodes.len()];
    let mut actions: Vec<ZeroWidthAction> = Vec::new();
    let mut boundary = region.boundary.clone();

    for (idx, node) in region.nodes.iter().enumerate() {
        let orig_id = NodeId(idx);
        let mapped = match node {
            Node::Input { width, identity } => {
                if *width == 0 {
                    actions.push(ZeroWidthAction::ZeroWidthInput {
                        node: orig_id,
                        identity: identity.clone(),
                    });
                    builder.intern(
                        Node::Constant {
                            width: 0,
                            bits: BitValue::empty(),
                        },
                        0,
                    )
                } else {
                    builder.intern(
                        Node::Input {
                            width: *width,
                            identity: identity.clone(),
                        },
                        *width,
                    )
                }
            }
            Node::Constant { width, bits } => builder.intern(
                Node::Constant {
                    width: *width,
                    bits: bits.clone(),
                },
                *width,
            ),
            Node::Concat(hi, lo) => {
                let m_hi = remap[hi.0];
                let m_lo = remap[lo.0];
                let w_hi = builder.width(m_hi);
                let w_lo = builder.width(m_lo);
                match (w_hi, w_lo) {
                    (0, 0) => {
                        actions.push(ZeroWidthAction::ConcatBothEmpty { node: orig_id });
                        builder.intern(
                            Node::Constant {
                                width: 0,
                                bits: BitValue::empty(),
                            },
                            0,
                        )
                    }
                    (0, _) => {
                        actions.push(ZeroWidthAction::ConcatLeftEmpty {
                            node: orig_id,
                            kept: m_lo,
                        });
                        m_lo
                    }
                    (_, 0) => {
                        actions.push(ZeroWidthAction::ConcatRightEmpty {
                            node: orig_id,
                            kept: m_hi,
                        });
                        m_hi
                    }
                    _ => builder.intern(Node::Concat(m_hi, m_lo), w_hi + w_lo),
                }
            }
            Node::Extract {
                input,
                start,
                width,
            } => {
                let m_in = remap[input.0];
                let in_w = orig_widths[input.0];
                if *width == 0 {
                    let mut dynamic_bounds_preserved = false;
                    if let ExtractStart::Dynamic(idx_node) = start {
                        dynamic_bounds_preserved = true;
                        if let Some(index_term) = node_to_domain_term(&region.nodes, *idx_node) {
                            let already_present = boundary.checks.iter().any(|chk| {
                                matches!(
                                    chk,
                                    BoundaryCheck::DynamicBoundsCheck {
                                        index_term: existing_term,
                                        extract_width: 0,
                                        input_width: existing_w,
                                        ..
                                    } if existing_term == &index_term && *existing_w == in_w
                                )
                            });
                            if !already_present {
                                let next_order = boundary
                                    .checks
                                    .iter()
                                    .map(BoundaryCheck::check_order)
                                    .max()
                                    .map_or(0, |m| m + 1);
                                boundary.checks.push(BoundaryCheck::DynamicBoundsCheck {
                                    index_term,
                                    extract_width: 0,
                                    input_width: in_w,
                                    check_order: next_order,
                                });
                            }
                        }
                    }
                    actions.push(ZeroWidthAction::SliceEmpty {
                        node: orig_id,
                        dynamic_bounds_preserved,
                    });
                    builder.intern(
                        Node::Constant {
                            width: 0,
                            bits: BitValue::empty(),
                        },
                        0,
                    )
                } else {
                    let m_start = match start {
                        ExtractStart::Const(k) => ExtractStart::Const(*k),
                        ExtractStart::Dynamic(idx_node) => ExtractStart::Dynamic(remap[idx_node.0]),
                    };
                    builder.intern(
                        Node::Extract {
                            input: m_in,
                            start: m_start,
                            width: *width,
                        },
                        *width,
                    )
                }
            }
            Node::And(l, r) => {
                let ml = remap[l.0];
                let mr = remap[r.0];
                let w = builder.width(ml);
                if w == 0 {
                    actions.push(ZeroWidthAction::AndEmpty { node: orig_id });
                    builder.intern(
                        Node::Constant {
                            width: 0,
                            bits: BitValue::empty(),
                        },
                        0,
                    )
                } else {
                    builder.intern(Node::And(ml, mr), w)
                }
            }
            Node::Or(l, r) => {
                let ml = remap[l.0];
                let mr = remap[r.0];
                let w = builder.width(ml);
                if w == 0 {
                    actions.push(ZeroWidthAction::OrEmpty { node: orig_id });
                    builder.intern(
                        Node::Constant {
                            width: 0,
                            bits: BitValue::empty(),
                        },
                        0,
                    )
                } else {
                    builder.intern(Node::Or(ml, mr), w)
                }
            }
            Node::Not(inner) => {
                let m_in = remap[inner.0];
                let w = builder.width(m_in);
                if w == 0 {
                    actions.push(ZeroWidthAction::NotEmpty { node: orig_id });
                    builder.intern(
                        Node::Constant {
                            width: 0,
                            bits: BitValue::empty(),
                        },
                        0,
                    )
                } else {
                    builder.intern(Node::Not(m_in), w)
                }
            }
            Node::Equal(l, r) => {
                let ml = remap[l.0];
                let mr = remap[r.0];
                let w = builder.width(ml);
                if w == 0 {
                    actions.push(ZeroWidthAction::EqualEmpty { node: orig_id });
                    builder.intern(
                        Node::Constant {
                            width: 1,
                            bits: BitValue::from_bool(true),
                        },
                        1,
                    )
                } else {
                    builder.intern(Node::Equal(ml, mr), 1)
                }
            }
            Node::ZeroExtend { input, width } => {
                let m_in = remap[input.0];
                let in_w = builder.width(m_in);
                if in_w == 0 {
                    actions.push(ZeroWidthAction::ZeroExtendEmpty {
                        node: orig_id,
                        target_width: *width,
                    });
                    builder.intern(
                        Node::Constant {
                            width: *width,
                            bits: BitValue::zeros(*width),
                        },
                        *width,
                    )
                } else {
                    builder.intern(
                        Node::ZeroExtend {
                            input: m_in,
                            width: *width,
                        },
                        *width,
                    )
                }
            }
        };
        remap[idx] = mapped;
    }

    let new_root = remap[region.root.0];
    let compacted = compact_reachable(
        &region.inputs,
        &region.domain,
        &builder.nodes,
        new_root,
        region.output_kind,
        boundary,
    )?;
    compacted.validate()?;

    Ok(ZeroWidthResult {
        region: compacted,
        actions,
    })
}

#[derive(Default)]
struct ZwBuilder {
    nodes: Vec<Node>,
    widths: Vec<u32>,
    intern: BTreeMap<Node, NodeId>,
}

impl ZwBuilder {
    fn intern(&mut self, node: Node, w: u32) -> NodeId {
        if let Some(&existing) = self.intern.get(&node) {
            return existing;
        }
        let id = NodeId(self.nodes.len());
        self.intern.insert(node.clone(), id);
        self.nodes.push(node);
        self.widths.push(w);
        id
    }

    fn width(&self, id: NodeId) -> u32 {
        self.widths[id.0]
    }
}

/// Converts a pure DAG subgraph (without `Equal` or dynamic `Extract`) into a [`DomainTerm`].
pub fn node_to_domain_term(nodes: &[Node], id: NodeId) -> Option<DomainTerm> {
    match &nodes[id.0] {
        Node::Input { width, identity } => Some(DomainTerm::Input {
            identity: identity.clone(),
            width: *width,
        }),
        Node::Constant { bits, .. } => Some(DomainTerm::Constant(bits.clone())),
        Node::Concat(hi, lo) => Some(DomainTerm::Concat(
            Box::new(node_to_domain_term(nodes, *hi)?),
            Box::new(node_to_domain_term(nodes, *lo)?),
        )),
        Node::Extract {
            input,
            start: ExtractStart::Const(start),
            width,
        } => Some(DomainTerm::Extract {
            term: Box::new(node_to_domain_term(nodes, *input)?),
            start: *start,
            width: *width,
        }),
        Node::Extract {
            start: ExtractStart::Dynamic(_),
            ..
        }
        | Node::Equal(_, _) => None,
        Node::And(l, r) => Some(DomainTerm::And(
            Box::new(node_to_domain_term(nodes, *l)?),
            Box::new(node_to_domain_term(nodes, *r)?),
        )),
        Node::Or(l, r) => Some(DomainTerm::Or(
            Box::new(node_to_domain_term(nodes, *l)?),
            Box::new(node_to_domain_term(nodes, *r)?),
        )),
        Node::Not(inner) => Some(DomainTerm::Not(Box::new(node_to_domain_term(
            nodes, *inner,
        )?))),
        Node::ZeroExtend { input, width } => Some(DomainTerm::ZeroExtend {
            term: Box::new(node_to_domain_term(nodes, *input)?),
            width: *width,
        }),
    }
}

/// Prunes unreachable pure DAG nodes while preserving all declared inputs, domain, and boundary.
pub(crate) fn compact_reachable(
    inputs: &BTreeMap<InputIdentity, u32>,
    domain: &crate::ir::DomainConstraint,
    nodes: &[Node],
    root: NodeId,
    output_kind: crate::ir::OutputKind,
    boundary: crate::ir::BehaviorBoundary,
) -> Result<Region, IrError> {
    let mut visited = vec![false; nodes.len()];
    let mut order = Vec::new();
    mark_reachable(nodes, root, &mut visited, &mut order);

    let mut remap = vec![NodeId(0); nodes.len()];
    let mut out_nodes = Vec::with_capacity(order.len());
    for old_id in order {
        let new_node = match &nodes[old_id.0] {
            Node::Input { width, identity } => Node::Input {
                width: *width,
                identity: identity.clone(),
            },
            Node::Constant { width, bits } => Node::Constant {
                width: *width,
                bits: bits.clone(),
            },
            Node::Concat(l, r) => Node::Concat(remap[l.0], remap[r.0]),
            Node::Extract {
                input,
                start,
                width,
            } => Node::Extract {
                input: remap[input.0],
                start: match start {
                    ExtractStart::Const(k) => ExtractStart::Const(*k),
                    ExtractStart::Dynamic(d) => ExtractStart::Dynamic(remap[d.0]),
                },
                width: *width,
            },
            Node::And(l, r) => Node::And(remap[l.0], remap[r.0]),
            Node::Or(l, r) => Node::Or(remap[l.0], remap[r.0]),
            Node::Not(inner) => Node::Not(remap[inner.0]),
            Node::Equal(l, r) => Node::Equal(remap[l.0], remap[r.0]),
            Node::ZeroExtend { input, width } => Node::ZeroExtend {
                input: remap[input.0],
                width: *width,
            },
        };
        let nid = NodeId(out_nodes.len());
        remap[old_id.0] = nid;
        out_nodes.push(new_node);
    }

    Ok(Region {
        inputs: inputs.clone(),
        domain: domain.clone(),
        nodes: out_nodes,
        root: remap[root.0],
        output_kind,
        boundary,
    })
}

fn mark_reachable(nodes: &[Node], id: NodeId, visited: &mut [bool], order: &mut Vec<NodeId>) {
    if visited[id.0] {
        return;
    }
    visited[id.0] = true;
    match &nodes[id.0] {
        Node::Input { .. } | Node::Constant { .. } => {}
        Node::Concat(l, r) | Node::And(l, r) | Node::Or(l, r) | Node::Equal(l, r) => {
            mark_reachable(nodes, *l, visited, order);
            mark_reachable(nodes, *r, visited, order);
        }
        Node::Extract { input, start, .. } => {
            mark_reachable(nodes, *input, visited, order);
            if let ExtractStart::Dynamic(d) = start {
                mark_reachable(nodes, *d, visited, order);
            }
        }
        Node::Not(inner) | Node::ZeroExtend { input: inner, .. } => {
            mark_reachable(nodes, *inner, visited, order);
        }
    }
    order.push(id);
}
