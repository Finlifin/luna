//! Owned aggregate component identities: tuple slots, record fields and tagged
//! variant payloads. These are program facts, never PTC terms or permissions.
use super::*;
pub(super) type Component = (AggregateKind, usize);
pub(super) fn record_path(fields: &[(nir::program::FieldOwner, usize)]) -> Vec<Component> {
    fields
        .iter()
        .map(|(id, index)| (field_owner_kind(*id), *index))
        .collect()
}

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct FieldPath {
    pub root: PlaceId,
    pub fields: Vec<Component>,
}
#[derive(Clone, PartialEq, Eq)]
pub(super) struct Projection {
    pub(super) root_version: ValueId,
    pub(super) path: FieldPath,
}
impl fmt::Display for FieldPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(field-path (root {} {}) (fields",
            self.root.owner, self.root.local
        )?;
        for (declaration, index) in &self.fields {
            write!(f, " ({declaration} {index})")?;
        }
        f.write_str("))")
    }
}
impl fmt::Display for Projection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "(projection {} {})", self.root_version, self.path)
    }
}
pub(super) fn copy_paths(
    paths: &BTreeMap<FieldPath, ValueId>,
    limits: &mut Limits,
) -> Result<BTreeMap<FieldPath, ValueId>, Unknown> {
    for path in paths.keys() {
        limits.step(path.fields.len().saturating_add(1))?;
    }
    Ok(paths.clone())
}
impl State {
    pub(super) fn invalidate_paths(
        &mut self,
        root: Option<PlaceId>,
        fields: &[Component],
        limits: &mut Limits,
    ) -> Result<(), Unknown> {
        let mut removed = Vec::new();
        for (path, id) in &self.paths {
            limits.step(path.fields.len().saturating_add(1))?;
            if root.is_none() && self.frozen.contains_key(&path.root) {
                continue;
            }
            if root.is_none_or(|root| root == path.root)
                && (path.fields.starts_with(fields) || fields.starts_with(&path.fields))
            {
                self.facts.invalidate(id.place)?;
                removed.push(path.clone());
            }
        }
        for path in removed {
            self.paths.remove(&path);
        }
        Ok(())
    }
    pub(super) fn join_paths_and_facts(
        &mut self,
        other: &Self,
        limits: &mut Limits,
    ) -> Result<(), Unknown> {
        self.charge_read_paths(limits)?;
        other.charge_read_paths(limits)?;
        if !self.facts.is_reachable() {
            self.reads = other.reads.clone();
            self.read_aliases = other.read_aliases.clone();
        } else if other.facts.is_reachable() {
            self.reads
                .retain(|address, observation| other.reads.get(address) == Some(observation));
            self.read_aliases
                .retain(|address, alias| other.read_aliases.get(address) == Some(alias));
        }
        let mut paths = BTreeMap::new();
        let mut left_mapping = BTreeMap::new();
        let mut right_mapping = BTreeMap::new();
        if !self.facts.is_reachable() {
            paths = copy_paths(&other.paths, limits)?;
        } else if !other.facts.is_reachable() {
            paths = copy_paths(&self.paths, limits)?;
        } else {
            for (path, id) in &self.paths {
                limits.step(path.fields.len().saturating_add(1))?;
                if let Some(other_id) = other.paths.get(path) {
                    let joined = if id == other_id {
                        *id
                    } else {
                        let joined = ValueId {
                            place: id.place,
                            version: self.facts.fresh_version()?,
                        };
                        left_mapping.insert(*id, joined);
                        right_mapping.insert(*other_id, joined);
                        joined
                    };
                    paths.insert(path.clone(), joined);
                }
            }
        }
        self.facts = if left_mapping.is_empty() {
            self.facts.join(&other.facts, limits)?
        } else {
            let left = self.facts.rename_values(&left_mapping, limits)?;
            let right = other.facts.rename_values(&right_mapping, limits)?;
            left.join(&right, limits)?
        };
        self.paths = paths;
        Ok(())
    }
}
impl Walker<'_> {
    pub(super) fn local_value(&mut self, state: &State, local: LocalId) -> Result<Value, Reason> {
        self.local_value_at(state, place(local))
    }
    pub(super) fn local_value_at(&mut self, state: &State, root: PlaceId) -> Result<Value, Reason> {
        if let Some(value) = state.aggregates.get(&root) {
            if let Some((frame, projection)) = &value.immutable_projection {
                if *frame == state.frame
                    && state.frozen.contains_key(&projection.path.root)
                    && state.facts.value(projection.path.root) == Some(projection.root_version)
                {
                    self.limits
                        .step(projection.path.fields.len().saturating_add(1))?;
                    let mut value = value.snapshot(self.limits, 0)?;
                    value.projection = Some(projection.clone());
                    return Ok(value);
                }
            }
        }
        let mut result = state
            .aggregates
            .get(&root)
            .map(|v| v.snapshot(self.limits, 0))
            .transpose()?
            .unwrap_or_default();
        if let Some(id) = state.facts.value(root) {
            result.projection = Some(Projection {
                root_version: id,
                path: FieldPath {
                    root,
                    fields: vec![],
                },
            });
            result.scalar = Some(Scalar::Local(id));
            match state.facts.bound(id, self.limits) {
                Ok(bound) => result.range = Some(bound.copy(self.limits)?),
                Err(Unknown::MissingBound(_)) => {}
                Err(e) => return Err(e.into()),
            }
            // Materialize refinements into copies and captures. The stored
            // aggregate itself remains unchanged until a real source write.
            let start = FieldPath {
                root,
                fields: vec![],
            };
            for (path, field_id) in state
                .paths
                .range(start..)
                .take_while(|(p, _)| p.root == root)
            {
                self.limits.step(path.fields.len().saturating_add(1))?;
                let bound = match state.facts.bound(*field_id, self.limits) {
                    Ok(bound) => Some(bound.copy(self.limits)?),
                    Err(Unknown::MissingBound(_)) => None,
                    Err(e) => return Err(e.into()),
                };
                if matches!(
                    path.fields.last(),
                    Some((
                        AggregateKind::EnumTag(_)
                            | AggregateKind::OptionalTag
                            | AggregateKind::ErrorTag,
                        _
                    ))
                ) {
                    continue;
                }
                // Alternative payload observations are meaningful only under
                // their tag. Keep their facts, but never reconstruct an unknown
                // enum snapshot as one arbitrarily chosen variant.
                let mut cursor = Some(&result);
                let mut compatible = true;
                for (kind, slot) in &path.fields {
                    if matches!(
                        kind,
                        AggregateKind::Variant(..)
                            | AggregateKind::Optional(_)
                            | AggregateKind::ErrorResult(_)
                    ) && cursor.and_then(|value| value.aggregate_kind) != Some(*kind)
                    {
                        compatible = false;
                        break;
                    }
                    cursor = cursor.and_then(|value| value.fields.as_ref()?.get(*slot));
                }
                if !compatible {
                    continue;
                }
                self.replace_field(
                    &mut result,
                    &path.fields,
                    Value {
                        unbounded_integer: bound.is_none(),
                        range: bound,
                        ..Value::default()
                    },
                    0,
                )?;
            }
        }
        Ok(result)
    }
    pub(super) fn project_component(
        &mut self,
        state: &mut State,
        result: &mut Value,
        projection: Option<Projection>,
        component: Component,
    ) -> Result<(), Reason> {
        if !state.facts.is_reachable() {
            return Ok(());
        }
        let Some(mut projection) = projection else {
            return Ok(());
        };
        // A base value evaluated before a write remains a snapshot; it cannot
        // install facts about the newly assigned root.
        if state.facts.value(projection.path.root) != Some(projection.root_version) {
            return Ok(());
        }
        self.limits
            .step(projection.path.fields.len().saturating_add(1))?;
        projection.path.fields.push(component);
        if result.range.is_some() || result.unbounded_integer {
            let id = match state.paths.get(&projection.path) {
                Some(id) => *id,
                None => {
                    let local = self.next_field;
                    self.next_field = local.checked_add(1).ok_or(Unknown::VersionExhausted)?;
                    let place = PlaceId {
                        owner: u64::MAX,
                        local,
                    };
                    let id = match &result.range {
                        Some(bound) => state.facts.assign(place, bound.copy(self.limits)?)?,
                        None => state.facts.invalidate(place)?,
                    };
                    state.paths.insert(projection.path.clone(), id);
                    id
                }
            };
            result.range = match state.facts.bound(id, self.limits) {
                Ok(bound) => Some(bound.copy(self.limits)?),
                Err(Unknown::MissingBound(_)) => None,
                Err(e) => return Err(e.into()),
            };
            result.scalar = Some(Scalar::Local(id));
        }
        result.projection = Some(projection);
        Ok(())
    }
    fn fixed_pointer_observation(&mut self, mut expr: &Expr) -> Result<bool, Reason> {
        for _ in 0..128 {
            self.limits.step(1)?;
            expr = match &expr.kind {
                K::Local(_) => return Ok(true),
                K::Field { value, .. } | K::AnonymousField { value, .. } => value,
                K::PointerField { pointer, .. }
                | K::PointerAnonymousField { pointer, .. }
                | K::ReadPointer(pointer) => pointer,
                _ => return Ok(false),
            };
        }
        Err(Reason::DepthLimit)
    }
    /// Register pure local/field reads before short-circuit forks so conditions
    /// on either operand name the same scalar identity in both alternatives.
    /// No calls, writes, checks or other source effects execute here.
    pub(super) fn prime_fields(
        &mut self,
        expr: &Expr,
        state: &mut State,
        depth: usize,
    ) -> Result<(), Reason> {
        self.limits.step(1)?;
        if depth >= 128 {
            return Err(Reason::DepthLimit);
        }
        if !state.facts.is_reachable() {
            return Ok(());
        }
        match &expr.kind {
            K::ComptimeValue(Term::Rigid {
                sort: nir::Sort::ComptimeValue,
                ..
            })
            | K::AssociatedValue { .. } => {
                // Immutable declaration inputs need the same symbolic identity
                // on both sides of a short-circuit join, just like local reads.
                self.child(expr, state, depth + 1)?;
            }
            K::ReadPointer(pointer) if self.fixed_pointer_observation(pointer)? => {
                // Establish an unconstrained memory observation identity before
                // short-circuit forks. Only fixed load chains reach this branch;
                // no pointer factories run and no read permission is granted.
                // Real writes still invalidate observations.
                // Otherwise a RHS-only ghost disappears at the join while the
                // resulting boolean condition still refers to it.
                self.child(expr, state, depth + 1)?;
            }
            K::AnonymousField { .. } | K::Field { .. } => {
                let mut base = expr;
                let mut count = depth;
                while let K::AnonymousField { value, .. } | K::Field { value, .. } = &base.kind {
                    self.limits.step(1)?;
                    count += 1;
                    if count >= 128 {
                        return Err(Reason::DepthLimit);
                    }
                    base = value;
                }
                if matches!(base.kind, K::Local(_)) {
                    self.child(expr, state, depth)?;
                }
            }
            K::And(a, b)
            | K::Or(a, b)
            | K::ScalarEqual(a, b)
            | K::ScalarLess(a, b)
            | K::ScalarLessEqual(a, b)
            | K::ScalarGreater(a, b)
            | K::ScalarGreaterEqual(a, b)
            | K::IntegerBinary {
                left: a, right: b, ..
            } => {
                self.prime_fields(a, state, depth + 1)?;
                self.prime_fields(b, state, depth + 1)?;
            }
            K::Not(v) | K::IntegerNegate(v) => self.prime_fields(v, state, depth + 1)?,
            K::Block { statements, result } | K::CallPreparation { statements, result } => {
                for statement in statements {
                    self.prime_fields(statement, state, depth + 1)?;
                }
                if let Some(result) = result {
                    self.prime_fields(result, state, depth + 1)?;
                }
            }
            K::If {
                condition,
                then_branch,
                else_branch,
            } => {
                self.prime_fields(condition, state, depth + 1)?;
                self.prime_fields(then_branch, state, depth + 1)?;
                if let Some(branch) = else_branch {
                    self.prime_fields(branch, state, depth + 1)?;
                }
            }
            K::Let { value, .. } | K::UnsafeScope(value) | K::Discard(value) => {
                self.prime_fields(value, state, depth + 1)?
            }
            _ => {}
        }
        Ok(())
    }
}
