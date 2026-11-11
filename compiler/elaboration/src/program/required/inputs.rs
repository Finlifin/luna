//! Read-only input evidence owned by an active required preparation region.
//! These are supplied values, never pending producers or live mutable storage.
use super::*;
use std::{fmt, sync::Arc};

#[derive(Clone, Default)]
pub(in crate::program) struct Scope(Option<Arc<Frame>>);

struct Frame {
    parent: Scope,
    values: BTreeMap<n::LocalId, (Term, Option<Term>)>,
}

impl Scope {
    pub(in crate::program) fn child(
        &self,
        inputs: &[(n::LocalId, Term, Term)],
        live: &[(n::LocalId, Term, Value)],
        fuel: &mut u64,
        span: Span,
    ) -> Result<Self, Error> {
        let mut values = BTreeMap::new();
        for (binding, domain, value) in inputs {
            let domain = copy(domain, fuel, span)?;
            let value = copy(value, fuel, span)?;
            if values.insert(*binding, (domain, Some(value))).is_some() {
                return Err(error(span, "duplicate required comptime input"));
            }
        }
        // A live cell shadows any outer snapshot with the same binding. Its
        // current contents are supplied by the invocation, not by observation.
        for (binding, domain, _) in live {
            let domain = copy(domain, fuel, span)?;
            if values.insert(*binding, (domain, None)).is_some() {
                return Err(error(span, "duplicate required comptime input"));
            }
        }
        Ok(Self(Some(Arc::new(Frame {
            parent: self.clone(),
            values,
        }))))
    }

    pub(in crate::program) fn supply(
        &mut self,
        binding: n::LocalId,
        domain: &Term,
        value: &Term,
        fuel: &mut u64,
        span: Span,
    ) -> Result<(), Error> {
        let entry = (copy(domain, fuel, span)?, Some(copy(value, fuel, span)?));
        let current = self.0.as_mut().expect("active required input region");
        if current.values.contains_key(&binding) {
            return Err(error(span, "duplicate required comptime input"));
        }
        if Arc::get_mut(current).is_none() {
            let mut values = BTreeMap::new();
            for (binding, (domain, value)) in &current.values {
                let domain = copy(domain, fuel, span)?;
                let value = value
                    .as_ref()
                    .map(|value| copy(value, fuel, span))
                    .transpose()?;
                values.insert(*binding, (domain, value));
            }
            *current = Arc::new(Frame {
                parent: current.parent.clone(),
                values,
            });
        }
        Arc::get_mut(current)
            .expect("owned required input frame")
            .values
            .insert(binding, entry);
        Ok(())
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
        let mut values = BTreeMap::new();
        for binding in bindings {
            if let Some((domain, Some(_))) = self.lookup(*binding, fuel, span)? {
                values.insert(*binding, (copy(domain, fuel, span)?, None));
            }
        }
        if !values.is_empty() {
            // Replace this frame instead of extending the lexical chain once
            // per write. Depth follows required regions, not statement count.
            let current = self.0.as_ref().expect("invalidated supplied inputs");
            for (binding, (domain, value)) in &current.values {
                if !values.contains_key(binding) {
                    let domain = copy(domain, fuel, span)?;
                    let value = value
                        .as_ref()
                        .map(|value| copy(value, fuel, span))
                        .transpose()?;
                    values.insert(*binding, (domain, value));
                }
            }
            *self = Self(Some(Arc::new(Frame {
                parent: current.parent.clone(),
                values,
            })));
        }
        Ok(())
    }

    pub(in crate::program) fn lookup(
        &self,
        binding: n::LocalId,
        fuel: &mut u64,
        span: Span,
    ) -> Result<Option<(&Term, Option<&Term>)>, Error> {
        let mut scope = self;
        while let Some(frame) = &scope.0 {
            *fuel = fuel
                .checked_sub(1)
                .ok_or_else(|| error(span, "required input observation budget exhausted"))?;
            if let Some((domain, value)) = frame.values.get(&binding) {
                return Ok(Some((domain, value.as_ref())));
            }
            scope = &frame.parent;
        }
        Ok(None)
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
        let mut scope = self;
        while let Some(frame) = &scope.0 {
            f.write_str(" (frame")?;
            for (binding, (domain, value)) in &frame.values {
                write!(f, " (input {binding} (domain {domain}) ")?;
                match value {
                    Some(value) => write!(f, "(supplied {value})")?,
                    None => f.write_str("(unavailable)")?,
                }
                f.write_str(")")?;
            }
            f.write_str(")")?;
            scope = &frame.parent;
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
