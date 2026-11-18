//! Declaration-slot observations belong to one call preparation. Only the
//! separate formation proof view needs symbolic stand-ins for pending inputs.
use super::*;

/// Static supplies available while checking a declaration domain. Parent and
/// local slots share one simultaneous substitution into the original domain.
#[derive(Default)]
pub(in crate::program) struct KnownInputs {
    pub(super) ids: Vec<DeclarationId>,
    pub(super) domains: Vec<Term>,
    pub(super) values: Vec<Term>,
}

impl KnownInputs {
    /// Split the selected declaration once. Only the captured values originate
    /// outside this call; both portions retain their original domain templates.
    pub(in crate::program) fn partition(
        ids: &[DeclarationId],
        mut domains: Vec<Term>,
        prefix: &[Term],
        steps: &mut u64,
        span: Span,
    ) -> Result<(Self, Vec<Term>), Error> {
        if ids.len() != domains.len() || prefix.len() > ids.len() {
            return Err(error(span, "clause input prefix arity mismatch"));
        }
        environment::charge(steps, ids.len(), span)?;
        let local = if prefix.is_empty() {
            std::mem::take(&mut domains)
        } else {
            domains.split_off(prefix.len())
        };
        let values = prefix
            .iter()
            .map(|value| environment::copy_term(value, steps, span))
            .collect::<Result<_, _>>()?;
        Ok((
            Self {
                ids: ids[..prefix.len()].to_vec(),
                domains,
                values,
            },
            local,
        ))
    }

    pub(super) fn fmt_prefix(
        &self,
        count: usize,
        f: &mut std::fmt::Formatter<'_>,
    ) -> std::fmt::Result {
        f.write_str("(captured")?;
        for ((id, domain), value) in self
            .ids
            .iter()
            .zip(&self.domains)
            .zip(&self.values)
            .take(count)
        {
            write!(f, " (input {id} {domain} {value})")?;
        }
        f.write_str(")")
    }

    pub(in crate::program) fn into_parts(self) -> (Vec<DeclarationId>, Vec<Term>, Vec<Term>) {
        (self.ids, self.domains, self.values)
    }

    pub(in crate::program) fn values(&self) -> &[Term] {
        &self.values
    }

    /// Check the observed prefix in the current caller frame once per consumer.
    /// A packet or a default body may use it in many separate annotations.
    pub(in crate::program) fn with_substitution<R>(
        &self,
        checker: &mut Checker<'_, '_>,
        span: Span,
        apply: impl FnOnce(
            &nir::DeclarationSubstitution<'_>,
            &mut usize,
        ) -> Result<R, nir::type_computations::Error>,
    ) -> Result<R, Error> {
        let frames = checker.call_input_frames();
        let before = usize::try_from(checker.required_steps).unwrap_or(usize::MAX);
        let mut fuel = before;
        let result = nir::DeclarationSubstitution::bounded(
            &self.ids,
            &self.domains,
            &self.values,
            &frames,
            &mut fuel,
        )
        .and_then(|substitution| apply(&substitution, &mut fuel));
        checker.required_steps -= (before - fuel) as u64;
        result.map_err(|failure| error(span, failure.to_string()))
    }
}
pub(in crate::program) struct Observations(Vec<(nir::DeclarationParameter, Option<Term>)>);

impl Observations {
    pub(in crate::program) fn new(slots: Vec<(nir::DeclarationParameter, Option<Term>)>) -> Self {
        Self(slots)
    }

    pub(in crate::program) fn with_prefix(
        self,
        prefix: impl ExactSizeIterator<Item = (nir::DeclarationParameter, Term)>,
    ) -> Self {
        if prefix.len() == 0 {
            return self;
        }
        Self(
            prefix
                .map(|(binder, value)| (binder, Some(value)))
                .chain(self.0)
                .collect(),
        )
    }

    pub(super) fn identity(&self, slot: usize) -> Option<&Term> {
        self.0[slot].1.as_ref()
    }

    pub(in crate::program) fn into_formation(self) -> FormationInputs {
        let mut deferred = BTreeSet::new();
        let terms = self
            .0
            .into_iter()
            .map(|(binder, value)| match value {
                Some(term) => term,
                None => {
                    deferred.insert(binder.declaration);
                    Term::Rigid {
                        declaration: binder.declaration,
                        sort: binder.sort,
                    }
                }
            })
            .collect();
        FormationInputs {
            terms,
            deferred,
            environment: BTreeMap::new(),
        }
    }

    /// Keep only observed replacements in the callable substitution. Retain
    /// the same observations separately for immediate or queued formation.
    pub(in crate::program) fn bind(
        self,
        domains: &[Term],
        frames: Vec<Vec<Sort>>,
        steps: &mut u64,
        span: Span,
    ) -> Result<(CallableBindings, FormationInputs), Error> {
        environment::charge(steps, self.0.len(), span)?;
        let mut ids = Vec::new();
        let mut values = Vec::new();
        let mut observed_domains = Vec::new();
        let mut proof = Vec::new();
        for ((binder, value), domain) in self.0.into_iter().zip(domains) {
            let retained = if let Some(value) = value {
                let retained = environment::copy_term(&value, steps, span)?;
                ids.push(binder.declaration);
                values.push(value);
                observed_domains.push(environment::copy_term(domain, steps, span)?);
                Some(retained)
            } else {
                None
            };
            proof.push((binder, retained));
        }
        let formation = Self(proof).into_formation();
        let before = usize::try_from(*steps).unwrap_or(usize::MAX);
        let mut fuel = before;
        let substitution = nir::DeclarationSubstitution::bounded_owned(
            &ids,
            &observed_domains,
            values,
            frames,
            &mut fuel,
        );
        *steps -= (before - fuel) as u64;
        let bindings = CallableBindings {
            substitution: substitution.map_err(|reason| error(span, reason.to_string()))?,
            pending: formation.deferred.clone(),
        };
        Ok((bindings, formation))
    }
}

impl std::fmt::Display for Observations {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("(observations")?;
        for (binder, value) in &self.0 {
            write!(f, " (slot {binder} ")?;
            match value {
                Some(term) => write!(f, "(observed {term})")?,
                None => f.write_str("(pending)")?,
            }
            f.write_str(")")?;
        }
        f.write_str(")")
    }
}

pub(in crate::program) struct CallableBindings {
    substitution: nir::DeclarationSubstitution<'static>,
    pending: BTreeSet<DeclarationId>,
}

impl CallableBindings {
    /// Inspect the callee's original domain. Once substituted, the same
    /// declaration ID inside a replacement can refer to the caller instead.
    pub(in crate::program) fn awaits_input(
        &self,
        template: &Term,
        steps: &mut u64,
        span: Span,
    ) -> Result<bool, Error> {
        Ok(!self.pending.is_empty()
            && !self
                .pending
                .is_disjoint(&input_observation::term(template, steps, span)?))
    }

    pub(in crate::program) fn instantiate(
        &self,
        template: &Term,
        steps: &mut u64,
        span: Span,
    ) -> Result<Term, Error> {
        let before = usize::try_from(*steps).unwrap_or(usize::MAX);
        let mut fuel = before;
        let result = self.substitution.instantiate_bounded(template, &mut fuel);
        *steps -= (before - fuel) as u64;
        result.map_err(|failure| error(span, failure.to_string()))
    }

    pub(in crate::program) fn into_identities(self) -> Option<Vec<Term>> {
        self.pending
            .is_empty()
            .then(|| self.substitution.into_arguments().into_owned())
    }
}

impl std::fmt::Display for CallableBindings {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "(callable-bindings {} (pending", self.substitution)?;
        for id in &self.pending {
            write!(f, " {id}")?;
        }
        f.write_str("))")
    }
}
