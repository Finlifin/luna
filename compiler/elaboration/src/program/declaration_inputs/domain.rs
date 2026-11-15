//! Dependency checks retain their consumer and child position. Checking a later
//! source input first never changes the execution schedule owned by Progress.
use super::*;

pub(super) enum Poll {
    Input {
        slot: usize,
        domain: DeclarationInputDomain,
    },
    Complete(DeclarationInputDomain),
}

pub(super) struct Frame {
    slot: usize,
    span: Span,
    dependencies: Vec<usize>,
    next: usize,
    complete: bool,
}

impl Frame {
    pub(super) fn new(
        slot: usize,
        parameters: &[(DeclarationId, Sort, Term)],
        uses_default: impl Fn(usize) -> bool,
        fuel: &mut u64,
        span: Span,
    ) -> Result<Self, Error> {
        let Some((_, _, domain)) = parameters.get(slot) else {
            return Err(error(span, "declaration domain slot is out of range"));
        };
        let mut needed = input_observation::term_inputs(domain, fuel, span)?.declarations;
        let mut dependencies = Vec::new();
        let mut prefix = 0;
        for previous in (0..slot).rev() {
            *fuel = fuel
                .checked_sub(1)
                .ok_or_else(|| error(span, "input domain dependency budget exhausted"))?;
            if previous < prefix || needed.contains_key(&parameters[previous].0) {
                needed.extend(
                    input_observation::term_inputs(&parameters[previous].2, fuel, span)?
                        .declarations,
                );
                // Default providers bind their preceding declaration telescope.
                // Prepare that prefix, including its own dependencies, without
                // moving any producer in the execution schedule.
                if uses_default(previous) {
                    prefix = prefix.max(previous);
                }
                dependencies.push(previous);
            }
        }
        dependencies.reverse();
        Ok(Self {
            slot,
            span,
            dependencies,
            next: 0,
            complete: false,
        })
    }

    pub(super) fn slot(&self) -> usize {
        self.slot
    }

    pub(super) fn poll(
        &mut self,
        checker: &mut Checker<'_, '_>,
        parameters: &[(DeclarationId, Sort, Term)],
        progress: &progress::Progress,
    ) -> Result<Poll, Error> {
        if self.complete {
            return Err(error(self.span, "declaration domain check is complete"));
        }
        while let Some(&previous) = self.dependencies.get(self.next) {
            self.next += 1;
            if progress.is_prepared(previous) {
                continue;
            }
            return Ok(Poll::Input {
                slot: previous,
                domain: Self::domain(checker, parameters, progress, previous, self.span)?,
            });
        }
        let domain = Self::domain(checker, parameters, progress, self.slot, self.span)?;
        self.complete = true;
        Ok(Poll::Complete(domain))
    }

    fn domain(
        checker: &mut Checker<'_, '_>,
        parameters: &[(DeclarationId, Sort, Term)],
        progress: &progress::Progress,
        slot: usize,
        span: Span,
    ) -> Result<DeclarationInputDomain, Error> {
        let domain = &parameters[slot].2;
        let pending = checker.declaration_domain_pending(
            domain,
            parameters,
            progress.observed_parameters(),
            span,
        )?;
        let ty = checker.instantiate_input_types(
            domain,
            progress.observed_parameters(),
            progress.observed_values(),
            span,
        )?;
        Ok(DeclarationInputDomain { ty, pending })
    }
}

impl std::fmt::Display for Frame {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "(declaration-domain-frame (consumer {}) (source {}) (next {}) (complete {}) (dependencies",
            self.slot,
            source(self.span),
            self.next,
            self.complete
        )?;
        for dependency in &self.dependencies {
            write!(f, " {dependency}")?;
        }
        f.write_str("))")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(local: u32) -> DeclarationId {
        DeclarationId { package: 7, local }
    }

    fn dependency(local: u32) -> Term {
        Term::Rigid {
            declaration: id(local),
            sort: Sort::Type,
        }
    }

    #[test]
    fn domain_dependencies_are_transitive_bounded_and_keep_the_consumer() {
        let parameters = [
            (id(0), Sort::Type, p(P::Type)),
            (id(1), Sort::ComptimeValue, p(P::Bool)),
            (id(2), Sort::ComptimeValue, dependency(0)),
            (id(3), Sort::ComptimeValue, dependency(2)),
        ];
        let span = Span::default();
        let mut short = 1;
        assert!(Frame::new(3, &parameters, |_| false, &mut short, span).is_err());
        assert_eq!(short, 0);
        let mut fuel = 100;
        let mut frame = Frame::new(3, &parameters, |_| false, &mut fuel, span).unwrap();
        assert!(fuel < 100);
        assert_eq!(frame.dependencies, [0, 2]);
        assert_eq!(frame.slot(), 3);
        frame.next = 1;
        let dump = frame.to_string();
        assert!(dump.contains("(consumer 3)"), "{dump}");
        assert!(dump.contains("(dependencies 0 2)"), "{dump}");
        assert!(dump.contains("(next 1)"), "{dump}");
    }

    #[test]
    fn default_dependencies_prepare_prefixes_without_adding_unrelated_suffixes() {
        let parameters = [
            (id(0), Sort::Type, p(P::Type)),
            (id(1), Sort::ComptimeValue, p(P::Bool)),
            (id(2), Sort::Type, p(P::Type)),
            (id(3), Sort::ComptimeValue, p(P::Bool)),
            (id(4), Sort::ComptimeValue, dependency(2)),
        ];
        let frame =
            Frame::new(4, &parameters, |slot| slot == 2, &mut 100, Span::default()).unwrap();
        assert_eq!(frame.dependencies, [0, 1, 2]);
        let overridden = Frame::new(4, &parameters, |_| false, &mut 100, Span::default()).unwrap();
        assert_eq!(overridden.dependencies, [2]);
    }

    #[test]
    fn domain_dependency_observation_rejects_conflicting_input_sorts() {
        let domain = Term::TupleValue(vec![
            dependency(0),
            Term::Rigid {
                declaration: id(0),
                sort: Sort::ComptimeValue,
            },
        ]);
        let parameters = [(id(0), Sort::Type, p(P::Type)), (id(1), Sort::Type, domain)];
        assert!(Frame::new(1, &parameters, |_| false, &mut 100, Span::default()).is_err());
    }
}
