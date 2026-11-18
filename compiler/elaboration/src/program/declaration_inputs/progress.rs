//! Call-local preparation progress. Contextual producers may still await their
//! domains; an observed identity neither executes nor erases the source producer.
use super::*;

struct Prepared {
    producer: n::NormalFormArgument,
    observation: Option<usize>,
}

struct Slot {
    binding: n::LocalId,
    prepared: Option<Prepared>,
}

pub(super) struct Progress {
    pub(super) telescope: domain::Telescope,
    slots: Vec<Slot>,
    schedule: Vec<usize>,
    source_prefix: usize,
    declaration_prefix: usize,
    checked_order: Vec<usize>,
    pub(super) known: KnownInputs,
}

impl Progress {
    pub fn new(
        telescope: domain::Telescope,
        bindings: Vec<n::LocalId>,
        mut explicit: Vec<usize>,
        known: KnownInputs,
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
        Ok(Self {
            slots: bindings
                .into_iter()
                .map(|binding| Slot {
                    binding,
                    prepared: None,
                })
                .collect(),
            schedule: explicit,
            source_prefix: 0,
            declaration_prefix: 0,
            checked_order: Vec::new(),
            known,
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
        let ty = self
            .known
            .with_substitution(checker, span, |substitution, fuel| {
                substitution.instantiate_bounded(&self.telescope.parameters[slot].2, fuel)
            })?;
        Ok(DeclarationInputDomain { ty, pending })
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
        self.slots[slot]
            .prepared
            .as_ref()
            .and_then(|value| value.observation.map(|index| &self.known.values[index]))
    }

    pub fn record(
        &mut self,
        slot: usize,
        producer: n::NormalFormArgument,
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
        let observation = if let Some(value) = identity {
            let before = usize::try_from(*fuel).unwrap_or(usize::MAX);
            let mut remaining = before;
            let charged = nir::type_computations::charge_term(&value, &mut remaining);
            *fuel -= (before - remaining) as u64;
            charged.map_err(|failure| {
                error(
                    span,
                    format!("declaration input identity capture failed: {failure}"),
                )
            })?;
            self.known.domains.push(environment::copy_term(
                &self.telescope.parameters[slot].2,
                fuel,
                span,
            )?);
            self.known.ids.push(self.telescope.parameters[slot].0);
            self.known.values.push(value);
            Some(self.known.values.len() - 1)
        } else {
            None
        };
        self.slots[slot].prepared = Some(Prepared {
            producer,
            observation,
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
        self,
        preparations: Vec<n::NormalFormPreparation>,
        span: Span,
    ) -> Result<PreparedDeclarationInputs, Error> {
        if self.current().is_some() {
            return Err(error(span, "declaration input preparation is incomplete"));
        }
        let mut arguments = Vec::with_capacity(self.slots.len());
        let mut bindings = Vec::with_capacity(self.slots.len());
        let mut slots = Vec::with_capacity(self.slots.len());
        let mut values: Vec<_> = self.known.values.into_iter().map(Some).collect();
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
            slots.push((
                binder,
                prepared
                    .observation
                    .map(|index| values[index].take().expect("unique observed input")),
            ));
        }
        Ok(PreparedDeclarationInputs {
            arguments,
            plan: n::NormalFormInputPlan {
                order: self.schedule,
                preparations,
                bindings,
            },
            observations: Observations::new(slots),
        })
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
        f.write_str(") (slots")?;
        for (index, slot) in self.slots.iter().enumerate() {
            let (declaration, sort, domain) = &self.telescope.parameters[index];
            write!(
                f,
                " (slot {index} {declaration} {sort} (binding {}) (domain {domain})",
                slot.binding
            )?;
            if let Some(prepared) = &slot.prepared {
                write!(f, " (producer {}) (identity", prepared.producer)?;
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
    fn dependency_checks_retain_source_producers_and_execution_order() {
        let (parameters, bindings) = inputs();
        let span = Span::default();
        let mut progress = Progress::new(
            domain::Telescope::new(parameters.to_vec().into(), &mut 100, span).unwrap(),
            bindings.clone(),
            vec![2, 0],
            KnownInputs::default(),
            &mut 100,
            span,
        )
        .unwrap();
        assert_eq!(progress.current(), Some(2));
        let pending = progress.to_string();
        assert!(
            progress
                .record(3, producer("wrong"), None, &mut 100, span)
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
                None,
                &mut 100,
                span,
            )
            .unwrap();
        assert_eq!(progress.current(), Some(2));
        let pending = progress.to_string();
        assert!(
            progress
                .record(0, producer("duplicate"), None, &mut 100, span)
                .is_err()
        );
        assert_eq!(progress.to_string(), pending);
        let identity = Term::String("observed".into());
        assert!(
            progress
                .record(
                    2,
                    producer("interrupted"),
                    Some(identity.clone()),
                    &mut 1,
                    span
                )
                .is_err()
        );
        assert_eq!(progress.to_string(), pending);
        assert!(progress.known.ids.is_empty());
        progress
            .record(
                2,
                producer("quote\"\n"),
                Some(identity.clone()),
                &mut 100,
                span,
            )
            .unwrap();
        assert_eq!(progress.identity(2), Some(&identity));
        assert_eq!(progress.known.ids, vec![parameters[2].0]);
        assert_eq!(progress.current(), Some(1));
        progress
            .record(1, producer("default"), None, &mut 100, span)
            .unwrap();
        assert_eq!(progress.current(), None);
        let dump = progress.to_string();
        assert!(dump.contains("(schedule 2 0 1) (checked-order 0 2 1)"));
        assert!(dump.contains("(identity pending)"));
        assert!(dump.contains("quote\\\"\\n"), "{dump}");
        let prepared = progress.finish(vec![], span).unwrap();
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
            domain::Telescope::new(parameters.into(), &mut fuel, span).unwrap(),
            bindings,
            vec![1, 0],
            KnownInputs::default(),
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
            .record(0, n::NormalFormArgument::Value(read), None, &mut fuel, span)
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
                    domain::Telescope::new(parameters.to_vec().into(), &mut 100, span).unwrap(),
                    bindings.clone(),
                    order,
                    KnownInputs::default(),
                    &mut 100,
                    span
                )
                .is_err()
            );
        }
        assert!(
            Progress::new(
                domain::Telescope::new(parameters.to_vec().into(), &mut 100, span).unwrap(),
                bindings.clone(),
                vec![2, 0],
                KnownInputs::default(),
                &mut 0,
                span
            )
            .is_err()
        );
        let progress = Progress::new(
            domain::Telescope::new(parameters.to_vec().into(), &mut 100, span).unwrap(),
            bindings,
            vec![2, 0],
            KnownInputs::default(),
            &mut 100,
            span,
        )
        .unwrap();
        assert!(progress.finish(vec![], span).is_err());
    }
}
