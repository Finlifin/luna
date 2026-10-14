//! Import scalar post-state bounds without treating entry reads as exit reads.
use super::*;

impl Walker<'_> {
    // A complete scalar-write summary cannot overwrite a pointer cell. Unknown
    // calls and aggregate/pointer writes cannot establish this guarantee.
    pub(super) fn call_preserves_pointer_bindings(&mut self, callee: &Expr) -> Result<bool, Reason> {
        let Some(summary) = self.call_summary(callee) else { return Ok(false); };
        for access in &summary.accesses {
            self.limits.step(1)?;
            if access.write {
                let ty = self.scalar_type_view(&access.ty)?;
                if !matches!(ty, Term::Primitive(p) if p == nir::Primitive::Bool || p.fixed_integer_layout().is_some()) {
                    return Ok(false);
                }
            }
        }
        Ok(true)
    }

    pub(super) fn apply_memory_postconditions(
        &mut self,
        state: &mut State,
        prepared: &State,
        predicates: &[Expr],
        snapshots: &[nir::program::EntrySnapshot],
        bindings: &[LocalId],
        targets: &BTreeMap<usize, Option<Vec<nir::program::Place>>>,
        addresses: &BTreeMap<usize, FieldPath>,
        result_binding: Option<LocalId>,
        result_ty: &Term,
        input_ghosts: &[copies::InstalledCopy],
        depth: usize,
    ) -> Result<Option<Value>, Reason> {
        let mut after = prepared.fork(self.limits)?;
        // Prepared reads denote entry memory. Retire their value identities as
        // well as the lookup keys, so preconditions cannot constrain exit reads.
        for (_, root) in after.reads.values() {
            self.limits.step(1)?;
            after.facts.invalidate(*root)?;
        }
        after.forget_reads(self.limits)?;
        let mut reads = Vec::new();
        for predicate in predicates {
            iir::visit::expression(predicate, &mut self.limits.arithmetic.steps, |expr, _| {
                if let K::ReadPointer(pointer) = &expr.kind {reads.push((pointer.as_ref(), &expr.ty));}
                Ok(())
            }).map_err(|_| Reason::Unknown(comptime_runtime::Error::BudgetExhausted.into()))?;
        }
        let mut outputs = Vec::new();
        let mut transfers = Vec::new();
        let mut observed_cells = BTreeMap::new();
        for (pointer, ty) in reads {
            let Some(path) = self.external_read_path(pointer)? else {continue;};
            if after.reads.contains_key(&path) {continue;}
            self.limits.step(bindings.len())?;
            let Some(parameter) = bindings.iter().position(|binding| place(*binding) == path.root) else {continue;};
            let view = self.scalar_type_view(ty)?;
            if !matches!(view, Term::Primitive(p) if p == nir::Primitive::Bool || p.fixed_integer_layout().is_some()) {continue;}
            let current = if let Some(Some(actual)) = targets.get(&parameter) {
                // A selected destination does not constrain every may-alias target.
                let [target] = actual.as_slice() else {continue;};
                self.limits.step(target.fields.len().saturating_add(path.fields.len()).saturating_add(1))?;
                let mut target = target.clone();
                for (owner, index) in &path.fields {
                    let owner = match owner {
                        AggregateKind::Record(id) => nir::program::FieldOwner::Nominal(*id),
                        AggregateKind::AnonymousRecord => nir::program::FieldOwner::Anonymous,
                        _ => return Err(Reason::InvalidSignature),
                    };
                    target.fields.push((owner, *index));
                }
                self.read_pointer_place(state, &target)?
            } else if let Some(address) = addresses.get(&parameter) {
                self.limits.step(address.fields.len().saturating_add(path.fields.len()).saturating_add(1))?;
                let mut address = address.clone();
                address.fields.extend_from_slice(&path.fields);
                if let Some((observed_view, root)) = state.reads.get(&address) {
                    if *observed_view != view { continue; }
                    self.local_value_at(state, *root)?
                } else {
                    // This denotes exit memory, after all invalidation. It has
                    // no entry facts until the checked postconditions refine it.
                    let root = self.result_root(&view)?.ok_or(Reason::Unsupported)?;
                    let unknown = self.unknown_value(&view)?;
                    bind_value_place(state, root, &view, unknown, self.limits)?;
                    state.reads.insert(address, (view.clone(), root));
                    self.local_value_at(state, root)?
                }
            } else {continue;};
            let Some(Scalar::Local(original)) = current.scalar else {continue;};
            if let Some((observed_view, root)) = observed_cells.get(&original) {
                if *observed_view == view {
                    after.reads.insert(path, (view, *root));
                }
                continue;
            }
            let root = self.result_root(&view)?.ok_or(Reason::Unsupported)?;
            bind_value_place(&mut after, root, &view, current, self.limits)?;
            let observed = after.facts.value(root).ok_or(Reason::InvalidSignature)?;
            transfers.push(copies::observation_link(&state.facts, original, observed, self.limits)?);
            after.reads.insert(path, (view.clone(), root));
            observed_cells.insert(original, (view.clone(), root));
            outputs.push((root, original));
        }
        if outputs.is_empty() {return Ok(None);}
        let result_slot = if let Some(binding) = result_binding {
            self.result_root(result_ty)?.map(|root| (binding, root))
        } else { None };
        let result_binding = result_slot.map(|(binding, _)| binding);
        if let Some(binding) = result_binding {
            let result = self.unknown_value(result_ty)?;
            bind_value(&mut after, binding, result_ty, result, self.limits)?;
            self.materialize_entry_fields(&mut after, binding)?;
        }
        let mut applied = false;
        for predicate in predicates {
            if !exportable_postcondition(predicate, result_binding, snapshots, Some(&after.reads), self.limits)? {continue;}
            let old = std::mem::replace(&mut self.assuming_postcondition, true);
            let checked = self.child(predicate, &mut after, depth);
            self.assuming_postcondition = old;
            if let Some(condition) = checked?.condition {
                after.facts.assume(&condition, true, self.limits)?;
                applied = true;
            }
        }
        if !applied {return Ok(None);}
        if !after.facts.is_reachable() {
            state.facts.assume(&Condition::Bool(false), true, self.limits)?;
            return Ok(None);
        }
        for (root, original) in outputs {
            if state.facts.value(original.place) != Some(original) {continue;}
            if let Some(range) = self.local_value_at(&after, root)?.range {
                state.facts.refine(original, Some(&range.lower), Some(&range.upper), self.limits)?;
            }
        }
        self.export_memory_observations(state, &after, &transfers, input_ghosts)?;
        if let Some((binding, root)) = result_slot {
            let result = self.local_value(&after, binding)?;
            self.bind_call_result(state, root, result_ty, result, &after, &transfers, input_ghosts)?;
            return Ok(Some(self.local_value_at(state, root)?));
        }
        Ok(None)
    }
}
