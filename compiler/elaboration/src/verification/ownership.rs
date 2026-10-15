//! Availability of disjoint owned positions. No alias, Copy, Drop or pointer
//! permission is inferred here. Callers must establish those separate contracts.
use super::PlaceId;
use std::{collections::BTreeMap, fmt};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Shape {
    Value,
    Fields(Vec<Shape>),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Availability {
    Present,
    Empty,
    Conditional,
}
#[derive(Clone, Debug, PartialEq, Eq)]
enum Node {
    Value(Availability),
    EmptyAggregate(Availability),
    Fields {
        constructed: Availability,
        fields: Vec<Node>,
    },
}
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Place {
    pub root: PlaceId,
    pub fields: Vec<usize>,
}
impl fmt::Display for Place {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(owned-place (owner {}) (local {}) (fields",
            self.root.owner, self.root.local
        )?;
        for field in &self.fields {
            write!(f, " {field}")?;
        }
        f.write_str("))")
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    DuplicateRoot,
    MissingPlace,
    ShapeMismatch,
    NotPresent,
    NotEmpty,
    ContradictoryGuard,
    Budget,
}
/// A state at one reachable program point. Unreachable branches must be removed
/// by control-flow analysis before joining. Roots have stable declaration IDs.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct State {
    roots: BTreeMap<PlaceId, Node>,
    /// Facts valid only when the antecedent's construction token is present.
    /// Consequents are descendants, never unrelated owners or program values.
    constructed: BTreeMap<Place, BTreeMap<Place, Availability>>,
}

/// A conditional node needs runtime/path evidence before cleanup. An Empty
/// aggregate node with live children was never fully constructed: only its
/// initialized children are candidates for cleanup, not the enclosing Drop.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CleanupTree {
    pub place: Place,
    pub availability: Availability,
    /// None = atomic value; Some = aggregate, in reverse field order.
    pub children: Option<Vec<CleanupTree>>,
}
impl fmt::Display for CleanupTree {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(cleanup-tree {} (availability {})",
            self.place,
            Node::Value(self.availability)
        )?;
        if let Some(children) = &self.children {
            f.write_str(" (children")?;
            for child in children {
                write!(f, " {child}")?;
            }
            f.write_str(")")?;
        }
        f.write_str(")")
    }
}

impl Node {
    fn token(&self) -> Availability {
        match self {
            Self::Value(token) | Self::EmptyAggregate(token) => *token,
            Self::Fields { constructed, .. } => *constructed,
        }
    }
    fn build(
        shape: &Shape,
        initial: Availability,
        depth: usize,
        fuel: &mut usize,
    ) -> Result<Self, Error> {
        if depth > 128 || *fuel == 0 {
            return Err(Error::Budget);
        }
        *fuel -= 1;
        Ok(match shape {
            // Empty aggregates still carry one ownership token.
            Shape::Value => Self::Value(initial),
            Shape::Fields(fields) if fields.is_empty() => Self::EmptyAggregate(initial),
            Shape::Fields(fields) => Self::Fields {
                constructed: initial,
                fields: fields
                    .iter()
                    .map(|s| Self::build(s, initial, depth + 1, fuel))
                    .collect::<Result<_, _>>()?,
            },
        })
    }
    fn all(&self, expected: Availability) -> bool {
        match self {
            Self::Value(s) | Self::EmptyAggregate(s) => *s == expected,
            Self::Fields {
                constructed,
                fields,
            } => *constructed == expected && fields.iter().all(|n| n.all(expected)),
        }
    }
    fn may_be_present(&self) -> bool {
        self.token() != Availability::Empty
            && match self {
                Self::Fields { fields, .. } => fields.iter().all(Self::may_be_present),
                _ => true,
            }
    }
    fn fill(&mut self, state: Availability) {
        match self {
            Self::Value(s) | Self::EmptyAggregate(s) => *s = state,
            Self::Fields {
                constructed,
                fields,
            } => {
                *constructed = state;
                for field in fields {
                    field.fill(state);
                }
            }
        }
    }
    fn merge(&self, other: &Self, fuel: &mut usize) -> Result<Self, Error> {
        if *fuel == 0 {
            return Err(Error::Budget);
        }
        *fuel -= 1;
        match (self, other) {
            (Self::Value(a), Self::Value(b)) => Ok(Self::Value(if a == b {
                *a
            } else {
                Availability::Conditional
            })),
            (Self::EmptyAggregate(a), Self::EmptyAggregate(b)) => {
                Ok(Self::EmptyAggregate(if a == b {
                    *a
                } else {
                    Availability::Conditional
                }))
            }
            (
                Self::Fields {
                    constructed: ca,
                    fields: a,
                },
                Self::Fields {
                    constructed: cb,
                    fields: b,
                },
            ) if a.len() == b.len() => Ok(Self::Fields {
                constructed: if ca == cb {
                    *ca
                } else {
                    Availability::Conditional
                },
                fields: a
                    .iter()
                    .zip(b)
                    .map(|(a, b)| a.merge(b, fuel))
                    .collect::<Result<_, _>>()?,
            }),
            _ => Err(Error::ShapeMismatch),
        }
    }
}
impl State {
    fn prune_construction_facts(&mut self) {
        let mut implications = std::mem::take(&mut self.constructed);
        implications.retain(|guard, facts| {
            if self.node(guard).map(Node::token) != Ok(Availability::Conditional) {
                return false;
            }
            facts.retain(|place, _| {
                self.node(place).map(Node::token) == Ok(Availability::Conditional)
            });
            !facts.is_empty()
        });
        self.constructed = implications;
    }
    fn invalidate_construction_facts(&mut self, changed: &Place) {
        self.constructed.retain(|guard, facts| {
            if guard.root == changed.root && guard.fields.starts_with(&changed.fields) {
                return false;
            }
            facts.retain(|place, _| {
                !(place.root == changed.root && place.fields.starts_with(&changed.fields))
            });
            !facts.is_empty()
        });
    }
    fn invalidate_reconstructed_ancestors(&mut self, changed: &Place) {
        // Installing a child may complete a parent on paths where its guard
        // was previously false. Old true-branch premises do not cover these
        // newly constructed paths, even if the joined token stays Conditional.
        // Newly completed parents have every field present, so positive facts
        // about untouched descendants remain true on these additional paths.
        self.constructed.retain(|guard, facts| {
            if guard.root == changed.root && changed.fields.starts_with(&guard.fields) {
                facts.retain(|_, token| *token == Availability::Present);
            }
            !facts.is_empty()
        });
    }
    fn tokens(&self, fuel: &mut usize) -> Result<Vec<(Place, Availability)>, Error> {
        fn visit(
            node: &Node,
            place: &mut Place,
            fuel: &mut usize,
            out: &mut Vec<(Place, Availability)>,
        ) -> Result<(), Error> {
            *fuel = fuel.checked_sub(1).ok_or(Error::Budget)?;
            out.push((place.clone(), node.token()));
            if let Node::Fields { fields, .. } = node {
                for (index, field) in fields.iter().enumerate() {
                    place.fields.push(index);
                    visit(field, place, fuel, out)?;
                    place.fields.pop();
                }
            }
            Ok(())
        }
        let mut tokens = vec![];
        for (root, node) in &self.roots {
            visit(
                node,
                &mut Place {
                    root: *root,
                    fields: vec![],
                },
                fuel,
                &mut tokens,
            )?;
        }
        Ok(tokens)
    }
    fn guarded_token(&self, guard: &Place, place: &Place) -> Result<Availability, Error> {
        let token = self.node(place)?.token();
        Ok(if token == Availability::Conditional {
            self.constructed
                .get(guard)
                .and_then(|facts| facts.get(place))
                .copied()
                .unwrap_or(token)
        } else {
            token
        })
    }
    fn remap_construction_facts(
        &self,
        source: &Place,
        destination: &Place,
        fuel: &mut usize,
    ) -> Result<BTreeMap<Place, BTreeMap<Place, Availability>>, Error> {
        let remap = |place: &Place| Place {
            root: destination.root,
            fields: destination
                .fields
                .iter()
                .chain(&place.fields[source.fields.len()..])
                .copied()
                .collect(),
        };
        let mut output = BTreeMap::new();
        for (guard, facts) in &self.constructed {
            *fuel = fuel.checked_sub(1).ok_or(Error::Budget)?;
            if guard.root != source.root || !guard.fields.starts_with(&source.fields) {
                continue;
            }
            let mut mapped = BTreeMap::new();
            for (place, token) in facts {
                *fuel = fuel
                    .checked_sub(
                        place
                            .fields
                            .len()
                            .saturating_add(destination.fields.len())
                            .saturating_add(1),
                    )
                    .ok_or(Error::Budget)?;
                mapped.insert(remap(place), *token);
            }
            output.insert(remap(guard), mapped);
        }
        Ok(output)
    }
    pub fn declare(
        &mut self,
        root: PlaceId,
        shape: &Shape,
        initialized: bool,
        fuel: &mut usize,
    ) -> Result<(), Error> {
        if self.roots.contains_key(&root) {
            return Err(Error::DuplicateRoot);
        }
        let node = Node::build(
            shape,
            if initialized {
                Availability::Present
            } else {
                Availability::Empty
            },
            0,
            fuel,
        )?;
        self.roots.insert(root, node);
        Ok(())
    }
    fn node(&self, place: &Place) -> Result<&Node, Error> {
        let mut node = self.roots.get(&place.root).ok_or(Error::MissingPlace)?;
        for index in &place.fields {
            let Node::Fields { fields, .. } = node else {
                return Err(Error::MissingPlace);
            };
            node = fields.get(*index).ok_or(Error::MissingPlace)?;
        }
        Ok(node)
    }
    fn node_mut(&mut self, place: &Place) -> Result<&mut Node, Error> {
        let mut node = self.roots.get_mut(&place.root).ok_or(Error::MissingPlace)?;
        for index in &place.fields {
            let Node::Fields { fields, .. } = node else {
                return Err(Error::MissingPlace);
            };
            node = fields.get_mut(*index).ok_or(Error::MissingPlace)?;
        }
        Ok(node)
    }
    fn finish_initialized_ancestors(
        &mut self,
        place: &Place,
        fuel: &mut usize,
    ) -> Result<(), Error> {
        fn completed_facts(
            node: &Node,
            place: &mut Place,
            facts: &mut BTreeMap<Place, Availability>,
            fuel: &mut usize,
        ) -> Result<(), Error> {
            *fuel = fuel
                .checked_sub(place.fields.len().saturating_add(1))
                .ok_or(Error::Budget)?;
            if node.token() == Availability::Conditional {
                facts.insert(place.clone(), Availability::Present);
            }
            if let Node::Fields { fields, .. } = node {
                for (index, field) in fields.iter().enumerate() {
                    place.fields.push(index);
                    completed_facts(field, place, facts, fuel)?;
                    place.fields.pop();
                }
            }
            Ok(())
        }
        fn visit(
            node: &mut Node,
            path: &[usize],
            place: &mut Place,
            completed: &mut BTreeMap<Place, BTreeMap<Place, Availability>>,
            fuel: &mut usize,
        ) -> Result<(), Error> {
            *fuel = fuel.checked_sub(1).ok_or(Error::Budget)?;
            if let (
                Node::Fields {
                    constructed,
                    fields,
                },
                Some((index, rest)),
            ) = (node, path.split_first())
            {
                place.fields.push(*index);
                visit(&mut fields[*index], rest, place, completed, fuel)?;
                place.fields.pop();
                if fields.iter().all(|field| field.all(Availability::Present)) {
                    *constructed = Availability::Present;
                } else if *constructed == Availability::Empty
                    && fields.iter().all(Node::may_be_present)
                {
                    // This write may complete construction on only some paths.
                    // Keeping Empty here would silently omit their parent Drop.
                    *constructed = Availability::Conditional;
                    // The parent was absent on every incoming path. It can
                    // acquire responsibility only where this write completes
                    // all of its fields, unlike an already constructed parent
                    // whose fields may subsequently have been moved away.
                    let mut facts = BTreeMap::new();
                    for (index, field) in fields.iter().enumerate() {
                        place.fields.push(index);
                        completed_facts(field, place, &mut facts, fuel)?;
                        place.fields.pop();
                    }
                    if !facts.is_empty() {
                        completed.insert(place.clone(), facts);
                    }
                }
            }
            Ok(())
        }
        let mut completed = BTreeMap::new();
        visit(
            self.roots.get_mut(&place.root).expect("validated root"),
            &place.fields,
            &mut Place {
                root: place.root,
                fields: vec![],
            },
            &mut completed,
            fuel,
        )?;
        self.constructed.extend(completed);
        self.prune_construction_facts();
        Ok(())
    }
    /// Snapshot the actual remaining hierarchy, including the enclosing
    /// object's construction token even when every field has been moved out.
    /// The root precedes children, which are in reverse field order. This is
    /// structural planning data, not permission to execute a user destructor.
    pub fn cleanup_tree(
        &self,
        place: &Place,
        fuel: &mut usize,
    ) -> Result<Option<CleanupTree>, Error> {
        fn visit(
            node: &Node,
            place: &mut Place,
            fuel: &mut usize,
        ) -> Result<Option<CleanupTree>, Error> {
            if *fuel == 0 {
                return Err(Error::Budget);
            }
            *fuel -= 1;
            let (availability, children) = match node {
                Node::Value(a) => (*a, None),
                Node::EmptyAggregate(a) => (*a, Some(vec![])),
                Node::Fields {
                    constructed,
                    fields,
                } => {
                    let mut children = vec![];
                    for (index, field) in fields.iter().enumerate().rev() {
                        place.fields.push(index);
                        let child = visit(field, place, fuel)?;
                        place.fields.pop();
                        if let Some(child) = child {
                            children.push(child);
                        }
                    }
                    (*constructed, Some(children))
                }
            };
            if availability == Availability::Empty
                && children.as_ref().is_none_or(|children| children.is_empty())
            {
                return Ok(None);
            }
            Ok(Some(CleanupTree {
                place: place.clone(),
                availability,
                children,
            }))
        }
        visit(self.node(place)?, &mut place.clone(), fuel)
    }
    /// Required before reading or copying a whole value. This does not prove
    /// Copy: that capability is supplied by trait elaboration, never by shape.
    pub fn require_present(&self, place: &Place) -> Result<(), Error> {
        if self.node(place)?.all(Availability::Present) {
            Ok(())
        } else {
            Err(Error::NotPresent)
        }
    }
    /// Establish a syntactic owned position without reading or transferring its
    /// contents. A wildcard can ignore an existing position even after a move.
    pub fn require_place(&self, place: &Place) -> Result<(), Error> {
        self.node(place).map(|_| ())
    }
    /// Transfer a whole subtree. Failure preserves the state, including siblings.
    pub fn take(&mut self, place: &Place) -> Result<(), Error> {
        self.require_present(place)?;
        self.invalidate_construction_facts(place);
        self.node_mut(place)?.fill(Availability::Empty);
        Ok(())
    }
    /// Install a successfully constructed value in an empty, unique position.
    /// Overwriting a live/conditional responsibility needs prior cleanup or a
    /// path split; this operation never silently discharges Drop obligations.
    pub fn install(&mut self, place: &Place) -> Result<(), Error> {
        let mut fuel = usize::MAX;
        self.install_inner(place, &mut fuel)
    }
    fn install_inner(&mut self, place: &Place, fuel: &mut usize) -> Result<(), Error> {
        if !self.node(place)?.all(Availability::Empty) {
            return Err(Error::NotEmpty);
        }
        self.invalidate_construction_facts(place);
        self.node_mut(place)?.fill(Availability::Present);
        self.invalidate_reconstructed_ancestors(place);
        self.finish_initialized_ancestors(place, fuel)?;
        Ok(())
    }
    /// Budgeted transactional form for program analysis. Only this owner's
    /// hierarchy and implications are staged; unrelated roots are not copied.
    pub fn install_with_budget(&mut self, place: &Place, fuel: &mut usize) -> Result<(), Error> {
        let mut staged =
            self.select_roots(&std::collections::BTreeSet::from([place.root]), fuel)?;
        staged.install_inner(place, fuel)?;
        self.commit_owner(place.root, staged);
        Ok(())
    }
    fn commit_owner(&mut self, root: PlaceId, mut staged: Self) {
        self.roots
            .insert(root, staged.roots.remove(&root).expect("staged owner"));
        self.constructed.retain(|guard, _| guard.root != root);
        self.constructed.extend(staged.constructed);
    }
    /// Record outstanding cleanup before installing a fully constructed value.
    /// Returns the legacy leaf view; call `cleanup_tree` before replacement to
    /// retain enclosing object obligations too. This does not discharge Drop.
    pub fn replace(&mut self, place: &Place) -> Result<Vec<(Place, Availability)>, Error> {
        let mut fuel = usize::MAX;
        self.replace_inner(place, &mut fuel)
    }
    fn replace_inner(
        &mut self,
        place: &Place,
        fuel: &mut usize,
    ) -> Result<Vec<(Place, Availability)>, Error> {
        self.node(place)?;
        let outstanding = self
            .remaining()
            .into_iter()
            .filter(|(p, _)| p.root == place.root && p.fields.starts_with(&place.fields))
            .collect();
        self.invalidate_construction_facts(place);
        self.node_mut(place)?.fill(Availability::Present);
        self.invalidate_reconstructed_ancestors(place);
        self.finish_initialized_ancestors(place, fuel)?;
        Ok(outstanding)
    }
    pub fn replace_with_budget(
        &mut self,
        place: &Place,
        fuel: &mut usize,
    ) -> Result<Vec<(Place, Availability)>, Error> {
        let mut staged =
            self.select_roots(&std::collections::BTreeSet::from([place.root]), fuel)?;
        let outstanding = staged.replace_inner(place, fuel)?;
        self.commit_owner(place.root, staged);
        Ok(outstanding)
    }
    pub fn join(&self, other: &Self, fuel: &mut usize) -> Result<Self, Error> {
        if !self.roots.keys().eq(other.roots.keys()) {
            return Err(Error::ShapeMismatch);
        }
        let roots = self
            .roots
            .iter()
            .map(|(root, node)| Ok((*root, node.merge(&other.roots[root], fuel)?)))
            .collect::<Result<_, Error>>()?;
        let mut merged = Self {
            roots,
            constructed: BTreeMap::new(),
        };
        let tokens = merged.tokens(fuel)?;
        for (guard_index, (guard, token)) in tokens.iter().enumerate() {
            if *token != Availability::Conditional {
                continue;
            }
            let left_guard = self.node(guard)?.token();
            let right_guard = other.node(guard)?.token();
            let mut facts = BTreeMap::new();
            // DFS token order keeps each subtree contiguous. Do not compare
            // every conditional owner with unrelated roots and siblings.
            for (place, token) in &tokens[guard_index + 1..] {
                if place.root != guard.root || !place.fields.starts_with(&guard.fields) {
                    break;
                }
                *fuel = fuel.checked_sub(1).ok_or(Error::Budget)?;
                if *token != Availability::Conditional {
                    continue;
                }
                let left = self.guarded_token(guard, place)?;
                let right = other.guarded_token(guard, place)?;
                let known = match (left_guard, right_guard) {
                    (Availability::Empty, _) => right,
                    (_, Availability::Empty) => left,
                    _ if left == right => left,
                    _ => Availability::Conditional,
                };
                if known != Availability::Conditional {
                    facts.insert(place.clone(), known);
                }
            }
            if !facts.is_empty() {
                merged.constructed.insert(guard.clone(), facts);
            }
        }
        Ok(merged)
    }
    /// Branch-local premise for a runtime construction-token test. This is not
    /// a whole-value presence test: moving a field leaves its parent's token
    /// live, and partially initializing fields need not construct their parent.
    /// Refine the tested token and implications proved by joins, never infer
    /// field presence from construction alone. Callers must retain the runtime
    /// guard around every operation justified by the returned state.
    pub fn under_construction_guard(
        &self,
        place: &Place,
        present: bool,
        fuel: &mut usize,
    ) -> Result<Self, Error> {
        let expected = if present {
            Availability::Present
        } else {
            Availability::Empty
        };
        let token = |node: &Node| match node {
            Node::Value(a) | Node::EmptyAggregate(a) => *a,
            Node::Fields { constructed, .. } => *constructed,
        };
        let actual = token(self.node(place)?);
        if actual != Availability::Conditional && actual != expected {
            return Err(Error::ContradictoryGuard);
        }
        // Copy through the bounded traversal; a failed refinement leaves the
        // original state unchanged and cannot bypass the analysis work limit.
        let mut branch = self.join(self, fuel)?;
        match branch.node_mut(place)? {
            Node::Value(a) | Node::EmptyAggregate(a) => *a = expected,
            Node::Fields { constructed, .. } => *constructed = expected,
        }
        if present {
            if let Some(facts) = self.constructed.get(place) {
                for (consequence, expected) in facts {
                    *fuel = fuel
                        .checked_sub(consequence.fields.len().saturating_add(1))
                        .ok_or(Error::Budget)?;
                    let actual = branch.node(consequence)?.token();
                    if actual != Availability::Conditional && actual != *expected {
                        return Err(Error::ContradictoryGuard);
                    }
                    match branch.node_mut(consequence)? {
                        Node::Value(a) | Node::EmptyAggregate(a) => *a = *expected,
                        Node::Fields { constructed, .. } => *constructed = *expected,
                    }
                }
            }
        }
        Ok(branch)
    }
    // Remap analysis facts across an invocation frame. These operations do not
    // copy runtime values or grant resource permission.
    pub(crate) fn import_position(
        &mut self,
        root: PlaceId,
        source: &Self,
        place: &Place,
        fuel: &mut usize,
    ) -> Result<(), Error> {
        if self.roots.contains_key(&root) {
            return Err(Error::DuplicateRoot);
        }
        let node = source.node(place)?;
        let replacement = node.merge(node, fuel)?;
        let facts = source.remap_construction_facts(
            place,
            &Place {
                root,
                fields: vec![],
            },
            fuel,
        )?;
        self.roots.insert(root, replacement);
        self.constructed.extend(facts);
        Ok(())
    }
    pub(crate) fn update_position(
        &mut self,
        place: &Place,
        source: &Self,
        root: PlaceId,
        fuel: &mut usize,
    ) -> Result<(), Error> {
        let node = source.roots.get(&root).ok_or(Error::MissingPlace)?;
        self.node(place)?.merge(node, fuel)?; // validate shape before mutation
        let replacement = node.merge(node, fuel)?;
        let facts = source.remap_construction_facts(
            &Place {
                root,
                fields: vec![],
            },
            place,
            fuel,
        )?;
        self.invalidate_construction_facts(place);
        *self.node_mut(place)? = replacement;
        self.constructed.extend(facts);
        Ok(())
    }
    /// Original aggregate arity includes positions whose ownership was moved.
    pub(crate) fn field_count(&self, place: &Place) -> Result<Option<usize>, Error> {
        Ok(match self.node(place)? {
            Node::Value(_) => None,
            Node::EmptyAggregate(_) => Some(0),
            Node::Fields { fields, .. } => Some(fields.len()),
        })
    }

    /// Expand a fully present opaque payload only after its layout has been
    /// selected by a runtime tag guard or explicit variant evidence.
    pub(crate) fn refine_present_payload(
        &mut self,
        place: &Place,
        shape: &Shape,
        fuel: &mut usize,
    ) -> Result<(), Error> {
        if !matches!(self.node(place)?, Node::Value(Availability::Present)) {
            return Err(Error::ShapeMismatch);
        }
        let replacement = Node::build(shape, Availability::Present, 0, fuel)?;
        self.invalidate_construction_facts(place);
        *self.node_mut(place)? = replacement;
        Ok(())
    }
    // Project from the state itself, not just a flow's lexical-root list:
    // borrowed/external positions must remain observable too. Construction
    // facts are local to their root and follow the same bounded remapping.
    pub(crate) fn excluding_roots(
        &self,
        excluded: &std::collections::BTreeSet<PlaceId>,
        fuel: &mut usize,
    ) -> Result<Self, Error> {
        let mut result = Self::default();
        for root in self.roots.keys() {
            *fuel = fuel.checked_sub(1).ok_or(Error::Budget)?;
            if !excluded.contains(root) {
                result.import_position(
                    *root,
                    self,
                    &Place {
                        root: *root,
                        fields: vec![],
                    },
                    fuel,
                )?;
            }
        }
        Ok(result)
    }
    pub(crate) fn select_roots(
        &self,
        roots: &std::collections::BTreeSet<PlaceId>,
        fuel: &mut usize,
    ) -> Result<Self, Error> {
        let mut result = Self::default();
        for root in roots {
            result.import_position(
                *root,
                self,
                &Place {
                    root: *root,
                    fields: vec![],
                },
                fuel,
            )?;
        }
        Ok(result)
    }
    /// Remove a lexical root and hand its remaining responsibilities to the
    /// cleanup planner. This returns the legacy leaf view; call `cleanup_tree`
    /// before retirement to retain enclosing objects too. Removing tracking is
    /// not evidence that Drop occurred.
    pub fn retire(&mut self, root: PlaceId) -> Result<Vec<(Place, Availability)>, Error> {
        if !self.roots.contains_key(&root) {
            return Err(Error::MissingPlace);
        }
        let remaining = self
            .remaining()
            .into_iter()
            .filter(|(p, _)| p.root == root)
            .collect();
        self.roots.remove(&root);
        self.constructed.retain(|guard, _| guard.root != root);
        Ok(remaining)
    }
    /// Remaining leaf responsibilities, in stable field order. This is not a
    /// Drop schedule: aggregate user destructors and dependency order are separate.
    pub fn remaining(&self) -> Vec<(Place, Availability)> {
        fn visit(node: &Node, place: &mut Place, output: &mut Vec<(Place, Availability)>) {
            match node {
                Node::Value(Availability::Empty) | Node::EmptyAggregate(Availability::Empty) => {}
                Node::Value(state) | Node::EmptyAggregate(state) => {
                    output.push((place.clone(), *state))
                }
                Node::Fields { fields, .. } => {
                    for (index, node) in fields.iter().enumerate() {
                        place.fields.push(index);
                        visit(node, place, output);
                        place.fields.pop();
                    }
                }
            }
        }
        let mut output = vec![];
        for (root, node) in &self.roots {
            visit(
                node,
                &mut Place {
                    root: *root,
                    fields: vec![],
                },
                &mut output,
            );
        }
        output
    }
}
impl fmt::Display for Node {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Value(state) => f.write_str(match state {
                Availability::Present => "present",
                Availability::Empty => "empty",
                Availability::Conditional => "conditional",
            }),
            Self::EmptyAggregate(state) => write!(f, "(empty-aggregate {})", Self::Value(*state)),
            Self::Fields {
                constructed,
                fields,
            } => {
                write!(f, "(fields (constructed {})", Self::Value(*constructed))?;
                for field in fields {
                    write!(f, " {field}")?;
                }
                f.write_str(")")
            }
        }
    }
}
impl fmt::Display for State {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("(ownership-state")?;
        for (root, node) in &self.roots {
            write!(
                f,
                " (root (owner {}) (local {}) {})",
                root.owner, root.local, node
            )?;
        }
        if !self.constructed.is_empty() {
            f.write_str(" (construction-implications")?;
            for (guard, facts) in &self.constructed {
                write!(f, " (if-constructed {guard} (facts")?;
                for (place, token) in facts {
                    write!(f, " (token {place} {})", Node::Value(*token))?;
                }
                f.write_str("))")?;
            }
            f.write_str(")")?;
        }
        f.write_str(")")
    }
}

#[cfg(test)]
mod construction_remapping_tests {
    use super::*;
    fn root(local: u32) -> Place {
        Place {
            root: PlaceId { owner: 17, local },
            fields: vec![],
        }
    }
    fn child(place: &Place, index: usize) -> Place {
        let mut place = place.clone();
        place.fields.push(index);
        place
    }
    #[test]
    fn guarded_facts_follow_import_writeback_and_retirement_transactionally() {
        let source = root(0);
        let shape = Shape::Fields(vec![Shape::Value]);
        let mut live = State::default();
        live.declare(source.root, &shape, true, &mut 100).unwrap();
        let mut moved = live.clone();
        moved.take(&source).unwrap();
        let joined = live.join(&moved, &mut 1000).unwrap();
        let receiver = root(1);
        let mut frame = State::default();
        frame
            .declare(
                receiver.root,
                &Shape::Fields(vec![shape.clone(), Shape::Value]),
                true,
                &mut 100,
            )
            .unwrap();
        let captured = child(&receiver, 0);
        let before = frame.clone();
        assert_eq!(
            frame.update_position(&captured, &joined, source.root, &mut 0),
            Err(Error::Budget)
        );
        assert_eq!(frame, before);
        frame
            .update_position(&captured, &joined, source.root, &mut 1000)
            .unwrap();
        let guarded = frame
            .under_construction_guard(&captured, true, &mut 1000)
            .unwrap();
        guarded.require_present(&captured).unwrap();
        guarded.require_present(&child(&receiver, 1)).unwrap();
        assert_eq!(frame.require_present(&captured), Err(Error::NotPresent));
        let imported = root(2);
        let mut invocation = State::default();
        assert_eq!(
            invocation.import_position(imported.root, &frame, &captured, &mut 0),
            Err(Error::Budget)
        );
        assert_eq!(invocation, State::default());
        invocation
            .import_position(imported.root, &frame, &captured, &mut 1000)
            .unwrap();
        invocation
            .under_construction_guard(&imported, true, &mut 1000)
            .unwrap()
            .require_present(&imported)
            .unwrap();
        let snapshot = invocation.clone();
        assert_eq!(
            invocation.import_position(imported.root, &frame, &captured, &mut 1000),
            Err(Error::DuplicateRoot)
        );
        assert_eq!(invocation, snapshot);
        assert!(
            invocation
                .to_string()
                .contains("(construction-implications")
        );
        println!("CONSTRUCTION_FACTS_DUMP={invocation}");
        invocation.retire(imported.root).unwrap();
        assert_eq!(invocation, State::default());
    }
}
