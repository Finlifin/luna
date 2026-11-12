//! Scalar stores preserve observations only with a current, private address
//! identity and structurally disjoint pointee fields. Different pointer slots
//! do not establish disjoint pointees; arbitrary effects retain full havoc.
use super::*;

fn disjoint_fields(
    left: &FieldPath,
    right: &FieldPath,
    limits: &mut Limits,
) -> Result<bool, Unknown> {
    if left.root != right.root {
        return Ok(false);
    }
    for ((left_kind, left_index), (right_kind, right_index)) in
        left.fields.iter().zip(&right.fields)
    {
        limits.step(1)?;
        if left_kind != right_kind
            || !matches!(
                left_kind,
                AggregateKind::Record(_) | AggregateKind::AnonymousRecord
            )
        {
            return Ok(false);
        }
        if left_index != right_index {
            return Ok(true);
        }
    }
    // Equal paths and ancestor/descendant paths may overlap.
    Ok(false)
}

impl Walker<'_> {
    pub(super) fn havoc_after_store(
        &mut self,
        pointer: &Expr,
        stored_type: &Term,
        captured: Option<&(FieldPath, ValueId)>,
        state: &mut State,
    ) -> Result<(), Reason> {
        let mut retained = BTreeMap::new();
        if matches!(stored_type, Term::Primitive(p)
            if *p == nir::Primitive::Bool || self.integer_layout(*p).is_some())
        {
            if let Some((address, version)) = captured {
                if state.facts.value(address.root) == Some(*version)
                    && self.private_projection_root(pointer, address.root, state)?
                {
                    for (read, (ty, observation)) in &state.reads {
                        self.limits.step(read.fields.len().saturating_add(1))?;
                        if matches!(ty, Term::Primitive(p)
                            if *p == nir::Primitive::Bool || self.integer_layout(*p).is_some())
                            && disjoint_fields(address, read, self.limits)?
                        {
                            retained.insert(read.clone(), (ty.clone(), *observation));
                        }
                    }
                }
            }
        }
        state.havoc_with_retained_reads(Some(&self.addressed_locals), retained, self.limits)?;
        Ok(())
    }

    fn private_projection_root(
        &mut self,
        mut pointer: &Expr,
        expected: PlaceId,
        state: &State,
    ) -> Result<bool, Reason> {
        loop {
            self.limits.step(1)?;
            match &pointer.kind {
                K::PointerField { pointer: base, .. }
                | K::PointerAnonymousField { pointer: base, .. } => pointer = base,
                K::Local(binding) if matches!(pointer.ty, Term::Pointer(_)) => {
                    let canonical = self.canonical_read_path(
                        FieldPath {
                            root: place(*binding),
                            fields: vec![],
                        },
                        state,
                    )?;
                    let Ok(owner) = u32::try_from(canonical.root.owner) else {
                        return Ok(false);
                    };
                    // A loaded pointer or an aggregate's pointer-valued field
                    // has a separate alias obligation. A private lexical
                    // pointer value cannot be replaced by this pointee store.
                    return Ok(canonical.root == expected
                        && canonical.fields.is_empty()
                        && !self.addressed_locals.contains(&LocalId {
                            owner,
                            index: canonical.root.local,
                        }));
                }
                _ => return Ok(false),
            }
        }
    }
}

impl State {
    pub(super) fn havoc_with_retained_reads(
        &mut self,
        addressed: Option<&BTreeSet<LocalId>>,
        retained: BTreeMap<FieldPath, (Term, PlaceId)>,
        limits: &mut Limits,
    ) -> Result<(), Unknown> {
        limits.step(self.reads.len().saturating_add(retained.len()))?;
        let mut protected: BTreeSet<_> = retained
            .iter()
            .flat_map(|(path, (_, observation))| [path.root, *observation])
            .collect();
        // By-value pointer aliases keep naming the same address even when its
        // contents change. Preserve only current, unexposed lexical aliases of
        // a retained base; loaded pointers and unrelated roots stay unknown.
        let bases: BTreeSet<_> = retained.keys().map(|path| path.root).collect();
        if !bases.is_empty() {
            for (alias, (original, source_version, alias_version)) in &self.read_aliases {
                limits.step(original.fields.len().saturating_add(1))?;
                if bases.contains(&original.root)
                    && self.facts.value(original.root) == Some(*source_version)
                    && self.facts.value(*alias) == Some(*alias_version)
                    && addressed.is_some_and(|addressed| {
                        u32::try_from(alias.owner).ok().is_some_and(|owner| {
                            !addressed.contains(&LocalId {
                                owner,
                                index: alias.local,
                            })
                        })
                    })
                {
                    protected.insert(*alias);
                }
            }
        }
        self.reads = retained;
        if !self.facts.is_reachable() {
            return Ok(());
        }
        self.invalidate_paths(None, &[], limits)?;
        limits.step(self.aggregates.len())?;
        let mut forgotten = Vec::new();
        for (place, value) in &mut self.aggregates {
            if self.frozen.contains_key(place) || protected.contains(place) {
                continue;
            }
            if !value.forget_observations(limits)? {
                forgotten.push(*place);
            }
        }
        for place in forgotten {
            self.aggregates.remove(&place);
        }
        for (place, domain) in &self.domains {
            if self.frozen.contains_key(place) || protected.contains(place) {
                continue;
            }
            limits.step(1)?;
            // Synthetic observations are not lexical locals. Preserve only
            // scalar by-value storage that has never had its address exposed.
            if (domain.is_some()
                || self
                    .aggregates
                    .get(place)
                    .is_some_and(|value| value.unbounded_integer))
                && addressed.is_some_and(|addressed| {
                    u32::try_from(place.owner).ok().is_some_and(|owner| {
                        !addressed.contains(&LocalId {
                            owner,
                            index: place.local,
                        })
                    })
                })
            {
                continue;
            }
            match domain {
                Some(bound) => {
                    self.facts.assign(*place, bound.copy(limits)?)?;
                }
                None => {
                    self.facts.invalidate(*place)?;
                }
            }
        }
        Ok(())
    }
}
