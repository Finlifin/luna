//! Read-only input evidence owned by an active required preparation region.
//! These are supplied values, never pending producers or live mutable storage.
use super::*;
use std::{fmt, sync::Arc};

/// All visible observations, indexed independently of the depth of the caller.
/// Saved versions own their old root. A declaration and a mask both count as
/// a supply in the region that introduced them.
#[derive(Clone, Default)]
pub(in crate::program) struct Scope(Option<Region>);

#[derive(Clone)]
struct Region {
    owner: Arc<Owner>,
    visible: LocalBindings<Observation>,
}

struct Owner;

struct Observation {
    owner: Arc<Owner>,
    domain: Arc<Term>,
    value: Option<Arc<Term>>,
}

impl Region {
    fn supply(
        &mut self,
        binding: n::LocalId,
        domain: &Term,
        value: Option<&Term>,
        fuel: &mut u64,
        span: Span,
    ) -> Result<(), Error> {
        charge_index(fuel, span)?;
        if self
            .visible
            .get(&binding)
            .is_some_and(|entry| Arc::ptr_eq(&self.owner, &entry.owner))
        {
            return Err(error(span, "duplicate required comptime input"));
        }
        let domain = Arc::new(copy(domain, fuel, span)?);
        let value = value
            .map(|value| copy(value, fuel, span).map(Arc::new))
            .transpose()?;
        self.insert(binding, domain, value, fuel, span)
    }

    fn insert(
        &mut self,
        binding: n::LocalId,
        domain: Arc<Term>,
        value: Option<Arc<Term>>,
        fuel: &mut u64,
        span: Span,
    ) -> Result<(), Error> {
        // LocalId has a fixed width. One index operation visits at most one
        // branch per bit, regardless of the number of saved caller scopes.
        charge_index(fuel, span)?;
        self.visible.insert(
            binding,
            Observation {
                owner: self.owner.clone(),
                domain,
                value,
            },
        );
        Ok(())
    }
}

fn charge_index(fuel: &mut u64, span: Span) -> Result<(), Error> {
    *fuel = fuel
        .checked_sub(1)
        .ok_or_else(|| error(span, "required input observation budget exhausted"))?;
    Ok(())
}

impl Scope {
    pub(in crate::program) fn child(
        &self,
        inputs: &[(n::LocalId, Term, Term)],
        live: &[(n::LocalId, Term, Value)],
        fuel: &mut u64,
        span: Span,
    ) -> Result<Self, Error> {
        let mut region = Region {
            owner: Arc::new(Owner),
            visible: self
                .0
                .as_ref()
                .map(|parent| parent.visible.clone())
                .unwrap_or_default(),
        };
        for (binding, domain, value) in inputs {
            region.supply(*binding, domain, Some(value), fuel, span)?;
        }
        for (binding, domain, _) in live {
            region.supply(*binding, domain, None, fuel, span)?;
        }
        Ok(Self(Some(region)))
    }

    pub(in crate::program) fn supply(
        &mut self,
        binding: n::LocalId,
        domain: &Term,
        value: &Term,
        fuel: &mut u64,
        span: Span,
    ) -> Result<(), Error> {
        self.0
            .as_mut()
            .expect("active required input region")
            .supply(binding, domain, Some(value), fuel, span)
    }

    pub(in crate::program) fn is_empty(&self) -> bool {
        self.0.is_none()
    }

    pub(in crate::program) fn invalidate(
        &mut self,
        bindings: &BTreeSet<n::LocalId>,
        fuel: &mut u64,
        span: Span,
    ) -> Result<(), Error> {
        let Some(mut region) = self.0.clone() else {
            return Ok(());
        };
        // Publish a new root only after all affected bindings have been masked.
        // Failure spends the lookup/update work but preserves the prior version.
        for binding in bindings {
            charge_index(fuel, span)?;
            if let Some(entry) = region.visible.get(binding)
                && entry.value.is_some()
            {
                region.insert(*binding, entry.domain.clone(), None, fuel, span)?;
            }
        }
        self.0 = Some(region);
        Ok(())
    }

    pub(in crate::program) fn lookup(
        &self,
        binding: n::LocalId,
        fuel: &mut u64,
        span: Span,
    ) -> Result<Option<(&Term, Option<&Term>)>, Error> {
        Ok(self
            .shared(binding, fuel, span)?
            .map(|(domain, value)| (domain.as_ref(), value.map(Arc::as_ref))))
    }

    pub(in crate::program) fn shared(
        &self,
        binding: n::LocalId,
        fuel: &mut u64,
        span: Span,
    ) -> Result<Option<(&Arc<Term>, Option<&Arc<Term>>)>, Error> {
        let Some(region) = &self.0 else {
            return Ok(None);
        };
        charge_index(fuel, span)?;
        Ok(region
            .visible
            .get(&binding)
            .map(|entry| (&entry.domain, entry.value.as_ref())))
    }
}

pub(super) fn copy(value: &Term, fuel: &mut u64, span: Span) -> Result<Term, Error> {
    let before = usize::try_from(*fuel).unwrap_or(usize::MAX);
    let mut remaining = before;
    let result = nir::copy_budget::clone_term(value, &mut remaining);
    *fuel -= (before - remaining) as u64;
    result.map_err(|failure| error(span, format!("required input snapshot failed: {failure:?}")))
}

impl fmt::Display for Scope {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("(required-input-scope")?;
        if let Some(region) = &self.0 {
            f.write_str(" (frame")?;
            for (binding, entry) in &region.visible {
                write!(f, " (input {binding} (domain {}) ", entry.domain)?;
                match &entry.value {
                    Some(value) => write!(f, "(supplied {value})")?,
                    None => f.write_str("(unavailable)")?,
                }
                if !Arc::ptr_eq(&region.owner, &entry.owner) {
                    f.write_str(" (inherited)")?;
                }
                f.write_str(")")?;
            }
            f.write_str(")")?;
        }
        f.write_str(")")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshots_shadow_per_region_and_live_inputs_block_outer_values() {
        let span = Span::default();
        let binding = n::LocalId { owner: 1, index: 2 };
        let outer = Scope::default()
            .child(
                &[(binding, p(P::Bool), Term::Bool(true))],
                &[],
                &mut 1000,
                span,
            )
            .unwrap();
        let inner = outer
            .child(
                &[(binding, p(P::Bool), Term::Bool(false))],
                &[],
                &mut 1000,
                span,
            )
            .unwrap();
        assert_eq!(
            inner.lookup(binding, &mut 100, span).unwrap(),
            Some((&p(P::Bool), Some(&Term::Bool(false))))
        );
        let live = inner
            .child(
                &[],
                &[(binding, p(P::Bool), Value::Bool(true))],
                &mut 1000,
                span,
            )
            .unwrap();
        assert_eq!(
            live.lookup(binding, &mut 100, span).unwrap(),
            Some((&p(P::Bool), None))
        );
        drop(live);
        drop(inner);
        assert_eq!(
            outer.lookup(binding, &mut 100, span).unwrap(),
            Some((&p(P::Bool), Some(&Term::Bool(true))))
        );
        assert_eq!(
            outer.to_string(),
            "(required-input-scope (frame (input (local 1 2) (domain (primitive bool)) (supplied (bool true)))))"
        );
    }

    #[test]
    fn completed_prefix_inputs_extend_only_the_current_region() {
        let span = Span::default();
        let binding = n::LocalId { owner: 3, index: 0 };
        let mut scope = Scope::default().child(&[], &[], &mut 100, span).unwrap();
        let earlier = scope.clone();
        scope
            .supply(binding, &p(P::Bool), &Term::Bool(true), &mut 100, span)
            .unwrap();
        assert!(earlier.lookup(binding, &mut 100, span).unwrap().is_none());
        assert_eq!(
            scope.lookup(binding, &mut 100, span).unwrap(),
            Some((&p(P::Bool), Some(&Term::Bool(true))))
        );
        assert!(
            scope
                .supply(binding, &p(P::Bool), &Term::Bool(false), &mut 100, span)
                .is_err()
        );
        assert_eq!(
            scope.lookup(binding, &mut 100, span).unwrap(),
            Some((&p(P::Bool), Some(&Term::Bool(true))))
        );
    }

    #[test]
    fn failed_snapshot_and_lookup_preserve_the_parent_scope() {
        let span = Span::default();
        let binding = n::LocalId { owner: 1, index: 0 };
        let input = (binding, p(P::Bool), Term::Bool(true));
        let scope = Scope::default()
            .child(&[input.clone()], &[], &mut 1000, span)
            .unwrap();
        assert!(scope.child(&[input.clone()], &[], &mut 0, span).is_err());
        assert!(
            scope
                .child(&[input.clone(), input], &[], &mut 1000, span)
                .is_err()
        );
        assert!(scope.lookup(binding, &mut 0, span).is_err());
        assert_eq!(
            scope.lookup(binding, &mut 100, span).unwrap(),
            Some((&p(P::Bool), Some(&Term::Bool(true))))
        );
    }
}

#[cfg(test)]
mod invalidation_tests {
    use super::*;
    #[test]
    fn forked_regions_share_unrelated_payloads_and_publish_invalidations_atomically() {
        let span = Span::default();
        let [large, mutable, next] = [1, 2, 3].map(|index| n::LocalId { owner: 7, index });
        let outer = Scope::default()
            .child(
                &[
                    (large, p(P::Str), Term::String("x".repeat(4096).into())),
                    (mutable, p(P::Bool), Term::Bool(true)),
                ],
                &[],
                &mut 20_000,
                span,
            )
            .unwrap();
        let mut branch = outer.clone();
        let roots = BTreeSet::from([mutable]);
        assert!(branch.invalidate(&roots, &mut 1, span).is_err());
        assert_eq!(
            branch.lookup(mutable, &mut 1, span).unwrap().unwrap().1,
            Some(&Term::Bool(true))
        );
        branch
            .supply(next, &p(P::Bool), &Term::Bool(false), &mut 8, span)
            .unwrap();
        let mut shadow = branch.clone();
        shadow.invalidate(&roots, &mut 8, span).unwrap();
        let payload = outer
            .lookup(large, &mut 1, span)
            .unwrap()
            .unwrap()
            .1
            .unwrap();
        for scope in [&branch, &shadow] {
            assert!(std::ptr::eq(
                scope
                    .lookup(large, &mut 1, span)
                    .unwrap()
                    .unwrap()
                    .1
                    .unwrap(),
                payload
            ));
        }
        assert!(outer.lookup(next, &mut 1, span).unwrap().is_none());
        assert_eq!(
            shadow.lookup(mutable, &mut 1, span).unwrap().unwrap().1,
            None
        );
        assert_eq!(
            branch.lookup(mutable, &mut 1, span).unwrap().unwrap().1,
            Some(&Term::Bool(true))
        );
    }

    #[test]
    fn nested_prefix_lookup_and_branch_updates_are_independent_of_scope_depth_and_width() {
        let span = Span::default();
        let id = |index| n::LocalId { owner: 4, index };
        let lookup_and_supply = |width: u32, depth: usize| {
            let mut scope = Scope::default().child(&[], &[], &mut 1, span).unwrap();
            for index in 0..width {
                scope
                    .supply(id(index), &p(P::Bool), &Term::Bool(true), &mut 100, span)
                    .unwrap();
            }
            let parent = scope.clone();
            for _ in 0..depth {
                scope = scope.child(&[], &[], &mut 1, span).unwrap();
            }
            let mut fuel = 100;
            assert_eq!(
                scope.lookup(id(0), &mut fuel, span).unwrap().unwrap().1,
                Some(&Term::Bool(true))
            );
            assert!(scope.lookup(id(width), &mut fuel, span).unwrap().is_none());
            let sibling = scope.clone();
            scope
                .supply(id(width), &p(P::Bool), &Term::Bool(false), &mut fuel, span)
                .unwrap();
            scope.invalidate(&[id(0)].into(), &mut fuel, span).unwrap();
            assert_eq!(
                scope.lookup(id(0), &mut fuel, span).unwrap().unwrap().1,
                None
            );
            // The mask reserves this name locally. An independent child can
            // shadow the inherited binding with exactly the same domain/value.
            assert!(
                scope
                    .supply(id(0), &p(P::Bool), &Term::Bool(true), &mut 100, span)
                    .is_err()
            );
            for saved in [parent, sibling] {
                assert!(saved.lookup(id(width), &mut 1, span).unwrap().is_none());
                assert_eq!(
                    saved.lookup(id(0), &mut 1, span).unwrap().unwrap().1,
                    Some(&Term::Bool(true))
                );
            }
            fuel
        };
        assert_eq!(lookup_and_supply(1, 1), lookup_and_supply(1024, 512));
    }

    #[test]
    fn a_late_budget_failure_discards_masks_already_prepared_in_this_batch() {
        let span = Span::default();
        let keys = [0, 1].map(|index| n::LocalId { owner: 5, index });
        let mut scope = Scope::default()
            .child(
                &keys.map(|id| (id, p(P::Bool), Term::Bool(true))),
                &[],
                &mut 100,
                span,
            )
            .unwrap();
        let before = scope.to_string();
        let mut fuel = 3;
        assert!(scope.invalidate(&keys.into(), &mut fuel, span).is_err());
        assert_eq!(fuel, 0);
        assert_eq!(scope.to_string(), before);
        scope.invalidate(&keys.into(), &mut 4, span).unwrap();
        for key in keys {
            assert_eq!(scope.lookup(key, &mut 1, span).unwrap().unwrap().1, None);
        }
    }

    #[test]
    fn writes_mask_snapshots_without_mutating_saved_outer_regions() {
        let span = Span::default();
        let binding = n::LocalId { owner: 2, index: 0 };
        let other = n::LocalId { owner: 2, index: 1 };
        let scope = Scope::default()
            .child(
                &[
                    (binding, p(P::Bool), Term::Bool(true)),
                    (other, p(P::Bool), Term::Bool(false)),
                ],
                &[],
                &mut 1000,
                span,
            )
            .unwrap();
        let mut current = scope.clone();
        current
            .invalidate(&BTreeSet::from([binding]), &mut 100, span)
            .unwrap();
        assert_eq!(
            current.lookup(binding, &mut 100, span).unwrap(),
            Some((&p(P::Bool), None))
        );
        assert_eq!(
            current.lookup(other, &mut 100, span).unwrap(),
            Some((&p(P::Bool), Some(&Term::Bool(false))))
        );
        assert_eq!(
            scope.lookup(binding, &mut 100, span).unwrap(),
            Some((&p(P::Bool), Some(&Term::Bool(true))))
        );
    }
}
