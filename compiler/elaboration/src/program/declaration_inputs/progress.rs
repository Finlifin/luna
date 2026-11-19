//! Call-local preparation progress. Contextual producers may still await their
//! domains; an observed identity neither executes nor erases the source producer.
use super::*;

struct Prepared {
    producer: n::NormalFormArgument,
    expected_domain: Term,
}

struct Slot {
    binding: n::LocalId,
    prepared: Option<Prepared>,
}

pub(super) struct Progress {
    pub(super) telescope: domain::Telescope,
    slots: Vec<Slot>,
    pub(super) known_bindings: BTreeMap<n::LocalId, std::sync::Arc<Term>>,
    schedule: Vec<usize>,
    source_prefix: usize,
    declaration_prefix: usize,
    checked_order: Vec<usize>,
    caller_frames: Vec<Vec<Sort>>,
    captured: KnownInputs,
}

impl Progress {
    pub fn new(
        telescope: domain::Telescope,
        bindings: Vec<n::LocalId>,
        known_bindings: BTreeMap<n::LocalId, std::sync::Arc<Term>>,
        mut explicit: Vec<usize>,
        known: KnownInputs,
        caller_frames: Vec<Vec<Sort>>,
        fuel: &mut u64,
        span: Span,
    ) -> Result<Self, Error> {
        let parameters = &telescope.parameters;
        if parameters.len() != bindings.len() {
            return Err(error(span, "declaration input binding arity mismatch"));
        }
        let count = parameters.len();
        *fuel = fuel
            .checked_sub(count.saturating_mul(2).saturating_add(explicit.len()) as u64)
            .ok_or_else(|| error(span, "declaration input progress budget exhausted"))?;
        let mut supplied = vec![false; count];
        for &slot in &explicit {
            let Some(seen) = supplied.get_mut(slot) else {
                return Err(error(span, "declaration input schedule is out of range"));
            };
            if std::mem::replace(seen, true) {
                return Err(error(span, "duplicate declaration input schedule slot"));
            }
        }
        // Explicit inputs retain source order; omitted inputs follow the
        // declaration telescope. Each slot is scheduled once, including empty
        // collectors and defaults, so polling cannot skip unfinished checking.
        explicit.extend((0..count).filter(|&slot| !supplied[slot]));
        environment::charge(fuel, known.ids.len(), span)?;
        if let Some(id) = known
            .ids
            .iter()
            .find(|id| telescope.positions.contains_key(id))
        {
            return Err(error(
                span,
                nir::TypeError::DuplicateDeclarationParameter(*id).to_string(),
            ));
        }
        Ok(Self {
            slots: bindings
                .into_iter()
                .map(|binding| Slot {
                    binding,
                    prepared: None,
                })
                .collect(),
            known_bindings,
            schedule: explicit,
            source_prefix: 0,
            declaration_prefix: 0,
            checked_order: Vec::new(),
            caller_frames,
            captured: known,
            telescope,
        })
    }

    // Preparation is a source check. A slot whose producer is checked can
    // still be listed in domain.pending until its invocation supplies a value.
    pub(super) fn declaration_prefix(&self) -> usize {
        self.declaration_prefix
    }

    pub(super) fn may_need_preparation(&self, slot: usize) -> bool {
        slot > self.declaration_prefix && !self.telescope.direct(slot).is_empty()
    }

    pub(super) fn require_default_prefix(&self, slot: usize, span: Span) -> Result<(), Error> {
        if self.declaration_prefix < slot {
            return Err(error(span, "normal-form default awaits preceding input"));
        }
        Ok(())
    }

    pub(super) fn domain(
        &self,
        checker: &mut Checker<'_, '_>,
        slot: usize,
        span: Span,
    ) -> Result<DeclarationInputDomain, Error> {
        let pending = self.pending_inputs(slot, &mut checker.required_steps, span)?;
        let ty = self.instantiate(
            &self.telescope.parameters[slot].2,
            &mut checker.required_steps,
            span,
        )?;
        Ok(DeclarationInputDomain { ty, pending })
    }

    pub(super) fn instantiate(
        &self,
        domain: &Term,
        steps: &mut u64,
        span: Span,
    ) -> Result<Term, Error> {
        nominal_fields::metered(steps, |fuel| {
            nir::instantiate_observed_domain(
                domain,
                |id| self.observed(id),
                &self.caller_frames,
                fuel,
            )
        })
        .map_err(|reason| error(span, reason.to_string()))
    }

    fn observed(&self, id: DeclarationId) -> Option<(&Term, &Term)> {
        if let Some(&slot) = self.telescope.positions.get(&id) {
            return Some((&self.telescope.parameters[slot].2, self.identity(slot)?));
        }
        self.captured.observed(id)
    }

    /// A domain query needs just its own inputs. The handoff checks every
    /// observed identity together, including identities unused by later domains.
    fn check_observations(&self, steps: &mut u64, span: Span) -> Result<(), Error> {
        environment::charge(steps, self.slots.len(), span)?;
        let captured = self
            .captured
            .ids
            .iter()
            .copied()
            .zip(&self.captured.domains)
            .zip(&self.captured.values)
            .map(|((id, domain), value)| (id, domain, value));
        let local = self
            .telescope
            .parameters
            .iter()
            .enumerate()
            .filter_map(|(slot, (id, _, domain))| Some((*id, domain, self.identity(slot)?)));
        nominal_fields::metered(steps, |fuel| {
            nir::check_observed_inputs(captured.chain(local), &self.caller_frames, fuel)
        })
        .map_err(|reason| error(span, reason.to_string()))
    }

    fn pending_inputs(
        &self,
        slot: usize,
        fuel: &mut u64,
        span: Span,
    ) -> Result<BTreeSet<DeclarationId>, Error> {
        let direct = self.telescope.direct(slot);
        environment::charge(fuel, direct.len(), span)?;
        Ok(direct
            .iter()
            .copied()
            .filter(|&index| self.identity(index).is_none())
            .map(|index| self.telescope.parameters[index].0)
            .collect())
    }

    // This is the first unfinished check in source order, not an execution PC.
    // Dependencies may complete checks beyond this prefix without running them.
    pub fn current(&self) -> Option<usize> {
        self.schedule.get(self.source_prefix).copied()
    }
    pub fn binding(&self, slot: usize) -> n::LocalId {
        self.slots[slot].binding
    }
    pub fn is_prepared(&self, slot: usize) -> bool {
        self.slots[slot].prepared.is_some()
    }
    pub fn identity(&self, slot: usize) -> Option<&Term> {
        self.known_bindings
            .get(&self.binding(slot))
            .map(AsRef::as_ref)
    }

    pub(super) fn prepared_domain(&self, slot: usize) -> &Term {
        let input = self.slots[slot]
            .prepared
            .as_ref()
            .expect("checked default prefix");
        input
            .producer
            .expression()
            .map_or(&input.expected_domain, |value| &value.ty)
    }

    pub fn record(
        &mut self,
        slot: usize,
        producer: n::NormalFormArgument,
        expected_domain: Term,
        identity: Option<Term>,
        fuel: &mut u64,
        span: Span,
    ) -> Result<(), Error> {
        let Some(input) = self.slots.get(slot) else {
            return Err(error(span, "declaration input preparation is out of range"));
        };
        if input.prepared.is_some() {
            return Err(error(span, "declaration input was already prepared"));
        }
        *fuel = fuel
            .checked_sub(1)
            .ok_or_else(|| error(span, "declaration input recording budget exhausted"))?;
        if let Some(value) = &identity {
            let before = usize::try_from(*fuel).unwrap_or(usize::MAX);
            let mut remaining = before;
            let charged = nir::type_computations::charge_term(value, &mut remaining);
            *fuel -= (before - remaining) as u64;
            charged.map_err(|failure| {
                error(
                    span,
                    format!("declaration input identity capture failed: {failure}"),
                )
            })?;
        }
        if let Some(identity) = identity {
            self.known_bindings
                .insert(input.binding, std::sync::Arc::new(identity));
        }
        self.slots[slot].prepared = Some(Prepared {
            producer,
            expected_domain,
        });
        self.checked_order.push(slot);
        while self.declaration_prefix < self.slots.len()
            && self.is_prepared(self.declaration_prefix)
        {
            self.declaration_prefix += 1;
        }
        while self.current().is_some_and(|next| self.is_prepared(next)) {
            self.source_prefix += 1;
        }
        Ok(())
    }

    pub fn finish(
        mut self,
        preparations: Vec<n::NormalFormPreparation>,
        fuel: &mut u64,
        span: Span,
    ) -> Result<(PreparedTelescope, PreparedDeclarationInputs), Error> {
        if self.current().is_some() {
            return Err(error(span, "declaration input preparation is incomplete"));
        }
        self.check_observations(fuel, span)?;
        let mut arguments = Vec::with_capacity(self.slots.len());
        let mut bindings = Vec::with_capacity(self.slots.len());
        let mut slots = Vec::with_capacity(self.slots.len());
        for ((declaration, sort, _), slot) in self.telescope.parameters.iter().zip(self.slots) {
            let Some(prepared) = slot.prepared else {
                return Err(error(span, "missing prepared declaration input"));
            };
            arguments.push(prepared.producer);
            bindings.push(slot.binding);
            let binder = nir::DeclarationParameter {
                declaration: *declaration,
                sort: *sort,
            };
            let identity = self
                .known_bindings
                .remove(&slot.binding)
                .map(|known| {
                    std::sync::Arc::try_unwrap(known)
                        .or_else(|shared| environment::copy_term(&shared, fuel, span))
                })
                .transpose()?;
            slots.push((binder, identity));
        }
        let telescope = PreparedTelescope {
            captured: self.captured,
            local: self.telescope.parameters,
        };
        Ok((
            telescope,
            PreparedDeclarationInputs {
                arguments,
                plan: n::NormalFormInputPlan {
                    order: self.schedule,
                    preparations,
                    bindings,
                },
                observations: Observations::new(slots),
            },
        ))
    }
}

impl std::fmt::Display for Progress {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "(declaration-input-progress (source-prefix {}) (declaration-prefix {}) (schedule",
            self.source_prefix, self.declaration_prefix
        )?;
        for slot in &self.schedule {
            write!(f, " {slot}")?;
        }
        f.write_str(") (checked-order")?;
        for slot in &self.checked_order {
            write!(f, " {slot}")?;
        }
        f.write_str(") ")?;
        write!(f, "(caller-frames")?;
        for frame in &self.caller_frames {
            write!(f, " (frame")?;
            for sort in frame {
                write!(f, " {sort}")?;
            }
            f.write_str(")")?;
        }
        f.write_str(") ")?;
        write!(f, "{}", self.captured)?;
        f.write_str(" (slots")?;
        for (index, slot) in self.slots.iter().enumerate() {
            let (declaration, sort, domain) = &self.telescope.parameters[index];
            write!(
                f,
                " (slot {index} {declaration} {sort} (binding {}) (domain {domain})",
                slot.binding
            )?;
            if let Some(prepared) = &slot.prepared {
                write!(
                    f,
                    " (producer {}) (expected {}) (identity",
                    prepared.producer, prepared.expected_domain
                )?;
                match self.identity(index) {
                    Some(value) => write!(f, " {value}")?,
                    None => f.write_str(" pending")?,
                }
                f.write_str(")")?;
            } else {
                f.write_str(" (unprepared)")?;
            }
            f.write_str(")")?;
        }
        f.write_str("))")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inputs() -> (Vec<(DeclarationId, Sort, Term)>, Vec<n::LocalId>) {
        (
            (0..3)
                .map(|local| {
                    (
                        DeclarationId { package: 7, local },
                        Sort::ComptimeValue,
                        p(P::Str),
                    )
                })
                .collect(),
            (0..3).map(|index| n::LocalId { owner: 8, index }).collect(),
        )
    }

    fn producer(text: &str) -> n::NormalFormArgument {
        n::NormalFormArgument::Value(n::Expr {
            source: n::SourceRange { start: 3, end: 9 },
            ty: p(P::Str),
            kind: K::Literal(n::Literal::String(text.into())),
        })
    }

    #[test]
    fn source_domain_queries_only_pay_for_referenced_observations() {
        let span = Span::default();
        let cost = |count: u32| {
            let parameters = (0..count)
                .map(|local| (DeclarationId { package: 7, local }, Sort::Type, p(P::Type)))
                .collect();
            let bindings = (0..count)
                .map(|index| n::LocalId { owner: 8, index })
                .collect();
            let mut fuel = 500_000;
            let mut progress = Progress::new(
                domain::Telescope::new(parameters, &mut fuel, span).unwrap(),
                bindings,
                BTreeMap::new(),
                vec![],
                KnownInputs::default(),
                vec![],
                &mut fuel,
                span,
            )
            .unwrap();
            let target = Term::Rigid {
                declaration: progress.telescope.parameters[0].0,
                sort: Sort::Type,
            };
            for slot in (0..count as usize).rev() {
                let ty = p(P::Bool);
                let expr = n::Expr {
                    ty: p(P::Type),
                    kind: K::TypeValue(ty.clone()),
                    source: source(span),
                };
                progress
                    .record(
                        slot,
                        n::NormalFormArgument::Value(expr),
                        p(P::Type),
                        Some(ty),
                        &mut fuel,
                        span,
                    )
                    .unwrap();
            }
            let before = fuel;
            assert_eq!(
                progress.instantiate(&target, &mut fuel, span).unwrap(),
                p(P::Bool)
            );
            let spent = before - fuel;
            progress.finish(vec![], &mut fuel, span).unwrap();
            spent
        };
        assert_eq!(cost(1), cost(512));
    }

    #[test]
    fn dependency_checks_retain_source_producers_and_execution_order() {
        let (parameters, bindings) = inputs();
        let span = Span::default();
        let parent = DeclarationId {
            package: 7,
            local: 12,
        };
        let captured_value = Term::String("parent\"\n".into());
        let (prefix, local_domains) = KnownInputs::partition(
            &[parent],
            vec![p(P::Str)],
            std::slice::from_ref(&captured_value),
            &mut 100,
            span,
        )
        .unwrap();
        assert!(local_domains.is_empty());
        let mut progress = Progress::new(
            domain::Telescope::new(parameters.to_vec(), &mut 100, span).unwrap(),
            bindings.clone(),
            BTreeMap::new(),
            vec![2, 0],
            prefix,
            vec![],
            &mut 100,
            span,
        )
        .unwrap();
        assert_eq!(progress.current(), Some(2));
        let pending = progress.to_string();
        assert!(
            progress
                .record(3, producer("wrong"), p(P::Str), None, &mut 100, span)
                .is_err()
        );
        assert_eq!(progress.to_string(), pending);
        assert_eq!(progress.current(), Some(2));
        // A later source slot can finish checking before the current slot.
        // It must neither advance execution nor change the published schedule.
        progress
            .record(
                0,
                producer("producer-needs-invocation"),
                p(P::Str),
                None,
                &mut 100,
                span,
            )
            .unwrap();
        assert_eq!(progress.current(), Some(2));
        let pending = progress.to_string();
        assert!(
            progress
                .record(0, producer("duplicate"), p(P::Str), None, &mut 100, span)
                .is_err()
        );
        assert_eq!(progress.to_string(), pending);
        let identity = Term::String("observed".into());
        assert!(
            progress
                .record(
                    2,
                    producer("interrupted"),
                    p(P::Str),
                    Some(identity.clone()),
                    &mut 1,
                    span
                )
                .is_err()
        );
        assert_eq!(progress.to_string(), pending);
        assert!(progress.observed(parameters[2].0).is_none());
        progress
            .record(
                2,
                producer("quote\"\n"),
                p(P::Str),
                Some(identity.clone()),
                &mut 100,
                span,
            )
            .unwrap();
        assert_eq!(progress.identity(2), Some(&identity));
        assert_eq!(progress.observed(parameters[2].0).unwrap().1, &identity);
        assert_eq!(progress.current(), Some(1));
        progress
            .record(1, producer("default"), p(P::Str), None, &mut 100, span)
            .unwrap();
        assert_eq!(progress.current(), None);
        let dump = progress.to_string();
        assert!(dump.contains("(schedule 2 0 1) (checked-order 0 2 1)"));
        assert!(dump.contains("(identity pending)"));
        assert!(dump.contains("quote\\\"\\n"), "{dump}");
        assert!(
            dump.contains(&format!(
                "(captured (input {parent} {} {captured_value}))",
                p(P::Str)
            )),
            "{dump}"
        );
        let (telescope, prepared) = progress.finish(vec![], &mut 1_000, span).unwrap();
        assert_eq!(telescope.local, parameters);
        assert_eq!(
            telescope.captured.into_parts(),
            (vec![parent], vec![p(P::Str)], vec![captured_value])
        );
        assert_eq!(prepared.plan.order, vec![2, 0, 1]);
        assert_eq!(prepared.plan.bindings, bindings);
        assert!(prepared.observations.identity(0).is_none());
        assert!(prepared.observations.identity(1).is_none());
        assert_eq!(prepared.observations.identity(2), Some(&identity));
        // Static observation retains the original source producer. A typed
        // consumer must still execute it in its source position exactly once.
        let checked = prepared.into_typed(&mut 1000, span).unwrap();
        assert_eq!(checked.operands.statements.len(), 3);
        for (statement, index) in checked.operands.statements.iter().zip([2, 0, 1]) {
            let K::Let { binding, value } = &statement.kind else {
                panic!()
            };
            assert_eq!(*binding, bindings[index]);
            let K::Literal(n::Literal::String(text)) = &value.kind else {
                panic!()
            };
            assert_eq!(
                text.as_str(),
                ["producer-needs-invocation", "default", "quote\"\n"][index]
            );
        }
    }

    #[test]
    fn an_unobserved_checked_producer_keeps_its_domain_waiter() {
        let (mut parameters, bindings) = inputs();
        let id = parameters[0].0;
        parameters[0].1 = Sort::Type;
        parameters[0].2 = p(P::Type);
        parameters[1].2 = Term::Rigid {
            declaration: id,
            sort: Sort::Type,
        };
        let span = Span::default();
        let mut fuel = 200;
        let mut progress = Progress::new(
            domain::Telescope::new(parameters, &mut fuel, span).unwrap(),
            bindings,
            BTreeMap::new(),
            vec![1, 0],
            KnownInputs::default(),
            vec![],
            &mut fuel,
            span,
        )
        .unwrap();
        assert!(progress.may_need_preparation(1));
        // A typed Type-valued read leaves its actual identity to execution.
        let read = n::Expr {
            source: source(span),
            ty: p(P::Type),
            kind: K::Local(n::LocalId { owner: 9, index: 0 }),
        };
        progress
            .record(
                0,
                n::NormalFormArgument::Value(read),
                p(P::Type),
                None,
                &mut fuel,
                span,
            )
            .unwrap();
        assert!(!progress.may_need_preparation(1));
        assert_eq!(
            progress.pending_inputs(1, &mut fuel, span).unwrap(),
            BTreeSet::from([id])
        );
        assert_eq!(progress.current(), Some(1));
    }

    #[test]
    fn incomplete_or_malformed_progress_cannot_publish_a_plan() {
        let (parameters, bindings) = inputs();
        let span = Span::default();
        for order in [vec![3], vec![1, 1]] {
            assert!(
                Progress::new(
                    domain::Telescope::new(parameters.to_vec(), &mut 100, span).unwrap(),
                    bindings.clone(),
                    BTreeMap::new(),
                    order,
                    KnownInputs::default(),
                    vec![],
                    &mut 100,
                    span
                )
                .is_err()
            );
        }
        assert!(
            Progress::new(
                domain::Telescope::new(parameters.to_vec(), &mut 100, span).unwrap(),
                bindings.clone(),
                BTreeMap::new(),
                vec![2, 0],
                KnownInputs::default(),
                vec![],
                &mut 0,
                span
            )
            .is_err()
        );
        let progress = Progress::new(
            domain::Telescope::new(parameters.to_vec(), &mut 100, span).unwrap(),
            bindings,
            BTreeMap::new(),
            vec![2, 0],
            KnownInputs::default(),
            vec![],
            &mut 100,
            span,
        )
        .unwrap();
        assert!(progress.finish(vec![], &mut 1_000, span).is_err());
    }
}
