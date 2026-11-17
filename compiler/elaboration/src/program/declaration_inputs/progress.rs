//! Call-local preparation progress. Contextual producers may still await their
//! domains; an observed identity neither executes nor erases the source producer.
use super::*;

struct Prepared {
    producer: n::NormalFormArgument,
    identity: Option<Term>,
}

struct Slot {
    declaration: DeclarationId,
    sort: Sort,
    binding: n::LocalId,
    prepared: Option<Prepared>,
}

pub(super) struct Progress {
    parameters: Parameters,
    slots: Vec<Slot>,
    schedule: Vec<usize>,
    checked_prefix: usize,
    checked_order: Vec<usize>,
    observed_parameters: Vec<DeclarationId>,
    observed_values: Vec<Term>,
    observed_slots: Vec<usize>,
}

impl Progress {
    pub fn new(
        parameters: Parameters,
        bindings: Vec<n::LocalId>,
        mut explicit: Vec<usize>,
        fuel: &mut u64,
        span: Span,
    ) -> Result<Self, Error> {
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
            slots: parameters
                .iter()
                .zip(bindings)
                .map(|((declaration, sort, _), binding)| Slot {
                    declaration: *declaration,
                    sort: *sort,
                    binding,
                    prepared: None,
                })
                .collect(),
            schedule: explicit,
            checked_prefix: 0,
            checked_order: Vec::new(),
            observed_parameters: Vec::new(),
            observed_values: Vec::new(),
            observed_slots: Vec::new(),
            parameters,
        })
    }

    pub fn observed_parameters(&self) -> &[DeclarationId] {
        &self.observed_parameters
    }
    pub(super) fn instantiate(
        &self,
        checker: &mut Checker<'_, '_>,
        ty: &Term,
        span: Span,
    ) -> Result<Term, Error> {
        let domains = self
            .observed_slots
            .iter()
            .map(|slot| {
                environment::copy_term(&self.parameters[*slot].2, &mut checker.required_steps, span)
            })
            .collect::<Result<Vec<_>, _>>()?;
        let frames = checker.call_input_frames();
        let before = usize::try_from(checker.required_steps).unwrap_or(usize::MAX);
        let mut fuel = before;
        let result = nir::DeclarationSubstitution::bounded(
            &self.observed_parameters,
            &domains,
            &self.observed_values,
            &frames,
            &mut fuel,
        )
        .and_then(|substitution| substitution.instantiate_bounded(ty, &mut fuel));
        checker.required_steps -= (before - fuel) as u64;
        result.map_err(|failure| error(span, failure.to_string()))
    }

    // This is the first unfinished check in source order, not an execution PC.
    // Dependencies may complete checks beyond this prefix without running them.
    pub fn current(&self) -> Option<usize> {
        self.schedule.get(self.checked_prefix).copied()
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
            .and_then(|value| value.identity.as_ref())
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
            self.observed_parameters.push(self.slots[slot].declaration);
            self.observed_values.push(value.clone());
            self.observed_slots.push(slot);
        }
        self.slots[slot].prepared = Some(Prepared { producer, identity });
        self.checked_order.push(slot);
        while self.current().is_some_and(|next| self.is_prepared(next)) {
            self.checked_prefix += 1;
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
        let mut terms = Vec::with_capacity(self.slots.len());
        let mut deferred = BTreeSet::new();
        for slot in self.slots {
            let Some(prepared) = slot.prepared else {
                return Err(error(span, "missing prepared declaration input"));
            };
            arguments.push(prepared.producer);
            bindings.push(slot.binding);
            terms.push(prepared.identity.unwrap_or_else(|| {
                deferred.insert(slot.declaration);
                Term::Rigid {
                    declaration: slot.declaration,
                    sort: slot.sort,
                }
            }));
        }
        Ok(PreparedDeclarationInputs {
            arguments,
            plan: n::NormalFormInputPlan {
                order: self.schedule,
                preparations,
                bindings,
            },
            formation: FormationInputs {
                terms,
                deferred,
                environment: BTreeMap::new(),
            },
        })
    }
}

impl std::fmt::Display for Progress {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "(declaration-input-progress (checked-prefix {}) (schedule",
            self.checked_prefix
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
            write!(
                f,
                " (slot {index} {} {} (binding {}) (domain {})",
                slot.declaration, slot.sort, slot.binding, self.parameters[index].2
            )?;
            if let Some(prepared) = &slot.prepared {
                write!(f, " (producer {}) (identity", prepared.producer)?;
                match &prepared.identity {
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
            parameters.to_vec().into(),
            bindings.clone(),
            vec![2, 0],
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
        assert!(progress.observed_parameters().is_empty());
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
        assert_eq!(progress.observed_parameters, vec![parameters[2].0]);
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
        assert_eq!(
            prepared.formation.deferred,
            [parameters[0].0, parameters[1].0].into_iter().collect()
        );
        assert_eq!(prepared.formation.terms[2], identity);
        // Static observation retains the original source producer. A typed
        // consumer must still execute it in its source position exactly once.
        let checked = prepared.into_typed(&parameters, span).unwrap();
        assert_eq!(checked.statements.len(), 3);
        for (statement, index) in checked.statements.iter().zip([2, 0, 1]) {
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
    fn incomplete_or_malformed_progress_cannot_publish_a_plan() {
        let (parameters, bindings) = inputs();
        let span = Span::default();
        for order in [vec![3], vec![1, 1]] {
            assert!(
                Progress::new(
                    parameters.to_vec().into(),
                    bindings.clone(),
                    order,
                    &mut 100,
                    span
                )
                .is_err()
            );
        }
        assert!(
            Progress::new(
                parameters.to_vec().into(),
                bindings.clone(),
                vec![2, 0],
                &mut 0,
                span
            )
            .is_err()
        );
        let progress = Progress::new(
            parameters.to_vec().into(),
            bindings,
            vec![2, 0],
            &mut 100,
            span,
        )
        .unwrap();
        assert!(progress.finish(vec![], span).is_err());
    }
}
