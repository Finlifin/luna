//! Relational value transfer. Source facts are frozen at evaluation time, then
//! renamed to fresh destination identities. This is evidence about value copies,
//! not aliasing and not permission to read or mutate the original storage.
use super::super::exact_equal_values;
use super::*;
use std::collections::BTreeSet;

pub(super) struct PreparedCopy {
    source: std::sync::Arc<Facts>,
    scalar: Option<ValueId>,
    expression: Option<Polynomial>,
    range: Option<Interval>,
    fields: BTreeMap<Vec<Component>, ValueId>,
    children: Vec<(Component, std::sync::Arc<PreparedCopy>)>,
}
pub(super) struct InstalledCopy {
    source: std::sync::Arc<Facts>,
    mapping: BTreeMap<ValueId, ValueId>,
    expression: Option<(ValueId, Polynomial)>,
}

// A post-state memory value is observed in place, not copied from all caller
// relations. Keep only its identity correspondence for result export.
pub(super) fn observation_link(
    facts: &Facts,
    original: ValueId,
    observed: ValueId,
    limits: &mut Limits,
) -> Result<InstalledCopy, Unknown> {
    let mut source = facts.fork(limits)?;
    source.retain_places(&BTreeSet::from([original.place]), limits)?;
    Ok(InstalledCopy {
        source: std::sync::Arc::new(source),
        mapping: BTreeMap::from([(original, observed)]),
        expression: None,
    })
}
impl Walker<'_> {
    /// Publish relations between observed exit-memory cells even when the call
    /// has no scalar result. Only current, explicitly transported identities
    /// cross the frame boundary; unrelated callee locals remain private.
    pub(super) fn export_memory_observations(
        &mut self,
        state: &mut State,
        frame: &State,
        observations: &[InstalledCopy],
        entry_ghosts: &[InstalledCopy],
    ) -> Result<(), Reason> {
        let mut copy = InstalledCopy {
            source: std::sync::Arc::new(frame.facts.fork(self.limits)?),
            mapping: BTreeMap::new(),
            expression: None,
        };
        self.connect_call_observations(&mut copy, state, observations, entry_ghosts)?;
        if copy.mapping.is_empty() {
            return Ok(());
        }
        self.finish_copies(state, vec![copy])
    }

    fn connect_call_observations(
        &mut self,
        copy: &mut InstalledCopy,
        state: &State,
        ghosts: &[InstalledCopy],
        additional_ghosts: &[InstalledCopy],
    ) -> Result<(), Reason> {
        for ghost in ghosts.iter().chain(additional_ghosts) {
            self.limits.step(1)?;
            if !ghost.source.shares_lineage(&state.facts) {
                continue;
            }
            for (original, temporary) in &ghost.mapping {
                self.limits.step(1)?;
                if state.facts.value(original.place) == Some(*original)
                    && copy.source.value(temporary.place) == Some(*temporary)
                {
                    copy.mapping.insert(*temporary, *original);
                }
            }
        }
        // Connect callee snapshots to their equal caller ghosts before
        // renaming polynomial rows. Difference projection alone loses
        // quadratic and cubic result/input relationships.
        let equal = exact_equal_values(&copy.source, self.limits)?;
        let mut frozen = BTreeSet::new();
        for root in state.frozen.keys() {
            self.limits.step(1)?;
            if let Some(id) = state.facts.value(*root) {
                frozen.insert(id);
            }
        }
        for (path, id) in &state.paths {
            self.limits.step(1)?;
            if state.frozen.contains_key(&path.root) {
                frozen.insert(*id);
            }
        }
        // An unmapped callee snapshot may equal both a mutable argument and
        // an immutable caller entry ghost. Prefer the ghost before filling
        // remaining identities; nonlinear rows must retain the caller's old
        // inputs even when its current parameter later changes. Never replace
        // an explicit result/input mapping or infer equality from one bound.
        for frozen_only in [true, false] {
            loop {
                let mut changed = false;
                for (a, b) in &equal {
                    self.limits.step(1)?;
                    let candidate =
                        match (copy.mapping.get(a).copied(), copy.mapping.get(b).copied()) {
                            (Some(mapped), None) => Some((*b, mapped)),
                            (None, Some(mapped)) => Some((*a, mapped)),
                            _ => None,
                        };
                    if let Some((source, destination)) = candidate {
                        if !frozen_only || frozen.contains(&destination) {
                            copy.mapping.insert(source, destination);
                            changed = true;
                        }
                    }
                }
                if !changed {
                    break;
                }
            }
        }
        Ok(())
    }

    // Prefer an equal immutable entry value in stored polynomial expressions.
    // Both directed inequalities are required; one-sided bounds never license
    // substitution. This also preserves degree-two/three expressions on writes.
    fn freeze_expression(
        &mut self,
        polynomial: &Polynomial,
        state: &State,
    ) -> Result<Polynomial, Reason> {
        let mut frozen = BTreeSet::new();
        for root in state.frozen.keys() {
            self.limits.step(1)?;
            if let Some(id) = state.facts.value(*root) {
                frozen.insert(id);
            }
        }
        for (path, id) in &state.paths {
            self.limits.step(1)?;
            if state.frozen.contains_key(&path.root) {
                frozen.insert(*id);
            }
        }
        if frozen.is_empty() {
            return Ok(polynomial.copy(self.limits)?);
        }
        let mut aliases = BTreeMap::new();
        for (a, b) in exact_equal_values(&state.facts, self.limits)? {
            if frozen.contains(&b) && !frozen.contains(&a) {
                aliases.insert(a, b);
            } else if frozen.contains(&a) && !frozen.contains(&b) {
                aliases.insert(b, a);
            }
        }
        Ok(
            rename(polynomial, &aliases, &state.facts, true, self.limits)?
                .ok_or(Reason::InvalidSignature)?,
        )
    }
    pub(super) fn prepare_copy(
        &mut self,
        value: &Value,
        state: &State,
    ) -> Result<Option<std::sync::Arc<PreparedCopy>>, Reason> {
        self.prepare_copy_with_snapshot(value, state, &mut None)
    }

    // Components observed during one preparation see exactly the same state.
    // Freeze it lazily once; preexisting component evidence keeps its own older
    // snapshot. Separate prepare_copy calls never share a mutable observation.
    fn prepare_copy_with_snapshot(
        &mut self,
        value: &Value,
        state: &State,
        snapshot: &mut Option<std::sync::Arc<Facts>>,
    ) -> Result<Option<std::sync::Arc<PreparedCopy>>, Reason> {
        if let Some(evidence) = &value.evidence {
            return Ok(Some(evidence.clone()));
        }
        let scalar = match value.scalar {
            Some(Scalar::Local(id))
                if (value.range.is_some() || value.unbounded_integer)
                    && state.facts.value(id.place) == Some(id) =>
            {
                Some(id)
            }
            _ => None,
        };
        let mut fields = BTreeMap::new();
        if let Some(projection) = &value.projection {
            if state.facts.value(projection.path.root) == Some(projection.root_version) {
                for (path, id) in state
                    .paths
                    .range(projection.path.clone()..)
                    .take_while(|(p, _)| p.root == projection.path.root)
                {
                    self.limits.step(path.fields.len().saturating_add(1))?;
                    if path.fields.len() > projection.path.fields.len()
                        && path.fields.starts_with(&projection.path.fields)
                    {
                        fields.insert(path.fields[projection.path.fields.len()..].to_vec(), *id);
                    }
                }
            }
        }
        let expression = match &value.polynomial {
            Some(p) if p.terms().keys().any(|f| !f.is_empty()) => {
                if state.facts.relation_is_current(p, self.limits)? {
                    Some(self.freeze_expression(p, state)?)
                } else {
                    None
                }
            }
            _ => None,
        };
        let mut children = Vec::new();
        // A live projection already supplies the complete destination paths.
        // Otherwise preserve each constructor component's frozen evidence.
        if value.projection.is_none() {
            if let (Some(kind), Some(values)) = (value.aggregate_kind, &value.fields) {
                for (index, child) in values.iter().enumerate() {
                    self.limits.step(1)?;
                    if let Some(evidence) =
                        self.prepare_copy_with_snapshot(child, state, snapshot)?
                    {
                        children.push(((kind, index), evidence));
                    }
                }
            }
        }
        if scalar.is_none() && fields.is_empty() && expression.is_none() && children.is_empty() {
            return Ok(None);
        }
        if snapshot.is_none() {
            *snapshot = Some(std::sync::Arc::new(state.facts.fork(self.limits)?));
        }
        self.limits.step(1)?;
        Ok(Some(std::sync::Arc::new(PreparedCopy {
            source: snapshot.as_ref().unwrap().clone(),
            scalar,
            expression,
            range: value
                .range
                .as_ref()
                .map(|r| r.copy(self.limits))
                .transpose()?,
            fields,
            children,
        })))
    }
    pub(super) fn install_copy(
        &mut self,
        state: &mut State,
        binding: LocalId,
        copy: std::sync::Arc<PreparedCopy>,
    ) -> Result<Vec<InstalledCopy>, Reason> {
        self.install_copy_at(state, place(binding), &[], copy)
    }
    pub(super) fn install_copy_at(
        &mut self,
        state: &mut State,
        root: PlaceId,
        prefix: &[Component],
        copy: std::sync::Arc<PreparedCopy>,
    ) -> Result<Vec<InstalledCopy>, Reason> {
        let destination = if prefix.is_empty() {
            state.facts.value(root)
        } else if let Some(range) = &copy.range {
            let local = self.next_field;
            self.next_field = local.checked_add(1).ok_or(Unknown::VersionExhausted)?;
            let id = state.facts.assign(
                PlaceId {
                    owner: u64::MAX,
                    local,
                },
                range.copy(self.limits)?,
            )?;
            state.paths.insert(
                FieldPath {
                    root,
                    fields: prefix.to_vec(),
                },
                id,
            );
            Some(id)
        } else if copy.scalar.is_some() || copy.expression.is_some() {
            let local = self.next_field;
            self.next_field = local.checked_add(1).ok_or(Unknown::VersionExhausted)?;
            let id = state.facts.invalidate(PlaceId {
                owner: u64::MAX,
                local,
            })?;
            state.paths.insert(
                FieldPath {
                    root,
                    fields: prefix.to_vec(),
                },
                id,
            );
            Some(id)
        } else {
            None
        };
        let mut mapping = BTreeMap::new();
        if let Some(old) = copy.scalar {
            mapping.insert(old, destination.ok_or(Reason::InvalidSignature)?);
        }
        for (fields, old) in &copy.fields {
            self.limits.step(fields.len().saturating_add(1))?;
            let local = self.next_field;
            self.next_field = local.checked_add(1).ok_or(Unknown::VersionExhausted)?;
            let destination = PlaceId {
                owner: u64::MAX,
                local,
            };
            let id = match copy.source.bound(*old, self.limits) {
                Ok(bound) => state.facts.assign(destination, bound.copy(self.limits)?)?,
                Err(Unknown::MissingBound(_)) => state.facts.invalidate(destination)?,
                Err(e) => return Err(e.into()),
            };
            let mut destination_fields = prefix.to_vec();
            destination_fields.extend(fields);
            state.paths.insert(
                FieldPath {
                    root,
                    fields: destination_fields,
                },
                id,
            );
            mapping.insert(*old, id);
        }
        let expression = copy
            .expression
            .as_ref()
            .map(|p| -> Result<_, Reason> {
                Ok((
                    destination.ok_or(Reason::InvalidSignature)?,
                    p.copy(self.limits)?,
                ))
            })
            .transpose()?;
        let mut installed = Vec::new();
        if !mapping.is_empty() || expression.is_some() {
            self.limits.step(1)?;
            installed.push(InstalledCopy {
                source: copy.source.clone(),
                mapping,
                expression,
            });
        }
        for (component, child) in &copy.children {
            self.limits.step(prefix.len().saturating_add(1))?;
            let mut path = prefix.to_vec();
            path.push(*component);
            installed.extend(self.install_copy_at(state, root, &path, child.clone())?);
        }
        Ok(installed)
    }
    pub(super) fn bind_related_value(
        &mut self,
        state: &mut State,
        binding: LocalId,
        ty: &Term,
        value: Value,
    ) -> Result<(), Reason> {
        // Freeze a computed RHS before overwriting any local it reads. Its
        // relation to surviving entry snapshots can then be projected from the
        // old state, instead of confusing the input and output value versions.
        let value = if value.polynomial.as_ref().is_some_and(|p| {
            p.terms()
                .keys()
                .any(|ids| ids.iter().any(|id| id.place == place(binding)))
        }) {
            if let Some(root) = self.result_root(ty)? {
                self.bind_result(state, root, ty, value)?;
                self.local_value_at(state, root)?
            } else {
                value
            }
        } else {
            value
        };
        let transfer = self.prepare_copy(&value, state)?;
        bind_value(state, binding, ty, value, self.limits)?;
        if let Some(transfer) = transfer {
            let installed = self.install_copy(state, binding, transfer)?;
            self.finish_copies(state, installed)?;
        }
        Ok(())
    }
    pub(super) fn finish_copies(
        &mut self,
        state: &mut State,
        copies: Vec<InstalledCopy>,
    ) -> Result<(), Reason> {
        self.finish_copy_batch(state, &copies)
    }
    pub(super) fn finish_copy_batch(
        &mut self,
        state: &mut State,
        copies: &[InstalledCopy],
    ) -> Result<(), Reason> {
        // A shared frozen snapshot with the same complete substitution exports
        // the same evidence. Destination bounds only tighten during this batch;
        // individual expression/result equalities are still handled below.
        let mut imported: Vec<(std::sync::Arc<Facts>, BTreeMap<ValueId, ValueId>)> = Vec::new();
        for copy in copies {
            let mut mapping = BTreeMap::new();
            // Capture expressions evaluate in order. Only equal source lineage
            // AND value versions identify the same copied value across them.
            for other in copies {
                self.limits.step(1)?;
                if copy.source.shares_lineage(&other.source) {
                    for (old, new) in &other.mapping {
                        self.limits.step(1)?;
                        if let Some(previous) = mapping.insert(*old, *new) {
                            add_equality(
                                &mut state.facts,
                                Polynomial::variable(previous, self.limits)?,
                                Polynomial::variable(*new, self.limits)?,
                                self.limits,
                            )?;
                        }
                    }
                }
            }
            // Repeated inputs have several equal destinations. Transport this
            // copy's own conditions to its destination rather than letting the
            // last sibling win; unbounded equalities have no interval to spread.
            mapping.extend(copy.mapping.iter().map(|(old, new)| (*old, *new)));
            let same_lineage = copy.source.shares_lineage(&state.facts);
            let mut already_imported = false;
            for (source, previous) in &imported {
                self.limits.step(1)?;
                if std::sync::Arc::ptr_eq(source, &copy.source) {
                    self.limits.step(
                        previous
                            .len()
                            .saturating_add(mapping.len())
                            .saturating_add(1),
                    )?;
                    if previous == &mapping {
                        already_imported = true;
                        break;
                    }
                }
            }
            if !already_imported {
                let mut selected: BTreeSet<_> = mapping.keys().copied().collect();
                let mut all_source_values_current = same_lineage;
                if same_lineage {
                    for (place, (version, _)) in copy.source.current_entries() {
                        self.limits.step(1)?;
                        let id = ValueId {
                            place: *place,
                            version: *version,
                        };
                        if state.facts.value(*place) == Some(id) {
                            selected.insert(id);
                        } else {
                            all_source_values_current = false;
                        }
                    }
                }
                // With every source version still present in this lineage, rename
                // retains the entire relation graph. Copy equalities below connect
                // old and new identities, so eliminating intermediate variables
                // would only recompute redundant all-pairs difference guarantees.
                // Stale snapshots and cross-frame copies still need projection.
                let projected = if !all_source_values_current {
                    super::super::project_guarantees(&copy.source, &selected, self.limits)?
                } else {
                    Vec::new()
                };
                for row in copy.source.relation_rows().chain(&projected) {
                    if !copy.source.relation_is_current(row, self.limits)? {
                        continue;
                    }
                    // Unchanged facts already present in this frame need neither
                    // a polynomial clone nor a rename/import. Affected rows still
                    // get new identities: later writes may retire the originals.
                    if same_lineage {
                        let mut affected = false;
                        for factors in row.terms().keys() {
                            self.limits.step(
                                factors
                                    .len()
                                    .saturating_mul(mapping.len().max(1).ilog2() as usize + 1)
                                    .saturating_add(1),
                            )?;
                            if factors.iter().any(|id| mapping.contains_key(id)) {
                                affected = true;
                                break;
                            }
                        }
                        if !affected
                            && state.facts.relation_is_current(row, self.limits)?
                            && state.facts.contains_relation(row, self.limits)?
                        {
                            continue;
                        }
                    }
                    if let Some(row) =
                        rename(row, &mapping, &state.facts, same_lineage, self.limits)?
                    {
                        add_row(&mut state.facts, row, self.limits)?;
                    }
                }
                for (condition, truth) in copy.source.copy_conditions(self.limits)? {
                    if let Some(condition) =
                        condition.renamed(&mapping, &state.facts, same_lineage, self.limits, 0)?
                    {
                        // Copy evidence already establishes this premise, and the
                        // source bounds/relations were transported above. Remember
                        // its unresolved choices without replaying every retained
                        // disjunction after each individual field binding.
                        state
                            .facts
                            .remember_condition(&condition, truth, self.limits)?;
                    }
                }
                // A current source and its copy are equal, but their identities and
                // later writes remain distinct. Cross-invocation IDs never alias.
                if same_lineage {
                    for (old, new) in &mapping {
                        if state.facts.value(old.place) == Some(*old) {
                            add_equality(
                                &mut state.facts,
                                Polynomial::variable(*old, self.limits)?,
                                Polynomial::variable(*new, self.limits)?,
                                self.limits,
                            )?;
                        }
                    }
                }
                self.limits.step(mapping.len().saturating_add(1))?;
                imported.push((copy.source.clone(), mapping.clone()));
            }
            if let Some((destination, expression)) = &copy.expression {
                if let Some(expression) = rename(
                    expression,
                    &mapping,
                    &state.facts,
                    same_lineage,
                    self.limits,
                )? {
                    project_expression_bounds(
                        &mut state.facts,
                        *destination,
                        &expression,
                        self.limits,
                    )?;
                    add_equality(
                        &mut state.facts,
                        Polynomial::variable(*destination, self.limits)?,
                        expression,
                        self.limits,
                    )?;
                }
            }
        }
        state.facts.propagate_relations(self.limits)?;
        Ok(())
    }
}
// A computed value is equal to its frozen polynomial. Eliminate that
// expression from existing inequalities before later writes invalidate its
// operands. Only retain rewrites that reduce nonlinear terms, avoiding an
// unbounded algebraic closure while preserving useful degree-two/three bounds.
fn project_expression_bounds(
    facts: &mut Facts,
    destination: ValueId,
    expression: &Polynomial,
    limits: &mut Limits,
) -> Result<(), Unknown> {
    if !expression.terms().keys().any(|factors| factors.len() > 1) {
        return Ok(());
    }
    // Exact expression evidence does not require an interval. This optional
    // interval-row projection does: retain the equality below when unavailable.
    match expression.interval_enclosure(facts, limits) {
        Ok(_) => {}
        Err(Unknown::MissingBound(_)) => return Ok(()),
        Err(e) => return Err(e),
    }
    let variable = Polynomial::variable(destination, limits)?;
    for row in facts.copy_relations(limits)? {
        let nonlinear = row
            .terms()
            .keys()
            .filter(|factors| factors.len() > 1)
            .count();
        for subtract in [true, false] {
            limits.step(1)?;
            let shifted = if subtract {
                row.subtract(expression, limits)
            } else {
                row.add(expression, limits)
            };
            let shifted = match shifted {
                Ok(p) => p,
                Err(Unknown::TermLimit) => continue,
                Err(e) => return Err(e),
            };
            if shifted
                .terms()
                .keys()
                .filter(|factors| factors.len() > 1)
                .count()
                >= nonlinear
            {
                continue;
            }
            let projected = if subtract {
                shifted.add(&variable, limits)
            } else {
                shifted.subtract(&variable, limits)
            };
            match projected {
                Ok(row) => add_row(facts, row, limits)?,
                Err(Unknown::TermLimit) => (),
                Err(e) => return Err(e),
            }
        }
    }
    Ok(())
}

fn rename(
    p: &Polynomial,
    mapping: &BTreeMap<ValueId, ValueId>,
    target: &Facts,
    allow_current: bool,
    limits: &mut Limits,
) -> Result<Option<Polynomial>, Unknown> {
    let mut result = Polynomial::default();
    for (factors, c) in p.terms() {
        limits.step(factors.len().saturating_add(1))?;
        let mut renamed = Vec::new();
        for id in factors {
            let id = match mapping.get(id) {
                Some(new) => *new,
                None if allow_current && target.value(id.place) == Some(*id) => *id,
                _ => return Ok(None),
            };
            if target.value(id.place) != Some(id) {
                return Err(Unknown::StaleValue(id));
            }
            renamed.push(id);
        }
        renamed.sort_unstable();
        result.insert(renamed, c, limits)?;
    }
    Ok(Some(result))
}
fn add_equality(
    facts: &mut Facts,
    a: Polynomial,
    b: Polynomial,
    limits: &mut Limits,
) -> Result<(), Unknown> {
    // Unbounded values still have exact copy identity. Preserve that equality
    // as a symbolic condition until interval-based arithmetic becomes available.
    let bounded = match (
        a.interval_enclosure(facts, limits),
        b.interval_enclosure(facts, limits),
    ) {
        (Ok(_), Ok(_)) => true,
        (Err(Unknown::MissingBound(_)), Ok(_) | Err(Unknown::MissingBound(_)))
        | (Ok(_), Err(Unknown::MissingBound(_))) => false,
        (Err(e), _) | (_, Err(e)) => return Err(e),
    };
    if !bounded {
        return facts.assume(
            &Condition::PolynomialCompare {
                left: a,
                relation: C::Equal,
                right: b,
            },
            true,
            limits,
        );
    }
    add_row(facts, a.subtract(&b, limits)?, limits)?;
    add_row(facts, b.subtract(&a, limits)?, limits)
}
pub(super) fn add_row(
    facts: &mut Facts,
    row: Polynomial,
    limits: &mut Limits,
) -> Result<(), Unknown> {
    if row.terms().is_empty() {
        return Ok(());
    }
    facts.assume_relation(row, limits)?;
    Ok(())
}

impl Walker<'_> {
    /// Aggregate temporaries own their values too. Give their components fresh
    /// identities before a destructuring pattern or another constructor sees
    /// them, so the evidence does not depend on later source-local writes.
    pub(super) fn materialize_aggregate(
        &mut self,
        value: Value,
        ty: &Term,
        state: &mut State,
    ) -> Result<Value, Reason> {
        if value.fields.is_none() && value.nominal_type.is_none() {
            return Ok(value);
        }
        if value.projection.as_ref().is_some_and(|p| {
            let private_unchanged = u32::try_from(p.path.root.owner).ok().is_some_and(|owner| {
                let binding = LocalId {
                    owner,
                    index: p.path.root.local,
                };
                !self.addressed_locals.contains(&binding) && !self.written_locals.contains(&binding)
            });
            p.path.root.owner == u64::MAX - 1
                || ((state.frozen.contains_key(&p.path.root) || private_unchanged)
                    && state.facts.value(p.path.root) == Some(p.root_version))
        }) {
            return Ok(value);
        }
        if !state.facts.is_reachable() {
            return Ok(value);
        }
        let Some(copy) = self.prepare_copy(&value, state)? else {
            return Ok(value);
        };
        let local = self.next_field;
        self.next_field = local.checked_add(1).ok_or(Unknown::VersionExhausted)?;
        let root = PlaceId {
            owner: u64::MAX - 1,
            local,
        };
        bind_value_place(state, root, ty, value, self.limits)?;
        let installed = self.install_copy_at(state, root, &[], copy)?;
        self.finish_copies(state, installed)?;
        self.local_value_at(state, root)
    }
}

impl Walker<'_> {
    /// A control-flow result owns one value at the join. Each branch first
    /// binds its actual result to this root; component identities are then
    /// aligned by State::merge before intersecting relational guarantees.
    pub(super) fn result_root(&mut self, ty: &Term) -> Result<Option<PlaceId>, Reason> {
        let integer = matches!(ty, Term::Primitive(p) if self.integer_layout(*p).is_some() || matches!(p, nir::Primitive::Bool | nir::Primitive::Integer));
        if !integer
            && !matches!(
                ty,
                Term::Optional(_) | Term::Tuple(_) | Term::Record(_) | Term::Nominal { .. }
            )
            && self.tuple_field_types(ty)?.is_none()
        {
            return Ok(None);
        }
        self.limits.step(1)?;
        let local = self.next_field;
        self.next_field = local.checked_add(1).ok_or(Unknown::VersionExhausted)?;
        Ok(Some(PlaceId {
            owner: u64::MAX - 1,
            local,
        }))
    }
    // Rewrite a branch predicate onto surviving input observations before
    // pattern bindings leave scope. Equality evidence, never field spelling,
    // authorizes each substitution.
    pub(super) fn scoped_condition(
        &mut self,
        condition: &Condition,
        state: &State,
        scope: &BTreeSet<PlaceId>,
    ) -> Result<Condition, Reason> {
        let mut preferred = BTreeSet::new();
        for (path, id) in &state.paths {
            self.limits.step(1)?;
            if scope.contains(&path.root) {
                preferred.insert(*id);
            }
        }
        for root in scope {
            self.limits.step(1)?;
            preferred.extend(state.facts.value(*root));
        }
        let mut mapping = BTreeMap::new();
        for (a, b) in exact_equal_values(&state.facts, self.limits)? {
            if preferred.contains(&a) && !preferred.contains(&b) {
                mapping.insert(b, a);
            } else if preferred.contains(&b) && !preferred.contains(&a) {
                mapping.insert(a, b);
            }
        }
        condition
            .renamed(&mapping, &state.facts, true, self.limits, 0)?
            .ok_or(Reason::Unsupported)
    }
    pub(super) fn bind_result(
        &mut self,
        state: &mut State,
        root: PlaceId,
        ty: &Term,
        value: Value,
    ) -> Result<(), Reason> {
        if !state.facts.is_reachable() {
            return Ok(());
        }
        let transfer = self.prepare_copy(&value, state)?;
        bind_value_place(state, root, ty, value, self.limits)?;
        if let Some(transfer) = transfer {
            let installed = self.install_copy_at(state, root, &[], transfer)?;
            self.finish_copies(state, installed)?;
        }
        // Literal-only aggregate results need identities too: two different
        // pairs can share an ordering even when marginal intervals overlap.
        if let Some(value) = state.aggregates.get(&root) {
            let value = value.snapshot(self.limits, 0)?;
            self.track_result_components(state, root, &[], &value, 0)?;
        }
        Ok(())
    }
    pub(super) fn materialize_entry_fields(
        &mut self,
        state: &mut State,
        binding: LocalId,
    ) -> Result<(), Reason> {
        let root = place(binding);
        let mut value = self.local_value_at(state, root)?;
        self.expand_owned_shape(&mut value, 0)?;
        if value.fields.is_some() {
            self.track_result_components(state, root, &[], &value, 0)?;
            state.aggregates.insert(root, value);
        }
        Ok(())
    }
    pub(super) fn expand_owned_shape(
        &mut self,
        value: &mut Value,
        depth: usize,
    ) -> Result<(), Reason> {
        self.limits.step(1)?;
        if depth >= 128 {
            return Err(Reason::DepthLimit);
        }
        if value.fields.is_none() {
            if let Some(ty) = &value.nominal_type {
                if let Term::Nominal { declaration, .. } = ty.as_ref() {
                    if self
                        .package
                        .is_some_and(|p| p.structs.iter().any(|s| s.declaration == *declaration))
                    {
                        let fields = self.nominal_fields(ty, *declaration, None)?;
                        value.fields = Some(
                            fields
                                .iter()
                                .map(|ty| self.unknown_value(ty))
                                .collect::<Result<_, _>>()?,
                        );
                        value.aggregate_kind = Some(AggregateKind::Record(*declaration));
                    }
                }
            }
        }
        if let Some(fields) = &mut value.fields {
            for field in fields {
                self.expand_owned_shape(field, depth + 1)?;
            }
        }
        Ok(())
    }
    /// Copy only immutable entry roots into a private invocation frame. These
    /// ghosts preserve caller-old/input relations without retaining mutable locals.
    pub(super) fn copy_frozen_frame(
        &mut self,
        source: &State,
        destination: &mut State,
    ) -> Result<Vec<InstalledCopy>, Reason> {
        let mut copies = Vec::new();
        for (binding, ty) in &source.frozen {
            self.limits.step(1)?;
            let Some(root) = self.result_root(ty)? else {
                continue;
            };
            let value = self.local_value_at(source, *binding)?;
            let transfer = self.prepare_copy(&value, source)?;
            bind_value_place(destination, root, ty, value, self.limits)?;
            if let Some(transfer) = transfer {
                copies.extend(self.install_copy_at(destination, root, &[], transfer)?);
            }
            destination.frozen.insert(root, ty.clone());
        }
        self.finish_copy_batch(destination, &copies)?;
        Ok(copies)
    }
    pub(super) fn bind_call_result(
        &mut self,
        state: &mut State,
        root: PlaceId,
        ty: &Term,
        value: Value,
        frame: &State,
        ghosts: &[InstalledCopy],
        additional_ghosts: &[InstalledCopy],
    ) -> Result<(), Reason> {
        let transfer = self.prepare_copy(&value, frame)?;
        self.bind_result(state, root, ty, value)?;
        if let Some(transfer) = transfer {
            let mut copies = self.install_copy_at(state, root, &[], transfer)?;
            for copy in &mut copies {
                if !copy.source.shares_lineage(&frame.facts) {
                    continue;
                }
                self.connect_call_observations(copy, state, ghosts, additional_ghosts)?;
            }
            self.finish_copies(state, copies)?;
        }
        Ok(())
    }
    fn track_result_components(
        &mut self,
        state: &mut State,
        root: PlaceId,
        path: &[Component],
        value: &Value,
        depth: usize,
    ) -> Result<(), Reason> {
        self.limits.step(path.len().saturating_add(1))?;
        if depth >= 128 {
            return Err(Reason::DepthLimit);
        }
        if let Some(range) = &value.range {
            let key = FieldPath {
                root,
                fields: path.to_vec(),
            };
            if !state.paths.contains_key(&key) {
                let local = self.next_field;
                self.next_field = local.checked_add(1).ok_or(Unknown::VersionExhausted)?;
                let id = state.facts.assign(
                    PlaceId {
                        owner: u64::MAX,
                        local,
                    },
                    range.copy(self.limits)?,
                )?;
                state.paths.insert(key, id);
            }
        }
        if let (Some(kind), Some(fields)) = (value.aggregate_kind, &value.fields) {
            for (index, field) in fields.iter().enumerate() {
                let mut child = path.to_vec();
                child.push((kind, index));
                self.track_result_components(state, root, &child, field, depth + 1)?;
            }
        }
        Ok(())
    }
}
