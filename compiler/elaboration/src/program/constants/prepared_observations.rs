//! Temporary observations of prepared inputs, owned across source child checks.
//! These facts do not supply invocation values or grant execution permission.
use super::*;

#[must_use = "leave prepared observations in the same active checker"]
pub(in crate::program) struct PreparedObservations {
    saved: Vec<(n::LocalId, Option<std::sync::Arc<PartialStagedValue>>, bool)>,
}

impl PreparedObservations {
    pub(in crate::program) fn enter(
        checker: &mut Checker<'_, '_>,
        facts: &BTreeMap<n::LocalId, std::sync::Arc<Term>>,
        span: Span,
    ) -> Result<Self, Error> {
        Self::enter_in(
            &mut checker.input_facts.partial,
            &mut checker.immutable,
            &mut checker.required_steps,
            facts,
            span,
        )
    }

    fn enter_in(
        partial: &mut BTreeMap<n::LocalId, std::sync::Arc<PartialStagedValue>>,
        immutable: &mut BTreeSet<n::LocalId>,
        steps: &mut u64,
        facts: &BTreeMap<n::LocalId, std::sync::Arc<Term>>,
        span: Span,
    ) -> Result<Self, Error> {
        let Some(remaining) = steps.checked_sub(facts.len() as u64) else {
            *steps = 0;
            return Err(error(span, "prepared input observation budget exhausted"));
        };
        *steps = remaining;
        let mut saved = Vec::with_capacity(facts.len());
        for (binding, identity) in facts {
            let previous = partial.insert(*binding, PartialStagedValue::known(identity.clone()));
            let newly_immutable = immutable.insert(*binding);
            saved.push((*binding, previous, newly_immutable));
        }
        Ok(Self { saved })
    }

    pub(in crate::program) fn leave(self, checker: &mut Checker<'_, '_>) {
        self.leave_in(&mut checker.input_facts.partial, &mut checker.immutable);
    }

    fn leave_in(
        self,
        partial: &mut BTreeMap<n::LocalId, std::sync::Arc<PartialStagedValue>>,
        immutable: &mut BTreeSet<n::LocalId>,
    ) {
        // Only this packet's entries are restored. Unrelated writes retain
        // their invalidated observations and ordinary storage responsibilities.
        for (binding, previous, newly_immutable) in self.saved {
            if let Some(previous) = previous {
                partial.insert(binding, previous);
            } else {
                partial.remove(&binding);
            }
            if newly_immutable {
                immutable.remove(&binding);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn prepared_observations_charge_before_publishing_a_scope() {
        let ids = [1, 2, 3].map(|index| n::LocalId { owner: 1, index });
        let original = PartialStagedValue::known(Term::Bool(false));
        let facts = BTreeMap::from([
            (ids[0], Arc::new(Term::Bool(true))),
            (ids[1], Arc::new(Term::Bool(true))),
        ]);
        for budget in 0..2 {
            let mut partial = BTreeMap::from([(ids[0], original.clone())]);
            let mut immutable = BTreeSet::from([ids[0], ids[2]]);
            let mut fuel = budget;
            assert!(
                PreparedObservations::enter_in(
                    &mut partial,
                    &mut immutable,
                    &mut fuel,
                    &facts,
                    Span::default()
                )
                .is_err()
            );
            assert_eq!(fuel, 0);
            assert_eq!(partial.len(), 1);
            assert!(Arc::ptr_eq(&partial[&ids[0]], &original));
            assert_eq!(immutable, BTreeSet::from([ids[0], ids[2]]));
            let scope = PreparedObservations::enter_in(
                &mut partial,
                &mut immutable,
                &mut 10,
                &facts,
                Span::default(),
            )
            .unwrap();
            scope.leave_in(&mut partial, &mut immutable);
            assert!(Arc::ptr_eq(&partial[&ids[0]], &original));
            assert_eq!(partial.len(), 1);
        }
    }

    #[test]
    fn a_child_check_shares_prepared_payloads_before_it_reads_them() {
        let id = n::LocalId { owner: 3, index: 4 };
        let large = Arc::new(Term::TupleValue(vec![Term::Bool(true); 2048]));
        let facts = BTreeMap::from([(id, large.clone())]);
        let mut partial = BTreeMap::new();
        let mut immutable = BTreeSet::new();
        let scope = PreparedObservations::enter_in(
            &mut partial,
            &mut immutable,
            &mut 1,
            &facts,
            Span::default(),
        )
        .unwrap();
        assert!(std::ptr::eq(
            partial[&id].known_term().unwrap(),
            large.as_ref()
        ));
        scope.leave_in(&mut partial, &mut immutable);
        assert!(partial.is_empty() && immutable.is_empty());
    }

    #[test]
    fn prepared_observations_leave_packet_facts_without_reviving_outer_writes() {
        let ids = [1, 2, 3].map(|index| n::LocalId { owner: 1, index });
        let original = PartialStagedValue::known(Term::String("parent\"\n".into()));
        let mut partial = BTreeMap::from([(ids[0], original.clone()), (ids[2], original.clone())]);
        let mut immutable = BTreeSet::from([ids[0]]);
        let facts = BTreeMap::from([
            (ids[0], Arc::new(Term::Bool(true))),
            (ids[1], Arc::new(Term::Bool(false))),
        ]);
        let scope = PreparedObservations::enter_in(
            &mut partial,
            &mut immutable,
            &mut 10,
            &facts,
            Span::default(),
        )
        .unwrap();
        let dump = scope.to_string();
        assert!(dump.contains("parent\\\"\\n"), "{dump}");
        assert!(dump.contains("(newly-immutable true) (previous)"), "{dump}");
        assert!(!dump.contains('\n'));
        // A child invalidates unrelated outer storage. Packet exit must not
        // restore the whole map and thereby revive this stale observation.
        partial.remove(&ids[2]);
        scope.leave_in(&mut partial, &mut immutable);
        assert_eq!(partial.len(), 1);
        assert!(Arc::ptr_eq(&partial[&ids[0]], &original));
        assert_eq!(immutable, BTreeSet::from([ids[0]]));
    }
}

impl std::fmt::Display for PreparedObservations {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("(prepared-input-observations (parent")?;
        for (binding, previous, newly_immutable) in &self.saved {
            write!(
                f,
                " (binding {binding} (newly-immutable {newly_immutable}) (previous"
            )?;
            if let Some(previous) = previous {
                write!(f, " {previous}")?;
            }
            f.write_str("))")?;
        }
        f.write_str("))")
    }
}
