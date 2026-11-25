//! Compact hash-consed DAG decision automaton IR and Lisp-style S-expression formatter.

use crate::bitvec::{BitInterval, BitValue};
use crate::pattern::{
    CharInterval, CompileBudget, CompileError, ErrorTag, IntInterval, TypeDesc, VariantTag,
};
use crate::sexpr;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fmt;

/// Single projection step from a scrutinee or parent place.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PlaceStep {
    TupleField(usize),
    RecordField(String),
    OptionalInner,
    ErrorOkInner,
    ErrorErrInner,
    VariantTupleField {
        tag: VariantTag,
        index: usize,
    },
    VariantRecordField {
        tag: VariantTag,
        field: String,
    },
    ListElem(usize),
    ListRest(usize),
    BitSlice {
        lo: u32,
        hi: u32,
        ty: TypeDesc,
    },
    /// Payload extracted from a `pure` / `^attrs.pattern.stable` `case` pattern view call `id`.
    CasePayload(u32),
}

impl fmt::Display for PlaceStep {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TupleField(idx) => write!(f, "(tuple-field {idx})"),
            Self::RecordField(name) => {
                f.write_str("(record-field ")?;
                sexpr::write_quoted_str(f, name)?;
                f.write_str(")")
            }
            Self::OptionalInner => f.write_str("(opt-inner)"),
            Self::ErrorOkInner => f.write_str("(ok-inner)"),
            Self::ErrorErrInner => f.write_str("(err-inner)"),
            Self::VariantTupleField { tag, index } => {
                write!(f, "(variant-field {tag} {index})")
            }
            Self::VariantRecordField { tag, field } => {
                write!(f, "(variant-record-field {tag} ")?;
                sexpr::write_quoted_str(f, field)?;
                f.write_str(")")
            }
            Self::ListElem(idx) => write!(f, "(list-elem {idx})"),
            Self::ListRest(from) => write!(f, "(list-rest {from})"),
            Self::BitSlice { lo, hi, ty } => write!(f, "(bit-slice {lo} {hi} {ty})"),
            Self::CasePayload(id) => write!(f, "(case-payload {id})"),
        }
    }
}

/// Deterministic projection path from the root match scrutinee.
#[derive(Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Place {
    pub steps: Vec<PlaceStep>,
}

impl Place {
    /// Root scrutinee place.
    pub fn root() -> Self {
        Self { steps: Vec::new() }
    }

    /// Extend this path with one additional step.
    pub fn push(&self, step: PlaceStep) -> Self {
        let mut steps = self.steps.clone();
        steps.push(step);
        Self { steps }
    }

    /// Project tuple field `index`.
    pub fn tuple_field(&self, index: usize) -> Self {
        self.push(PlaceStep::TupleField(index))
    }

    /// Project named record field `field`.
    pub fn record_field(&self, field: impl Into<String>) -> Self {
        self.push(PlaceStep::RecordField(field.into()))
    }

    /// Project `?T` present payload.
    pub fn optional_inner(&self) -> Self {
        self.push(PlaceStep::OptionalInner)
    }

    /// Project `!E T` success payload.
    pub fn error_ok_inner(&self) -> Self {
        self.push(PlaceStep::ErrorOkInner)
    }

    /// Project `!E T` error payload.
    pub fn error_err_inner(&self) -> Self {
        self.push(PlaceStep::ErrorErrInner)
    }

    /// Project enum tuple payload field `index`.
    pub fn variant_tuple_field(&self, tag: VariantTag, index: usize) -> Self {
        self.push(PlaceStep::VariantTupleField { tag, index })
    }

    /// Project enum record payload field `field`.
    pub fn variant_record_field(&self, tag: VariantTag, field: impl Into<String>) -> Self {
        self.push(PlaceStep::VariantRecordField {
            tag,
            field: field.into(),
        })
    }

    /// Project list element `index`.
    pub fn list_elem(&self, index: usize) -> Self {
        self.push(PlaceStep::ListElem(index))
    }

    /// Project list tail slice starting at `from`.
    pub fn list_rest(&self, from: usize) -> Self {
        self.push(PlaceStep::ListRest(from))
    }

    /// Project `BitVec` bit slice `[lo, hi)` interpreted as `ty`.
    pub fn bit_slice(&self, lo: u32, hi: u32, ty: TypeDesc) -> Self {
        self.push(PlaceStep::BitSlice { lo, hi, ty })
    }

    /// Project the matched payload of `case` view call `id`.
    pub fn case_payload(&self, id: u32) -> Self {
        self.push(PlaceStep::CasePayload(id))
    }
}

impl fmt::Display for Place {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("(place root")?;
        for step in &self.steps {
            write!(f, " {step}")?;
        }
        f.write_str(")")
    }
}

/// Variable binding associating a user binder name with a scrutinee projection [`Place`].
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Binding {
    pub name: String,
    pub place: Place,
}

impl fmt::Display for Binding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("(binding ")?;
        sexpr::write_quoted_str(f, &self.name)?;
        write!(f, " {})", self.place)
    }
}

/// Inclusive list-length interval `[min, max_inclusive]` (or `[min, +inf)` when `None`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LenInterval {
    pub min: usize,
    pub max_inclusive: Option<usize>,
}

impl LenInterval {
    /// Exact length `[len, len]`.
    pub const fn exact(len: usize) -> Self {
        Self {
            min: len,
            max_inclusive: Some(len),
        }
    }

    /// Unbounded length `[min, +inf)`.
    pub const fn at_least(min: usize) -> Self {
        Self {
            min,
            max_inclusive: None,
        }
    }

    /// Bounded interval `[min, max_inclusive]`.
    pub const fn range(min: usize, max_inclusive: usize) -> Self {
        Self {
            min,
            max_inclusive: Some(max_inclusive),
        }
    }

    /// Check whether `len` falls inside this interval.
    pub fn contains(&self, len: usize) -> bool {
        len >= self.min && self.max_inclusive.is_none_or(|max| len <= max)
    }
}

impl fmt::Display for LenInterval {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.max_inclusive {
            Some(max) if max == self.min => write!(f, "(len {})", self.min),
            Some(max) => write!(f, "(len-range {} {})", self.min, max),
            None => write!(f, "(len-ge {})", self.min),
        }
    }
}

/// Interned DAG node handle.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NodeId(pub usize);

impl fmt::Display for NodeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "@{}", self.0)
    }
}

/// Discriminant test and target mapping for a [`DagNode::Switch`].
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SwitchKind {
    Bool {
        on_false: NodeId,
        on_true: NodeId,
    },
    Optional {
        on_null: NodeId,
        on_present: NodeId,
    },
    Error {
        on_ok: NodeId,
        cases: BTreeMap<ErrorTag, NodeId>,
        default_err: Option<NodeId>,
    },
    Enum {
        cases: BTreeMap<VariantTag, NodeId>,
        default: Option<NodeId>,
    },
    Int {
        cases: Vec<(IntInterval, NodeId)>,
        default: Option<NodeId>,
    },
    Char {
        cases: Vec<(CharInterval, NodeId)>,
        default: Option<NodeId>,
    },
    ListLen {
        cases: Vec<(LenInterval, NodeId)>,
        default: Option<NodeId>,
    },
    BitSlice {
        lo: u32,
        hi: u32,
        cases: Vec<(BitInterval, NodeId)>,
        default: Option<NodeId>,
    },
    /// Branch on a `pure` / `^attrs.pattern.stable` `case` pattern view call `id`.
    Case {
        id: u32,
        is_predicate: bool,
        on_miss: NodeId,
        on_hit: NodeId,
    },
}

impl SwitchKind {
    /// Visit all outgoing child [`NodeId`]s in deterministic order.
    pub fn children(&self) -> Vec<NodeId> {
        let mut out = Vec::new();
        match self {
            Self::Bool { on_false, on_true } => {
                out.push(*on_false);
                out.push(*on_true);
            }
            Self::Optional {
                on_null,
                on_present,
            } => {
                out.push(*on_null);
                out.push(*on_present);
            }
            Self::Error {
                on_ok,
                cases,
                default_err,
            } => {
                out.push(*on_ok);
                for &child in cases.values() {
                    out.push(child);
                }
                if let Some(def) = default_err {
                    out.push(*def);
                }
            }
            Self::Enum { cases, default } => {
                for &child in cases.values() {
                    out.push(child);
                }
                if let Some(def) = default {
                    out.push(*def);
                }
            }
            Self::Int { cases, default } => {
                for &(_, child) in cases {
                    out.push(child);
                }
                if let Some(def) = default {
                    out.push(*def);
                }
            }
            Self::Char { cases, default } => {
                for &(_, child) in cases {
                    out.push(child);
                }
                if let Some(def) = default {
                    out.push(*def);
                }
            }
            Self::ListLen { cases, default } => {
                for &(_, child) in cases {
                    out.push(child);
                }
                if let Some(def) = default {
                    out.push(*def);
                }
            }
            Self::BitSlice { cases, default, .. } => {
                for &(_, child) in cases {
                    out.push(child);
                }
                if let Some(def) = default {
                    out.push(*def);
                }
            }
            Self::Case {
                on_miss, on_hit, ..
            } => {
                out.push(*on_miss);
                out.push(*on_hit);
            }
        }
        out
    }

    fn remap(&self, map: &[NodeId]) -> Self {
        match self {
            Self::Bool { on_false, on_true } => Self::Bool {
                on_false: map[on_false.0],
                on_true: map[on_true.0],
            },
            Self::Optional {
                on_null,
                on_present,
            } => Self::Optional {
                on_null: map[on_null.0],
                on_present: map[on_present.0],
            },
            Self::Error {
                on_ok,
                cases,
                default_err,
            } => Self::Error {
                on_ok: map[on_ok.0],
                cases: cases.iter().map(|(k, v)| (k.clone(), map[v.0])).collect(),
                default_err: default_err.map(|d| map[d.0]),
            },
            Self::Enum { cases, default } => Self::Enum {
                cases: cases.iter().map(|(k, v)| (k.clone(), map[v.0])).collect(),
                default: default.map(|d| map[d.0]),
            },
            Self::Int { cases, default } => Self::Int {
                cases: cases.iter().map(|(k, v)| (*k, map[v.0])).collect(),
                default: default.map(|d| map[d.0]),
            },
            Self::Char { cases, default } => Self::Char {
                cases: cases.iter().map(|(k, v)| (*k, map[v.0])).collect(),
                default: default.map(|d| map[d.0]),
            },
            Self::ListLen { cases, default } => Self::ListLen {
                cases: cases.iter().map(|(k, v)| (*k, map[v.0])).collect(),
                default: default.map(|d| map[d.0]),
            },
            Self::BitSlice {
                lo,
                hi,
                cases,
                default,
            } => Self::BitSlice {
                lo: *lo,
                hi: *hi,
                cases: cases.iter().map(|(k, v)| (k.clone(), map[v.0])).collect(),
                default: default.map(|d| map[d.0]),
            },
            Self::Case {
                id,
                is_predicate,
                on_miss,
                on_hit,
            } => Self::Case {
                id: *id,
                is_predicate: *is_predicate,
                on_miss: map[on_miss.0],
                on_hit: map[on_hit.0],
            },
        }
    }
}

impl fmt::Display for SwitchKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Bool { on_false, on_true } => {
                write!(f, "(bool (false {on_false}) (true {on_true}))")
            }
            Self::Optional {
                on_null,
                on_present,
            } => {
                write!(f, "(optional (null {on_null}) (present {on_present}))")
            }
            Self::Error {
                on_ok,
                cases,
                default_err,
            } => {
                write!(f, "(error (ok {on_ok}) (cases")?;
                for (tag, child) in cases {
                    write!(f, " ({tag} {child})")?;
                }
                f.write_str(")")?;
                if let Some(def) = default_err {
                    write!(f, " (default-err {def})")?;
                }
                f.write_str(")")
            }
            Self::Enum { cases, default } => {
                f.write_str("(enum (cases")?;
                for (tag, child) in cases {
                    write!(f, " ({tag} {child})")?;
                }
                f.write_str(")")?;
                if let Some(def) = default {
                    write!(f, " (default {def})")?;
                }
                f.write_str(")")
            }
            Self::Int { cases, default } => {
                f.write_str("(int (cases")?;
                for (iv, child) in cases {
                    write!(f, " ({iv} {child})")?;
                }
                f.write_str(")")?;
                if let Some(def) = default {
                    write!(f, " (default {def})")?;
                }
                f.write_str(")")
            }
            Self::Char { cases, default } => {
                f.write_str("(char (cases")?;
                for (iv, child) in cases {
                    write!(f, " ({iv} {child})")?;
                }
                f.write_str(")")?;
                if let Some(def) = default {
                    write!(f, " (default {def})")?;
                }
                f.write_str(")")
            }
            Self::ListLen { cases, default } => {
                f.write_str("(list-len (cases")?;
                for (iv, child) in cases {
                    write!(f, " ({iv} {child})")?;
                }
                f.write_str(")")?;
                if let Some(def) = default {
                    write!(f, " (default {def})")?;
                }
                f.write_str(")")
            }
            Self::BitSlice {
                lo,
                hi,
                cases,
                default,
            } => {
                write!(f, "(bit-slice (range {lo} {hi}) (cases")?;
                for (iv, child) in cases {
                    write!(f, " ({iv} {child})")?;
                }
                f.write_str(")")?;
                if let Some(def) = default {
                    write!(f, " (default {def})")?;
                }
                f.write_str(")")
            }
            Self::Case {
                id,
                is_predicate,
                on_miss,
                on_hit,
            } => write!(
                f,
                "(case (id {id}) (predicate {is_predicate}) (miss {on_miss}) (hit {on_hit}))"
            ),
        }
    }
}

/// Decision automaton DAG node.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DagNode {
    /// Branch on the constructor/range/bit-segment at `place`.
    Switch { place: Place, kind: SwitchKind },
    /// Record settled bindings for `guard_id`/arm before continuing to a shared suffix `next`.
    Bind {
        arm_index: usize,
        bindings: Vec<Binding>,
        next: NodeId,
    },
    /// Evaluate runtime guard `guard_id` with `bindings`, branching to `on_pass` or `on_fail`.
    GuardCheck {
        guard_id: usize,
        bindings: Vec<Binding>,
        on_pass: NodeId,
        on_fail: NodeId,
    },
    /// Matched arm leaf returning `arm_index` and final `bindings`.
    Leaf {
        arm_index: usize,
        bindings: Vec<Binding>,
    },
    /// No arm matched (exhaustiveness witness terminal).
    Fail,
}

impl DagNode {
    /// Outgoing child node handles.
    pub fn children(&self) -> Vec<NodeId> {
        match self {
            Self::Switch { kind, .. } => kind.children(),
            Self::Bind { next, .. } => vec![*next],
            Self::GuardCheck {
                on_pass, on_fail, ..
            } => vec![*on_pass, *on_fail],
            Self::Leaf { .. } | Self::Fail => Vec::new(),
        }
    }

    fn remap(&self, map: &[NodeId]) -> Self {
        match self {
            Self::Switch { place, kind } => Self::Switch {
                place: place.clone(),
                kind: kind.remap(map),
            },
            Self::Bind {
                arm_index,
                bindings,
                next,
            } => Self::Bind {
                arm_index: *arm_index,
                bindings: bindings.clone(),
                next: map[next.0],
            },
            Self::GuardCheck {
                guard_id,
                bindings,
                on_pass,
                on_fail,
            } => Self::GuardCheck {
                guard_id: *guard_id,
                bindings: bindings.clone(),
                on_pass: map[on_pass.0],
                on_fail: map[on_fail.0],
            },
            Self::Leaf {
                arm_index,
                bindings,
            } => Self::Leaf {
                arm_index: *arm_index,
                bindings: bindings.clone(),
            },
            Self::Fail => Self::Fail,
        }
    }
}

impl fmt::Display for DagNode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Switch { place, kind } => write!(f, "(switch {place} {kind})"),
            Self::Bind {
                arm_index,
                bindings,
                next,
            } => {
                write!(f, "(bind-frame (arm {arm_index}) (bindings")?;
                for b in bindings {
                    write!(f, " {b}")?;
                }
                write!(f, ") (next {next}))")
            }
            Self::GuardCheck {
                guard_id,
                bindings,
                on_pass,
                on_fail,
            } => {
                write!(f, "(guard-check (guard {guard_id}) (bindings")?;
                for b in bindings {
                    write!(f, " {b}")?;
                }
                write!(f, ") (pass {on_pass}) (fail {on_fail}))")
            }
            Self::Leaf {
                arm_index,
                bindings,
            } => {
                write!(f, "(leaf (arm {arm_index}) (bindings")?;
                for b in bindings {
                    write!(f, " {b}")?;
                }
                f.write_str("))")
            }
            Self::Fail => f.write_str("(fail)"),
        }
    }
}

/// Finalized compact decision DAG with densely numbered reachable nodes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DecisionDag {
    pub root: NodeId,
    pub nodes: Vec<DagNode>,
}

impl DecisionDag {
    /// Total number of nodes in the compacted DAG.
    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    /// Retrieve node by [`NodeId`].
    pub fn node(&self, id: NodeId) -> &DagNode {
        &self.nodes[id.0]
    }

    /// Collect all reachable [`Leaf`](DagNode::Leaf) arm indices in deterministic order.
    pub fn reachable_arms(&self) -> BTreeSet<usize> {
        let mut out = BTreeSet::new();
        for node in &self.nodes {
            if let DagNode::Leaf { arm_index, .. } = node {
                out.insert(*arm_index);
            }
        }
        out
    }

    /// Return `true` if any reachable node is [`DagNode::Fail`].
    pub fn has_fail(&self) -> bool {
        self.nodes.iter().any(|node| matches!(node, DagNode::Fail))
    }
}

impl fmt::Display for DecisionDag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "(decision-dag (root {}) (nodes", self.root)?;
        for (idx, node) in self.nodes.iter().enumerate() {
            write!(f, " (node @{idx} {node})")?;
        }
        f.write_str("))")
    }
}

/// Structural hash-consing builder for [`DecisionDag`].
#[derive(Clone, Debug)]
pub struct DagBuilder {
    nodes: Vec<DagNode>,
    intern_map: BTreeMap<DagNode, NodeId>,
    budget: CompileBudget,
    steps_used: usize,
}

impl DagBuilder {
    /// Create a new builder bounded by `budget`.
    pub fn new(budget: CompileBudget) -> Self {
        Self {
            nodes: Vec::new(),
            intern_map: BTreeMap::new(),
            budget,
            steps_used: 0,
        }
    }

    /// Charge one specialization step against `budget`.
    pub fn charge_step(&mut self) -> Result<(), CompileError> {
        self.steps_used = self.steps_used.saturating_add(1);
        if self.steps_used > self.budget.max_steps {
            return Err(CompileError::BudgetExceeded {
                steps_used: self.steps_used,
                nodes_created: self.nodes.len(),
                budget: self.budget,
            });
        }
        Ok(())
    }

    /// Current step counter.
    pub fn steps_used(&self) -> usize {
        self.steps_used
    }

    /// Total unique interned nodes allocated so far.
    pub fn allocated_nodes(&self) -> usize {
        self.nodes.len()
    }

    /// Intern an arbitrary [`DagNode`], enforcing the node budget.
    pub fn intern(&mut self, node: DagNode) -> Result<NodeId, CompileError> {
        if let Some(&id) = self.intern_map.get(&node) {
            return Ok(id);
        }
        if self.nodes.len() >= self.budget.max_nodes {
            return Err(CompileError::BudgetExceeded {
                steps_used: self.steps_used,
                nodes_created: self.nodes.len() + 1,
                budget: self.budget,
            });
        }
        let id = NodeId(self.nodes.len());
        self.nodes.push(node.clone());
        self.intern_map.insert(node, id);
        Ok(id)
    }

    /// Intern [`DagNode::Fail`].
    pub fn fail(&mut self) -> Result<NodeId, CompileError> {
        self.intern(DagNode::Fail)
    }

    /// Intern [`DagNode::Leaf`] with normalized sorted bindings.
    pub fn leaf(
        &mut self,
        arm_index: usize,
        mut bindings: Vec<Binding>,
    ) -> Result<NodeId, CompileError> {
        normalize_bindings(&mut bindings);
        self.intern(DagNode::Leaf {
            arm_index,
            bindings,
        })
    }

    /// Intern [`DagNode::Bind`], eliding empty binding frames.
    pub fn bind(
        &mut self,
        arm_index: usize,
        mut bindings: Vec<Binding>,
        next: NodeId,
    ) -> Result<NodeId, CompileError> {
        normalize_bindings(&mut bindings);
        if bindings.is_empty() {
            return Ok(next);
        }
        self.intern(DagNode::Bind {
            arm_index,
            bindings,
            next,
        })
    }

    /// Intern [`DagNode::GuardCheck`] with normalized sorted bindings.
    pub fn guard_check(
        &mut self,
        guard_id: usize,
        mut bindings: Vec<Binding>,
        on_pass: NodeId,
        on_fail: NodeId,
    ) -> Result<NodeId, CompileError> {
        normalize_bindings(&mut bindings);
        if on_pass == on_fail {
            return Ok(on_pass);
        }
        self.intern(DagNode::GuardCheck {
            guard_id,
            bindings,
            on_pass,
            on_fail,
        })
    }

    /// Simplify and intern a [`DagNode::Switch`], collapsing redundant switches and
    /// coalescing adjacent intervals with identical targets.
    pub fn switch(&mut self, place: Place, kind: SwitchKind) -> Result<NodeId, CompileError> {
        let simplified = match kind {
            SwitchKind::Bool { on_false, on_true } => {
                if on_false == on_true {
                    return Ok(on_false);
                }
                SwitchKind::Bool { on_false, on_true }
            }
            SwitchKind::Optional {
                on_null,
                on_present,
            } => {
                if on_null == on_present {
                    return Ok(on_null);
                }
                SwitchKind::Optional {
                    on_null,
                    on_present,
                }
            }
            SwitchKind::Error {
                on_ok,
                cases,
                default_err,
            } => {
                let all_same =
                    cases.values().all(|&c| c == on_ok) && default_err.is_none_or(|d| d == on_ok);
                if all_same {
                    return Ok(on_ok);
                }
                SwitchKind::Error {
                    on_ok,
                    cases,
                    default_err,
                }
            }
            SwitchKind::Enum { cases, default } => {
                if cases.is_empty()
                    && let Some(def) = default
                {
                    return Ok(def);
                }
                if !cases.is_empty() {
                    let first = *cases.values().next().expect("checked non-empty");
                    let all_same =
                        cases.values().all(|&c| c == first) && default.is_none_or(|d| d == first);
                    if all_same {
                        return Ok(first);
                    }
                }
                SwitchKind::Enum { cases, default }
            }
            SwitchKind::Int { cases, default } => {
                let (cases, default) = simplify_interval_switch(cases, default, |a, b| {
                    match (a.hi_finite(), b.lo_finite()) {
                        (Some(hi), Some(lo)) => hi.add_one() == lo,
                        _ => false,
                    }
                });
                if cases.is_empty()
                    && let Some(def) = default
                {
                    return Ok(def);
                }
                if default.is_none() && !cases.is_empty() {
                    let first = cases[0].1;
                    if cases.iter().all(|&(_, target)| target == first) {
                        return Ok(first);
                    }
                }
                SwitchKind::Int { cases, default }
            }
            SwitchKind::Char { cases, default } => {
                let (cases, default) = simplify_interval_switch(cases, default, |a, b| {
                    (a.hi as u32) + 1 == (b.lo as u32)
                });
                if cases.is_empty()
                    && let Some(def) = default
                {
                    return Ok(def);
                }
                if default.is_none() && !cases.is_empty() {
                    let first = cases[0].1;
                    if cases.iter().all(|&(_, target)| target == first) {
                        return Ok(first);
                    }
                }
                SwitchKind::Char { cases, default }
            }
            SwitchKind::ListLen { cases, default } => {
                let (cases, default) =
                    simplify_interval_switch(cases, default, |a, b| match a.max_inclusive {
                        Some(max) => max.saturating_add(1) == b.min,
                        None => false,
                    });
                if cases.is_empty()
                    && let Some(def) = default
                {
                    return Ok(def);
                }
                if default.is_none() && !cases.is_empty() {
                    let first = cases[0].1;
                    if cases.iter().all(|&(_, target)| target == first) {
                        return Ok(first);
                    }
                }
                SwitchKind::ListLen { cases, default }
            }
            SwitchKind::BitSlice {
                lo,
                hi,
                cases,
                default,
            } => {
                let (cases, default) = simplify_interval_switch(cases, default, |a, b| {
                    a.hi.checked_add_one().is_some_and(|next| next == b.lo)
                });
                if cases.is_empty()
                    && let Some(def) = default
                {
                    return Ok(def);
                }
                if default.is_none() && !cases.is_empty() {
                    let first = cases[0].1;
                    if cases.iter().all(|&(_, target)| target == first) {
                        return Ok(first);
                    }
                }
                SwitchKind::BitSlice {
                    lo,
                    hi,
                    cases,
                    default,
                }
            }
            SwitchKind::Case {
                id,
                is_predicate,
                on_miss,
                on_hit,
            } => {
                if on_miss == on_hit {
                    return Ok(on_miss);
                }
                SwitchKind::Case {
                    id,
                    is_predicate,
                    on_miss,
                    on_hit,
                }
            }
        };

        self.intern(DagNode::Switch {
            place,
            kind: simplified,
        })
    }

    /// Compact unreachable interned nodes and renumber live nodes densely from `root` (`@0`).
    pub fn finalize(self, root: NodeId) -> DecisionDag {
        let mut reachable = vec![false; self.nodes.len()];
        let mut order = Vec::new();
        let mut queue = VecDeque::new();
        reachable[root.0] = true;
        queue.push_back(root);

        while let Some(id) = queue.pop_front() {
            order.push(id);
            for child in self.nodes[id.0].children() {
                if !reachable[child.0] {
                    reachable[child.0] = true;
                    queue.push_back(child);
                }
            }
        }

        let mut remap = vec![NodeId(0); self.nodes.len()];
        for (new_idx, &old_id) in order.iter().enumerate() {
            remap[old_id.0] = NodeId(new_idx);
        }

        let compact_nodes: Vec<DagNode> = order
            .iter()
            .map(|&old_id| self.nodes[old_id.0].remap(&remap))
            .collect();

        DecisionDag {
            root: remap[root.0],
            nodes: compact_nodes,
        }
    }
}

fn normalize_bindings(bindings: &mut Vec<Binding>) {
    let mut map = BTreeMap::new();
    for b in bindings.drain(..) {
        map.insert(b.name, b.place);
    }
    *bindings = map
        .into_iter()
        .map(|(name, place)| Binding { name, place })
        .collect();
}

fn simplify_interval_switch<I, F>(
    cases: Vec<(I, NodeId)>,
    default: Option<NodeId>,
    can_merge: F,
) -> (Vec<(I, NodeId)>, Option<NodeId>)
where
    I: Clone,
    F: Fn(&I, &I) -> bool,
{
    let mut merged: Vec<(I, NodeId)> = Vec::with_capacity(cases.len());
    for (iv, target) in cases {
        if default == Some(target) {
            continue;
        }
        if let Some((last_iv, last_target)) = merged.last_mut()
            && *last_target == target
            && can_merge(last_iv, &iv)
        {
            // Merge adjacent intervals by updating upper bound via reconstruction outside,
            // handled specifically below for each interval type or kept merged cleanly.
            let _ = last_iv;
        }
        merged.push((iv, target));
    }
    (merged, default)
}

/// Coalesce adjacent interval cases that share a target `NodeId` when rebuilding switches.
pub(crate) fn coalesce_int_cases(cases: Vec<(IntInterval, NodeId)>) -> Vec<(IntInterval, NodeId)> {
    let mut out: Vec<(IntInterval, NodeId)> = Vec::with_capacity(cases.len());
    for (iv, target) in cases {
        if let Some((last_iv, last_target)) = out.last_mut()
            && *last_target == target
            && let (Some(hi), Some(lo)) = (last_iv.hi_finite(), iv.lo_finite())
            && hi.add_one() == lo
        {
            last_iv.hi = iv.hi;
            continue;
        }
        out.push((iv, target));
    }
    out
}

/// Coalesce adjacent character intervals sharing a target `NodeId`.
pub(crate) fn coalesce_char_cases(
    cases: Vec<(CharInterval, NodeId)>,
) -> Vec<(CharInterval, NodeId)> {
    let mut out: Vec<(CharInterval, NodeId)> = Vec::with_capacity(cases.len());
    for (iv, target) in cases {
        if let Some((last_iv, last_target)) = out.last_mut()
            && *last_target == target
            && (last_iv.hi as u32) + 1 == (iv.lo as u32)
        {
            last_iv.hi = iv.hi;
            continue;
        }
        out.push((iv, target));
    }
    out
}

/// Coalesce adjacent list length intervals sharing a target `NodeId`.
pub(crate) fn coalesce_len_cases(cases: Vec<(LenInterval, NodeId)>) -> Vec<(LenInterval, NodeId)> {
    let mut out: Vec<(LenInterval, NodeId)> = Vec::with_capacity(cases.len());
    for (iv, target) in cases {
        if let Some((last_iv, last_target)) = out.last_mut()
            && *last_target == target
            && let Some(max) = last_iv.max_inclusive
            && max.saturating_add(1) == iv.min
        {
            last_iv.max_inclusive = iv.max_inclusive;
            continue;
        }
        out.push((iv, target));
    }
    out
}

/// Coalesce adjacent bit intervals sharing a target `NodeId`.
pub(crate) fn coalesce_bit_cases(cases: Vec<(BitInterval, NodeId)>) -> Vec<(BitInterval, NodeId)> {
    let mut out: Vec<(BitInterval, NodeId)> = Vec::with_capacity(cases.len());
    for (iv, target) in cases {
        if let Some((last_iv, last_target)) = out.last_mut()
            && *last_target == target
            && last_iv
                .hi
                .checked_add_one()
                .is_some_and(|next| next == iv.lo)
        {
            last_iv.hi = iv.hi;
            continue;
        }
        out.push((iv, target));
    }
    out
}

/// Utility helper formatting a slice of [`BitValue`]s cleanly in S-expressions.
#[allow(dead_code)]
pub(crate) fn write_bit_list(f: &mut fmt::Formatter<'_>, values: &[BitValue]) -> fmt::Result {
    for (idx, val) in values.iter().enumerate() {
        if idx > 0 {
            f.write_str(" ")?;
        }
        write!(f, "{val}")?;
    }
    Ok(())
}
