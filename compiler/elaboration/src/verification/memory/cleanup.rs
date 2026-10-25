//! Map read-only cleanup requirements through the current owned value's pointers.
use super::*;

pub(super) fn charge_read(read: &Access, fuel: &mut usize) -> Result<(), String> {
    *fuel = fuel
        .checked_sub(read.target.path_cost())
        .ok_or("cleanup read copy budget exhausted")?;
    bound_type(&read.ty, fuel)
}
fn private_storage_roots(body: &Expr, fuel: &mut usize) -> Result<BTreeSet<LocalId>, String> {
    let mut candidates = BTreeSet::new();
    let mut exposed = BTreeSet::new();
    nir::visit::expression_pruned(body, fuel, |node, fuel| {
        let mut expose = |expr: &Expr, fuel: &mut usize| {
            nir::visit::expression_pruned(expr, fuel, |node, _| {
                if let K::Local(binding) = node.kind {
                    exposed.insert(binding);
                }
                Ok(true)
            })
        };
        match &node.kind {
            K::Let { binding, .. } => {
                candidates.insert(*binding);
            }
            K::LetPattern { pattern, .. } => {
                collect_pattern_storage(pattern, &mut candidates, &mut exposed, fuel)?;
            }
            K::AddressOf(value) => expose(value, fuel)?,
            K::Closure { captures, .. } => {
                for capture in captures {
                    *fuel = fuel
                        .checked_sub(1)
                        .ok_or("private pointer capture budget exhausted")?;
                    if let Some(value) = &capture.value {
                        expose(value, fuel)?;
                    }
                }
                for capture in captures {
                    exposed.insert(capture.binding);
                }
                // Each closure invocation proves its own private locals.
                return Ok(false);
            }
            _ => {}
        }
        Ok(true)
    })?;
    for binding in exposed {
        *fuel = fuel
            .checked_sub(1)
            .ok_or("private pointer exclusion budget exhausted")?;
        candidates.remove(&binding);
    }
    Ok(candidates)
}

fn collect_pattern_storage(
    pattern: &Pattern,
    owned: &mut BTreeSet<LocalId>,
    aliases: &mut BTreeSet<LocalId>,
    fuel: &mut usize,
) -> Result<(), String> {
    let mut pending = vec![pattern];
    let mut remaps = vec![];
    while let Some(pattern) = pending.pop() {
        *fuel = fuel
            .checked_sub(1)
            .ok_or("private pattern storage budget exhausted")?;
        match pattern {
            Pattern::Binding { binding, inner } => {
                owned.insert(*binding);
                pending.push(inner);
            }
            Pattern::Observe { binding, inner } => {
                // A candidate alias is not independently owned storage.
                aliases.insert(*binding);
                pending.push(inner);
            }
            Pattern::Remap { pattern, bindings } => {
                *fuel = fuel
                    .checked_sub(bindings.len())
                    .ok_or("private pattern remap budget exhausted")?;
                remaps.extend(bindings.iter().copied());
                pending.push(pattern);
            }
            Pattern::Then { first, next, .. } => {
                pending.push(first);
                pending.push(next);
            }
            Pattern::Guard { pattern, .. } | Pattern::ErrorResult { inner: pattern, .. } => {
                pending.push(pattern)
            }
            Pattern::Tuple(fields)
            | Pattern::TypeTuple(fields)
            | Pattern::Or(fields)
            | Pattern::Record { fields, .. }
            | Pattern::AnonymousRecord { fields, .. }
            | Pattern::Variant { fields, .. }
            | Pattern::NominalType { fields, .. }
            | Pattern::NormalForm {
                arguments: fields, ..
            } => {
                *fuel = fuel
                    .checked_sub(fields.len())
                    .ok_or("private pattern fields budget exhausted")?;
                pending.extend(fields);
            }
            Pattern::CodeEvidence { .. } => return Err("unresolved private pattern storage".into()),
            Pattern::Wild | Pattern::Unit | Pattern::Literal(_) => {}
        }
    }
    let mut successors = BTreeMap::<LocalId, Vec<LocalId>>::new();
    for (source, target) in remaps {
        *fuel = fuel
            .checked_sub(1)
            .ok_or("private pattern remap indexing budget exhausted")?;
        successors.entry(source).or_default().push(target);
    }
    let mut changed = vec![];
    for source in successors.keys() {
        *fuel = fuel
            .checked_sub(1)
            .ok_or("private pattern remap seed budget exhausted")?;
        if aliases.contains(source) {
            changed.push((*source, true));
        } else if owned.contains(source) {
            changed.push((*source, false));
        }
    }
    // Propagate each newly established fact once. Unknown cycles do not
    // create facts; alias facts dominate ownership at final root filtering.
    while let Some((source, alias)) = changed.pop() {
        *fuel = fuel
            .checked_sub(1)
            .ok_or("private pattern remap work budget exhausted")?;
        let Some(targets) = successors.get(&source) else {
            continue;
        };
        for target in targets {
            *fuel = fuel
                .checked_sub(1)
                .ok_or("private pattern remap closure budget exhausted")?;
            let inserted = if alias {
                aliases.insert(*target)
            } else {
                owned.insert(*target)
            };
            if inserted {
                changed.push((*target, alias));
            }
        }
    }
    Ok(())
}
impl Infer<'_, '_> {
    pub(super) fn prepare_private_storage_roots(&mut self, body: &Expr) -> Result<(), String> {
        if self.round.cleanup.is_some() {
            self.private_storage_roots = private_storage_roots(body, self.fuel)?;
        }
        Ok(())
    }
    pub(super) fn note_cleanup_pointer_mutation(&mut self, expr: &Expr) -> Result<(), String> {
        if self.round.cleanup.is_none() || self.round.mutates_pointer_storage {
            return Ok(());
        }
        if let K::Assign { place, .. } = &expr.kind {
            if self.private_storage_roots.contains(&place.root) {
                // No alias can observe this storage. Assignment updates targets
                // and then re-records cleanup reads for the whole owning root.
                return Ok(());
            }
        }
        // The write visitor either updates local provenance or rejects a
        // provenance-bearing store. Imported call footprints still mark writes
        // in record_access; they do not acquire this local transition proof.
        if matches!(expr.kind, K::WritePointer { .. }) {
            return Ok(());
        }
        if let K::Assign { value, .. } = &expr.kind {
            let ty = self.ty(&value.ty)?;
            self.round.mutates_pointer_storage = pointer_input_layout(self.package, &ty, self.fuel)
                .map_or(true, |leaves| !leaves.is_empty());
        }
        Ok(())
    }
    pub(super) fn write_local_value_slots(
        &mut self,
        targets: BTreeSet<PointerPlace>,
        code: BTreeMap<Vec<usize>, CallableResult>,
        local_code: LocalCodeFields,
        pointers: BTreeMap<Vec<usize>, Targets>,
        ty: Term,
        source: nir::program::SourceRange,
    ) -> Result<(), String> {
        self.target_work(&targets)?;
        if targets.is_empty() {
            return Err("code store has no established local target".into());
        }
        // A unique destination updates the current environment directly. Only
        // ambiguous destinations need snapshots and alternative joins.
        let before = if targets.len() > 1 {
            Some(self.save_environment()?)
        } else {
            None
        };
        let mut joined = None;
        for target in targets {
            let PointerRoot::Local { binding, storage } = target.root else {
                return Err("external code store requires a callable transition contract".into());
            };
            if self.stable_capture_roots.contains(&binding)
                || !target.through.is_empty()
                || target.conditions.values().any(|guards| !guards.is_empty())
            {
                return Err(
                    "conditional or captured code store requires a callable transition contract"
                        .into(),
                );
            }
            let (owner, root_ty) = self
                .local_storage
                .get(storage)
                .ok_or("local code store storage is missing")?;
            if *owner != binding || self.locals.get(&binding) != Some(root_ty) {
                return Err("local code store storage identity mismatch".into());
            }
            bound_type(root_ty, self.fuel)?;
            let root_ty = root_ty.clone();
            let mut traversal = StorageTraversal::new(self.package, &root_ty, &[], self.fuel)?;
            traversal.then_guarded(self.package, &target.fields, &[], self.fuel)?;
            if traversal.target.ty != ty {
                return Err("local code store type mismatch".into());
            }
            // Exactly one candidate is selected at runtime. Analyze each update
            // against the same post-RHS environment, then join alternatives.
            if let Some(before) = &before {
                self.environment = self.clone_environment(before)?;
            }
            let place = CallablePlace {
                binding,
                fields: target.fields.iter().map(|(_, index)| *index).collect(),
            };
            charge_callee_fields(&code, self.fuel)?;
            self.replace_callees(place.clone(), code.clone())?;
            self.charge_local_fields(&local_code)?;
            self.replace_local_fields(place.clone(), local_code.clone())?;
            for (path, targets) in &pointers {
                self.step(path.len())?;
                if let Some(targets) = targets {
                    self.target_work(targets)?;
                }
            }
            self.replace_pointer_fields(place, pointers.clone())?;
            self.record_value_cleanup_reads(&Expr {
                source,
                ty: root_ty,
                kind: K::Local(binding),
            })?;
            if before.is_none() {
                return Ok(());
            }
            let branch = self.save_environment()?;
            joined = Some(if let Some(previous) = joined {
                self.join_environment(previous, branch)?
            } else {
                branch
            });
        }
        self.environment = joined.ok_or("local code store has no continuation")?;
        Ok(())
    }
    pub(super) fn write_local_pointer_slot(
        &mut self,
        targets: BTreeSet<PointerPlace>,
        values: Targets,
        ty: Term,
        source: nir::program::SourceRange,
    ) -> Result<(), String> {
        let targets = self.resolve_local_loads(targets)?;
        self.target_work(&targets)?;
        if targets.is_empty() {
            return Err("pointer store has no established local target".into());
        }
        let exact = targets.len() == 1;
        let values = values.ok_or("local pointer store has unknown value provenance")?;
        for target in targets {
            let PointerRoot::Local { binding, storage } = target.root else {
                return Err("pointer store or owned write requires a richer contract".into());
            };
            if self.stable_capture_roots.contains(&binding) {
                return Err(
                    "captured pointer store requires an environment transition contract".into(),
                );
            }
            if !target.through.is_empty()
                || target.conditions.values().any(|guards| !guards.is_empty())
            {
                return Err(
                    "conditional pointer store requires a storage transition contract".into(),
                );
            }
            let (owner, root_ty) = self
                .local_storage
                .get(storage)
                .ok_or("local pointer store storage is missing")?;
            if *owner != binding || self.locals.get(&binding) != Some(root_ty) {
                return Err("local pointer store storage identity mismatch".into());
            }
            bound_type(root_ty, self.fuel)?;
            let root_ty = root_ty.clone();
            let mut traversal = StorageTraversal::new(self.package, &root_ty, &[], self.fuel)?;
            traversal.then_guarded(self.package, &target.fields, &[], self.fuel)?;
            if traversal.target.ty != ty || !self.local_pointer_value(&ty)? {
                return Err("local pointer store type mismatch".into());
            }
            self.target_work(&values)?;
            let mut updated = values.clone();
            let fields: Vec<_> = target.fields.iter().map(|(_, index)| *index).collect();
            if !exact {
                let previous = if fields.is_empty() {
                    self.environment.pointers.get(&binding)
                } else {
                    self.environment.field_pointers.get(&CallablePlace {
                        binding,
                        fields: fields.clone(),
                    })
                }
                .and_then(Option::as_ref)
                .ok_or("ambiguous local pointer store has unknown previous provenance")?;
                // This slot may be unselected. Retain its old targets as well as
                // the replacement; only a singleton destination permits killing them.
                for old in previous {
                    *self.fuel = self
                        .fuel
                        .checked_sub(old.target_cost())
                        .ok_or("local pointer weak update budget exhausted")?;
                    updated.insert(old.clone());
                }
            }
            if fields.is_empty() {
                self.set_pointer(binding, Some(updated))?;
            } else {
                self.replace_pointer_fields(
                    CallablePlace { binding, fields },
                    BTreeMap::from([(vec![], Some(updated))]),
                )?;
            }
            self.record_value_cleanup_reads(&Expr {
                source,
                ty: root_ty,
                kind: K::Local(binding),
            })?;
        }
        Ok(())
    }
    pub(super) fn record_parameter_cleanup_reads(
        &mut self,
        bindings: &[LocalId],
        parameters: &[nir::Parameter],
        source: nir::program::SourceRange,
    ) -> Result<(), String> {
        if self.round.cleanup_reads.is_empty() {
            return Ok(());
        }
        for (binding, parameter) in bindings.iter().zip(parameters) {
            self.step(0)?;
            bound_type(&parameter.ty, self.fuel)?;
            self.record_value_cleanup_reads(&Expr {
                source,
                ty: parameter.ty.clone(),
                kind: K::Local(*binding),
            })?;
        }
        Ok(())
    }
    pub(super) fn record_value_cleanup_reads(&mut self, expr: &Expr) -> Result<(), String> {
        if self.round.cleanup_reads.is_empty() {
            return Ok(());
        }
        let ty = self.ty(&expr.ty)?;
        bound_type(&ty, self.fuel)?;
        let Some(reads) = self.round.cleanup_reads.get(&ty) else {
            return Ok(());
        };
        for read in reads {
            charge_read(read, self.fuel)?;
        }
        let reads = reads.clone();
        let fields = if pointer_representation(self.package, &ty, self.fuel)?.is_some() {
            BTreeMap::from([(vec![], self.optional_target(expr, 0)?)])
        } else {
            self.pointer_fields_for(expr)?
        };
        let layout = pointer_input_layout(self.package, &ty, self.fuel)?;
        for read in reads {
            let Some(slot) = read.target.through.first() else {
                continue;
            };
            self.round.used_external_cleanup = true;
            let guards = read
                .target
                .conditions
                .get(&0)
                .map(Vec::as_slice)
                .unwrap_or(&[]);
            let mut selected = None;
            for leaf in &layout {
                self.step(0)?;
                if leaf
                    .storage_fields
                    .iter()
                    .copied()
                    .eq(slot.iter().map(|(_, index)| *index))
                    && leaf.guards == guards
                {
                    selected = Some(leaf);
                    break;
                }
            }
            let Some(leaf) = selected else {
                return Err("cleanup pointer slot lacks an owned input path".into());
            };
            let Some(Some(targets)) = fields.get(&leaf.input_fields) else {
                self.may_run_cleanup = true;
                continue;
            };
            let suffix = ParameterPlace {
                parameter: 0,
                input_fields: vec![],
                through: read.target.through[1..].to_vec(),
                fields: read.target.fields,
                conditions: read
                    .target
                    .conditions
                    .into_iter()
                    .filter_map(|(stage, guards)| stage.checked_sub(1).map(|stage| (stage, guards)))
                    .collect(),
            };
            self.target_work(targets)?;
            let mut rebased = BTreeSet::new();
            for target in targets {
                *self.fuel = self
                    .fuel
                    .checked_sub(suffix.path_cost())
                    .ok_or("cleanup pointer rebase budget exhausted")?;
                let mut target = target.clone();
                target.append_resolved_path(&suffix)?;
                rebased.insert(target);
            }
            self.record_access(rebased, false, read.read_value, read.ty)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod private_storage_tests {
    use super::*;
    use nir::program::{Capture, SourceRange};
    fn id(index: u32) -> LocalId {
        LocalId { owner: 17, index }
    }
    fn pointer() -> Term {
        Term::Pointer(Box::new(Term::Primitive(Primitive::Bool)))
    }
    fn node(kind: K) -> Expr {
        Expr {
            source: SourceRange { start: 0, end: 1 },
            ty: pointer(),
            kind,
        }
    }
    fn binding(index: u32) -> Expr {
        node(K::Let {
            binding: id(index),
            value: Box::new(node(K::Local(id(0)))),
        })
    }
    fn block(statements: Vec<Expr>) -> Expr {
        node(K::Block {
            statements,
            result: None,
        })
    }
    #[test]
    fn whole_body_exposure_excludes_early_and_late_addresses() {
        let address = node(K::AddressOf(Box::new(node(K::Local(id(1))))));
        for statements in [
            vec![address.clone(), binding(1), binding(2)],
            vec![binding(1), binding(2), address],
        ] {
            assert_eq!(
                private_storage_roots(&block(statements), &mut 1000).unwrap(),
                BTreeSet::from([id(2)])
            );
        }
    }
    #[test]
    fn captures_are_excluded_and_closure_locals_are_not_imported() {
        for value in [None, Some(Box::new(node(K::Local(id(1)))))] {
            let closure = node(K::Closure {
                parameters: vec![],
                captures: vec![Capture {
                    binding: if value.is_some() { id(4) } else { id(1) },
                    ty: pointer(),
                    value,
                }],
                body: std::sync::Arc::new(block(vec![binding(3)])),
            });
            assert_eq!(
                private_storage_roots(&block(vec![binding(1), binding(2), closure]), &mut 1000)
                    .unwrap(),
                BTreeSet::from([id(2)])
            );
        }
    }
    #[test]
    fn pattern_remaps_preserve_ownership_and_observation_aliases() {
        let pattern = Pattern::Remap {
            pattern: Box::new(Pattern::Tuple(vec![
                Pattern::Binding {
                    binding: id(1),
                    inner: Box::new(Pattern::Wild),
                },
                Pattern::Observe {
                    binding: id(2),
                    inner: Box::new(Pattern::Wild),
                },
            ])),
            bindings: vec![(id(3), id(5)), (id(1), id(3)), (id(2), id(4))],
        };
        let mut owned = BTreeSet::new();
        let mut aliases = BTreeSet::new();
        collect_pattern_storage(&pattern, &mut owned, &mut aliases, &mut 1000).unwrap();
        assert_eq!(owned, BTreeSet::from([id(1), id(3), id(5)]));
        assert_eq!(aliases, BTreeSet::from([id(2), id(4)]));
        assert!(
            collect_pattern_storage(&pattern, &mut BTreeSet::new(), &mut BTreeSet::new(), &mut 0)
                .is_err()
        );
    }
    #[test]
    fn reverse_remap_chains_fit_a_linear_budget() {
        let pattern = Pattern::Remap {
            pattern: Box::new(Pattern::Binding {
                binding: id(1),
                inner: Box::new(Pattern::Wild),
            }),
            bindings: (1..=1024).rev().map(|n| (id(n), id(n + 1))).collect(),
        };
        let mut owned = BTreeSet::new();
        collect_pattern_storage(&pattern, &mut owned, &mut BTreeSet::new(), &mut 7000).unwrap();
        assert_eq!(owned, (1..=1025).map(id).collect());
    }
    #[test]
    fn remap_cycles_need_seeds_and_preserve_alias_dominance() {
        let pattern = Pattern::Remap {
            pattern: Box::new(Pattern::Tuple(vec![
                Pattern::Binding {
                    binding: id(1),
                    inner: Box::new(Pattern::Wild),
                },
                Pattern::Observe {
                    binding: id(2),
                    inner: Box::new(Pattern::Wild),
                },
            ])),
            bindings: vec![
                (id(1), id(3)),
                (id(3), id(4)),
                (id(4), id(3)),
                (id(2), id(4)),
                (id(8), id(9)),
                (id(9), id(8)),
            ],
        };
        let mut owned = BTreeSet::new();
        let mut aliases = BTreeSet::new();
        collect_pattern_storage(&pattern, &mut owned, &mut aliases, &mut 1000).unwrap();
        assert_eq!(
            owned.difference(&aliases).copied().collect::<BTreeSet<_>>(),
            BTreeSet::from([id(1)])
        );
        assert_eq!(aliases, BTreeSet::from([id(2), id(3), id(4)]));
        assert!(!owned.contains(&id(8)) && !owned.contains(&id(9)));
    }
    #[test]
    fn budget_failure_does_not_publish_partial_candidates() {
        let body = block(vec![binding(1), binding(2)]);
        assert!(private_storage_roots(&body, &mut 0).is_err());
        assert_eq!(
            private_storage_roots(&body, &mut 1000).unwrap(),
            BTreeSet::from([id(1), id(2)])
        );
    }
}
