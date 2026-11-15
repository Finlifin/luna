//! Dependency checks retain their consumer and child position. Checking a later
//! source input first never changes the execution schedule owned by Progress.
use super::*;

use request::Request;

struct Pending<'b, 'h> {
    slot: usize,
    input: Option<&'b Expr<'h>>,
    expected: Term,
}

pub(super) enum Poll<'b, 'h> {
    Continue,
    Child(Request<'b, 'h>),
    Complete(DeclarationInputDomain),
}

pub(super) struct Frame<'b, 'h> {
    slot: usize,
    span: Span,
    dependencies: Vec<usize>,
    next: usize,
    pending: Option<Pending<'b, 'h>>,
    complete: bool,
}

impl<'b, 'h> Frame<'b, 'h> {
    pub(super) fn new(
        slot: usize,
        parameters: &[(DeclarationId, Sort, Term)],
        fuel: &mut u64,
        span: Span,
    ) -> Result<Self, Error> {
        let Some((_, _, domain)) = parameters.get(slot) else {
            return Err(error(span, "declaration domain slot is out of range"));
        };
        let mut needed = input_observation::term_inputs(domain, fuel, span)?.declarations;
        let mut dependencies = Vec::new();
        for previous in (0..slot).rev() {
            *fuel = fuel
                .checked_sub(1)
                .ok_or_else(|| error(span, "input domain dependency budget exhausted"))?;
            if needed.contains_key(&parameters[previous].0) {
                needed.extend(
                    input_observation::term_inputs(&parameters[previous].2, fuel, span)?
                        .declarations,
                );
                dependencies.push(previous);
            }
        }
        dependencies.reverse();
        Ok(Self {
            slot,
            span,
            dependencies,
            next: 0,
            pending: None,
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
        routed: &mut [Option<RoutedDeclarationInput<'b, 'h>>],
        bindings: &mut BTreeMap<n::LocalId, std::sync::Arc<Term>>,
        progress: &mut progress::Progress,
    ) -> Result<Poll<'b, 'h>, Error> {
        if self.complete || self.pending.is_some() {
            return Err(error(
                self.span,
                "declaration domain check is pending or complete",
            ));
        }
        while let Some(&previous) = self.dependencies.get(self.next) {
            self.next += 1;
            if progress.is_prepared(previous)
                || !matches!(
                    routed[previous],
                    Some(RoutedDeclarationInput::Source(_) | RoutedDeclarationInput::Prepared(_))
                )
                || checker.declaration_domain_pending(
                    &parameters[previous].2,
                    parameters,
                    progress.observed_parameters(),
                    self.span,
                )?
            {
                continue;
            }
            let expected = checker.instantiate_input_types(
                &parameters[previous].2,
                progress.observed_parameters(),
                progress.observed_values(),
                self.span,
            )?;
            let input = routed[previous].take().expect("routed dependency");
            self.pending = Some(Pending {
                slot: previous,
                input: match &input {
                    RoutedDeclarationInput::Source(input) => Some(*input),
                    _ => None,
                },
                expected,
            });
            let expected = &self.pending.as_ref().unwrap().expected;
            return match input {
                RoutedDeclarationInput::Source(input) => Ok(Poll::Child(Request::ordinary(
                    input,
                    Some(checker.copy_alias_type(expected, self.span)?),
                ))),
                RoutedDeclarationInput::Prepared(value) => {
                    let result = checker
                        .require_expression(&value.ty, expected, self.span)
                        .map(|()| n::NormalFormArgument::Value(value));
                    self.accept(checker, bindings, progress, result)?;
                    Ok(Poll::Continue)
                }
                RoutedDeclarationInput::Collected(_) => unreachable!("fixed dependency input"),
            };
        }
        let domain = &parameters[self.slot].2;
        let pending = checker.declaration_domain_pending(
            domain,
            parameters,
            progress.observed_parameters(),
            self.span,
        )?;
        let ty = checker.instantiate_input_types(
            domain,
            progress.observed_parameters(),
            progress.observed_values(),
            self.span,
        )?;
        self.complete = true;
        Ok(Poll::Complete(DeclarationInputDomain { ty, pending }))
    }

    pub(super) fn accept(
        &mut self,
        checker: &mut Checker<'_, '_>,
        bindings: &mut BTreeMap<n::LocalId, std::sync::Arc<Term>>,
        progress: &mut progress::Progress,
        result: Result<n::NormalFormArgument, Error>,
    ) -> Result<(), Error> {
        let pending = self
            .pending
            .take()
            .ok_or_else(|| error(self.span, "declaration domain has no pending child"))?;
        let n::NormalFormArgument::Value(value) = result? else {
            return Err(error(
                self.span,
                "domain dependency requires an ordinary expression",
            ));
        };
        let identity = checker.preserved_input_identity(
            &value,
            bindings
                .iter()
                .map(|(binding, term)| (*binding, term.as_ref())),
            self.span,
        )?;
        let ty = value.ty.clone();
        checker.record_declaration_input(
            progress,
            pending.slot,
            n::NormalFormArgument::Value(value),
            identity,
            ty,
            bindings,
            self.span,
        )
    }
}

impl std::fmt::Display for Frame<'_, '_> {
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
        f.write_str(") (pending")?;
        if let Some(pending) = &self.pending {
            write!(
                f,
                " (slot {}) (expected {})",
                pending.slot, pending.expected
            )?;
            if let Some(input) = pending.input {
                write!(f, " (child {} {})", input.hir_id, source(input.span))?;
            }
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
        assert!(Frame::new(3, &parameters, &mut short, span).is_err());
        assert_eq!(short, 0);
        let mut fuel = 100;
        let mut frame = Frame::new(3, &parameters, &mut fuel, span).unwrap();
        assert!(fuel < 100);
        assert_eq!(frame.dependencies, [0, 2]);
        assert_eq!(frame.slot(), 3);
        let owner = hir::OwnerId::new(hir::LocalDefId::new(1));
        let input = Expr {
            hir_id: hir::HirId::new(owner, hir::ItemLocalId::new(2)),
            span,
            kind: H::Unit,
        };
        frame.next = 1;
        frame.pending = Some(Pending {
            slot: 0,
            input: Some(&input),
            expected: p(P::Type),
        });
        let dump = frame.to_string();
        assert!(dump.contains("(consumer 3)"), "{dump}");
        assert!(dump.contains("(dependencies 0 2)"), "{dump}");
        assert!(
            dump.contains("(pending (slot 0) (expected (primitive Type))"),
            "{dump}"
        );
        assert!(dump.contains(&format!("(child {}", input.hir_id)), "{dump}");
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
        assert!(Frame::new(1, &parameters, &mut 100, Span::default()).is_err());
    }
}
