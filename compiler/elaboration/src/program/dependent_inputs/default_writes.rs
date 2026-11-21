//! Values of parameter slots on normal completion of selected defaults. Reads
//! refer to the slot version at the store, independently of checking order.
use super::*;
use std::sync::Arc;

#[derive(Clone, Copy)]
pub(super) enum Read {
    Argument(usize),
    Store(usize),
    Unknown,
}

impl std::fmt::Display for Read {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Argument(i) => write!(f, "(argument {i})"),
            Self::Store(i) => write!(f, "(store {i})"),
            Self::Unknown => f.write_str("(unknown)"),
        }
    }
}

struct Store {
    rhs: n::Expr,
    inputs: BTreeMap<n::LocalId, Read>,
    value: Option<Arc<Term>>,
}

#[derive(Default)]
pub(super) struct Writes {
    versions: BTreeMap<n::LocalId, Vec<(usize, Read)>>,
    stores: Vec<Store>,
}

fn unavailable(span: Span) -> Error {
    context_error(
        span,
        "dependent default needs the parameter state left by an earlier default in the original invocation",
    )
}

impl Writes {
    pub(super) fn collect(
        checker: &mut Checker<'_, '_>,
        owner: Option<OwnerId>,
        parameters: &[Parameter],
        defaults: &[Option<n::Expr>],
        span: Span,
    ) -> Result<Self, Error> {
        let mut writes = Self::default();
        tasks::spend(&mut checker.required_steps, defaults.len(), span)?;
        let mut selected = defaults
            .iter()
            .enumerate()
            .filter_map(|(i, expr)| Some((i, expr.as_ref()?)));
        // Only a selected default to the right can read a published version.
        selected.next_back();
        let Some(first) = selected.next() else {
            return Ok(writes);
        };
        let Some(ItemKind::Fn(sig, _) | ItemKind::FnDeclaration(sig)) = owner
            .and_then(|id| checker.package.item(id))
            .map(|i| &i.kind)
        else {
            return Ok(writes);
        };
        let receiver = checker.has_comptime_value_receiver(owner);
        let mut formals = BTreeMap::new();
        tasks::spend(&mut checker.required_steps, sig.params.len(), span)?;
        for (slot, (formal, param)) in sig.params.iter().zip(parameters).enumerate() {
            let detached = !(slot == 0 && receiver)
                && matches!(param.ty, Term::Primitive(p) if p != P::Object);
            formals.insert(local(formal.hir_id), (slot, detached));
        }
        for (writer, expr) in std::iter::once(first).chain(selected) {
            let (free, changed) = checker.with_observation_budget(span, |_, budget| {
                let free = nir::free_bindings::collect(expr, &mut budget.steps)?;
                budget.consume(free.len())?;
                let mut retained: BTreeMap<_, _> = free
                    .iter()
                    .filter(|id| formals.contains_key(id))
                    .map(|&id| (id, ()))
                    .collect();
                Checker::invalidate_mutated_type_bindings(expr, &mut retained, &mut budget.steps)?;
                let changed: Vec<_> = free
                    .iter()
                    .copied()
                    .filter(|id| formals.contains_key(id) && !retained.contains_key(id))
                    .collect();
                Ok((free, changed))
            })?;
            if changed.is_empty() {
                continue;
            }
            // A frame slot whose representation may alias other storage can
            // be invalidated, but cannot supply an independent stored value.
            let mut locals = BTreeMap::new();
            for id in free {
                if let Some(&(slot, true)) = formals.get(&id) {
                    locals.insert(id, writes.read(id, slot, writer));
                }
            }
            writes.transfer(checker, expr, &formals, &mut locals, span)?;
            for id in changed {
                writes
                    .versions
                    .entry(id)
                    .or_default()
                    .push((writer, locals.get(&id).copied().unwrap_or(Read::Unknown)));
            }
        }
        Ok(writes)
    }

    pub(super) fn read(&self, id: n::LocalId, argument: usize, reader: usize) -> Read {
        self.versions
            .get(&id)
            .and_then(|writes| {
                let end = writes.partition_point(|(slot, _)| *slot < reader);
                writes[..end].last()
            })
            .map_or(Read::Argument(argument), |(_, read)| *read)
    }

    fn capture(
        &mut self,
        checker: &mut Checker<'_, '_>,
        rhs: &n::Expr,
        locals: &BTreeMap<n::LocalId, Read>,
        span: Span,
    ) -> Result<Read, Error> {
        if let K::Local(id) = rhs.kind {
            return Ok(locals.get(&id).copied().unwrap_or(Read::Unknown));
        }
        // An open annotation belongs to its declaration telescope. Keep it
        // unavailable until observation can specialize that scope at the read.
        if !checker.checked_substitution_inputs(rhs, span)?.is_empty() {
            return Ok(Read::Unknown);
        }
        let free = checker.with_observation_budget(span, |_, budget| {
            Ok(nir::free_bindings::collect(rhs, &mut budget.steps)?)
        })?;
        let mut inputs = BTreeMap::new();
        for id in free {
            let Some(&read) = locals.get(&id) else {
                return Ok(Read::Unknown);
            };
            if matches!(read, Read::Unknown) {
                return Ok(Read::Unknown);
            }
            inputs.insert(id, read);
        }
        constants::charge_copy(rhs, &mut checker.required_steps, span)?;
        let id = self.stores.len();
        self.stores.push(Store {
            rhs: rhs.clone(),
            inputs,
            value: None,
        });
        Ok(Read::Store(id))
    }

    fn transfer(
        &mut self,
        checker: &mut Checker<'_, '_>,
        expr: &n::Expr,
        formals: &BTreeMap<n::LocalId, (usize, bool)>,
        locals: &mut BTreeMap<n::LocalId, Read>,
        span: Span,
    ) -> Result<(), Error> {
        let mut pending = vec![expr];
        while let Some(expr) = pending.pop() {
            tasks::spend(&mut checker.required_steps, 1, span)?;
            match &expr.kind {
                K::Block { statements, result } | K::CallPreparation { statements, result } => {
                    tasks::spend(&mut checker.required_steps, statements.len(), span)?;
                    pending.extend(result.as_deref());
                    pending.extend(statements.iter().rev());
                }
                K::Discard(value) | K::UnsafeScope(value) => pending.push(value),
                _ => {
                    let store = match &expr.kind {
                        K::Let { binding, value } => Some((*binding, value)),
                        K::Assign { place, value } if place.is_root() => Some((place.root, value)),
                        _ => None,
                    };
                    let new_value = if let Some((id, rhs)) = store
                        && matches!(rhs.ty, Term::Primitive(p) if p != P::Object)
                        && !matches!(formals.get(&id), Some((_, false)))
                    {
                        Some((id, self.capture(checker, rhs, locals, span)?))
                    } else {
                        None
                    };
                    checker.with_observation_budget(span, |_, budget| {
                        Ok(Checker::invalidate_mutated_type_bindings(
                            expr,
                            locals,
                            &mut budget.steps,
                        )?)
                    })?;
                    if let Some((id, read)) = new_value {
                        locals.insert(id, read);
                    }
                }
            }
        }
        Ok(())
    }

    /// Reach only the stores used by this default. Older stores precede their
    /// readers in the arena; shared source reads and completed values are reused.
    pub(super) fn dependencies(
        &self,
        reads: impl ExactSizeIterator<Item = Read>,
        steps: &mut u64,
        span: Span,
    ) -> Result<(BTreeSet<usize>, BTreeSet<usize>), Error> {
        tasks::spend(steps, reads.len(), span)?;
        let mut pending: Vec<_> = reads.collect();
        let mut arguments = BTreeSet::new();
        let mut stores = BTreeSet::new();
        while let Some(read) = pending.pop() {
            tasks::spend(steps, 1, span)?;
            match read {
                Read::Argument(slot) => {
                    arguments.insert(slot);
                }
                Read::Store(id) if self.stores[id].value.is_none() && stores.insert(id) => {
                    let inputs = &self.stores[id].inputs;
                    tasks::spend(steps, inputs.len(), span)?;
                    pending.extend(inputs.values().copied());
                }
                Read::Store(_) => {}
                Read::Unknown => return Err(unavailable(span)),
            }
        }
        Ok((arguments, stores))
    }

    pub(super) fn value<'a>(
        &'a self,
        read: Read,
        arguments: &'a [Option<Term>],
        span: Span,
    ) -> Result<&'a Term, Error> {
        match read {
            Read::Argument(slot) => arguments[slot].as_ref(),
            Read::Store(id) => self.stores[id].value.as_deref(),
            Read::Unknown => None,
        }
        .ok_or_else(|| unavailable(span))
    }

    pub(super) fn observe(
        &mut self,
        checker: &mut Checker<'_, '_>,
        stores: BTreeSet<usize>,
        arguments: &[Option<Term>],
        span: Span,
    ) -> Result<(), Error> {
        for id in stores {
            let store = &self.stores[id];
            let mut facts = BTreeMap::new();
            for (&binding, &read) in &store.inputs {
                let value = match read {
                    Read::Store(id) => self.stores[id]
                        .value
                        .clone()
                        .ok_or_else(|| unavailable(span))?,
                    Read::Argument(_) => Arc::new(tasks::copy_evidence(
                        self.value(read, arguments, span)?,
                        &mut checker.required_steps,
                        span,
                    )?),
                    Read::Unknown => return Err(unavailable(span)),
                };
                facts.insert(binding, value);
            }
            let value = checker
                .with_observation_budget(span, |checker, budget| {
                    // Captured store inputs are detached scalars. Independent
                    // work in the RHS still executes in the original default.
                    budget.consume(facts.len())?;
                    let snapshots = facts.keys().copied().collect();
                    normal::observation::hint(
                        checker.observe_preserved_input_in(&store.rhs, facts, &snapshots, budget),
                    )
                })?
                .ok_or_else(|| unavailable(span))?;
            self.stores[id].value = Some(Arc::new(value));
        }
        Ok(())
    }
}

impl std::fmt::Display for Writes {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("(default-writes")?;
        for (binding, versions) in &self.versions {
            for (writer, read) in versions {
                write!(f, " (write {binding} {writer} {read})")?;
            }
        }
        for (index, store) in self.stores.iter().enumerate() {
            write!(f, " (store {index} (rhs {}) (inputs", store.rhs)?;
            for (id, read) in &store.inputs {
                write!(f, " (input {id} {read})")?;
            }
            f.write_str("))")?;
        }
        f.write_str(")")
    }
}
