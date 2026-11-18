//! Bounded, read-only observations of checked declaration inputs. These do not
//! establish stage availability, scope admission, or permission to invoke code.
use super::*;
use normal::observation::{self, Failure};

fn structural_limit(limit: nir::copy_budget::CopyBudgetExhausted, span: Span) -> Error {
    error(
        span,
        match limit {
            nir::copy_budget::CopyBudgetExhausted::Steps => {
                "checked input dependency budget exhausted"
            }
            nir::copy_budget::CopyBudgetExhausted::Depth => {
                "checked input dependency depth exhausted"
            }
        },
    )
}

fn substitution_failure(failure: nir::copy_budget::InputObservationError, span: Span) -> Error {
    match failure {
        nir::copy_budget::InputObservationError::Limit(limit) => structural_limit(limit, span),
        nir::copy_budget::InputObservationError::Scope(reason) => error(span, reason.to_string()),
    }
}

pub(super) fn substitution_term(
    value: &Term,
    steps: &mut u64,
    span: Span,
) -> Result<BTreeMap<usize, Sort>, Error> {
    nir::copy_budget::term_substitution_parameters(value, steps)
        .map_err(|failure| substitution_failure(failure, span))
}

pub(super) fn term_inputs(
    value: &Term,
    steps: &mut u64,
    span: Span,
) -> Result<nir::copy_budget::TermInputs, Error> {
    nir::copy_budget::term_inputs(value, steps)
        .map_err(|failure| substitution_failure(failure, span))
}

pub(super) fn expression(
    value: &n::Expr,
    steps: &mut u64,
    span: Span,
) -> Result<BTreeSet<DeclarationId>, Error> {
    nir::copy_budget::expression_declaration_inputs(value, steps)
        .map_err(|limit| structural_limit(limit, span))
}

pub(super) fn function(
    value: &n::Function,
    steps: &mut u64,
    span: Span,
) -> Result<BTreeSet<DeclarationId>, Error> {
    nir::copy_budget::function_declaration_inputs(value, steps)
        .map_err(|limit| structural_limit(limit, span))
}

pub(super) fn term(
    value: &Term,
    steps: &mut u64,
    span: Span,
) -> Result<BTreeSet<DeclarationId>, Error> {
    let before = usize::try_from(*steps).unwrap_or(usize::MAX);
    let mut fuel = before;
    let mut inputs = BTreeSet::new();
    let result = nir::type_computations::rigid_declarations_bounded(value, &mut fuel, &mut inputs);
    *steps -= (before - fuel) as u64;
    result.map_err(|reason| {
        error(
            span,
            format!("type input dependency observation failed: {reason}"),
        )
    })?;
    Ok(inputs)
}

/// Whether a checked domain still needs any of these declaration inputs.
pub(super) fn uses_any(
    domain: &Term,
    mut candidates: impl ExactSizeIterator<Item = DeclarationId>,
    steps: &mut u64,
    span: Span,
) -> Result<bool, Error> {
    if candidates.len() == 0 {
        return Ok(false);
    }
    let used = term(domain, steps, span)?;
    if used.is_empty() {
        return Ok(false);
    }
    *steps = steps
        .checked_sub(candidates.len() as u64)
        .ok_or_else(|| error(span, "input domain dependency budget exhausted"))?;
    Ok(candidates.any(|id| used.contains(&id)))
}

/// Dependencies are measured in the original callee domain. After substitution,
/// equally numbered caller binders no longer identify unsupplied callee inputs.
/// The result is an input dependency set, not permission to execute a producer.
pub(super) fn pending_domain_inputs(
    domain: &Term,
    inputs: impl IntoIterator<Item = (DeclarationId, bool)>,
    steps: &mut u64,
    span: Span,
) -> Result<BTreeSet<DeclarationId>, Error> {
    let referenced = term(domain, steps, span)?;
    let mut pending = BTreeSet::new();
    for (id, supplied) in inputs {
        *steps = steps
            .checked_sub(1)
            .ok_or_else(|| error(span, "input domain dependency budget exhausted"))?;
        if !supplied && referenced.contains(&id) {
            pending.insert(id);
        }
    }
    Ok(pending)
}

impl Checker<'_, '_> {
    /// Observe identity while retaining the whole producer for execution.
    /// Facts describe earlier bound default inputs and new immutable locals.
    /// Unknown work invalidates facts; it never invokes code to recover them.
    pub(super) fn preserved_input_identity<'value>(
        &mut self,
        producer: &n::Expr,
        initial: impl IntoIterator<Item = (n::LocalId, &'value Term)>,
        span: Span,
    ) -> Result<Option<Term>, Error> {
        // Each new observation may use completed immutable caller bindings.
        // The owned query below invalidates them across unknown effects; never
        // reload these facts after beginning to walk the producer.
        let mut facts = self.observed_expression_bindings(producer, span)?;
        let before = usize::try_from(self.required_steps).unwrap_or(usize::MAX);
        let mut fuel = before;
        let copied = initial.into_iter().try_for_each(|(binding, identity)| {
            nir::type_computations::charge_term(identity, &mut fuel)?;
            facts.insert(binding, std::sync::Arc::new(identity.clone()));
            Ok::<_, nir::type_computations::Error>(())
        });
        self.required_steps -= (before - fuel) as u64;
        copied.map_err(|failure| error(span, failure.to_string()))?;
        let before = usize::try_from(self.required_steps).unwrap_or(usize::MAX);
        let mut budget = comptime_runtime::Budget {
            steps: before,
            max_limbs: 4096,
        };
        let observed = self.observe_preserved_input_in(producer, facts, &mut budget);
        self.required_steps -= (before - budget.steps) as u64;
        observation::optional(observed).map_err(|failure| {
            error(
                span,
                format!("input identity observation failed: {failure}"),
            )
        })
    }

    /// The caller retains the producer. Keep the classified result so a domain
    /// waiter can report why observation failed without treating it as absence.
    /// Ownership of facts prevents one query from publishing prefix mutations
    /// into another query's lexical evidence.
    pub(super) fn observe_preserved_input_in(
        &self,
        producer: &n::Expr,
        mut facts: BTreeMap<n::LocalId, std::sync::Arc<Term>>,
        budget: &mut comptime_runtime::Budget,
    ) -> Result<Term, Failure> {
        if let Some(identity) =
            observation::optional(self.observe_normal_input_in(producer, 0, budget, &facts))?
        {
            return Ok(identity);
        }
        let mut result = producer;
        loop {
            budget.consume(1)?;
            let (statements, tail) = match &result.kind {
                K::Block { statements, result } | K::CallPreparation { statements, result } => {
                    (statements, result)
                }
                _ => {
                    return if input_facts_cover(result, &facts, budget)? {
                        self.observe_normal_input_in(result, 0, budget, &facts)
                    } else {
                        Err(Failure::Unavailable)
                    };
                }
            };
            self.observe_input_prefix_in(statements, &mut facts, budget)?;
            let Some(tail) = tail else {
                return Err(Failure::Unavailable);
            };
            result = tail;
        }
    }

    /// Retain only facts surviving this executable prefix. The caller keeps
    /// every producer; observation does not supply values to an invocation.
    pub(super) fn observe_input_prefix_in(
        &self,
        statements: &[n::Expr],
        facts: &mut BTreeMap<n::LocalId, std::sync::Arc<Term>>,
        budget: &mut comptime_runtime::Budget,
    ) -> Result<(), Failure> {
        for statement in statements {
            if !input_facts_cover(statement, facts, budget)? {
                facts.clear();
                continue;
            }
            let observed = if matches!(statement.kind, K::Let { .. } | K::LetPattern { .. }) {
                observation::optional(self.observe_normal_binding(statement, 0, budget, facts))?
                    .is_some()
            } else {
                observation::optional(self.observe_normal_input_in(statement, 0, budget, facts))?
                    .is_some()
            };
            if !observed {
                // Unknown calls or writes can invalidate aliases as well as
                // their apparent operands. Keep no pre-effect local facts.
                facts.clear();
            }
        }
        Ok(())
    }

    pub(super) fn observed_term_inputs(
        &mut self,
        value: &Term,
        span: Span,
    ) -> Result<nir::copy_budget::TermInputs, Error> {
        term_inputs(value, &mut self.required_steps, span)
    }

    pub(super) fn checked_substitution_inputs(
        &mut self,
        value: &n::Expr,
        span: Span,
    ) -> Result<BTreeMap<usize, Sort>, Error> {
        nir::copy_budget::expression_substitution_parameters(value, &mut self.required_steps)
            .map_err(|failure| substitution_failure(failure, span))
    }

    pub(super) fn term_substitution_inputs(
        &mut self,
        value: &Term,
        span: Span,
    ) -> Result<BTreeMap<usize, Sort>, Error> {
        substitution_term(value, &mut self.required_steps, span)
    }

    pub(super) fn checked_declaration_inputs(
        &mut self,
        value: &n::Expr,
        span: Span,
    ) -> Result<BTreeSet<DeclarationId>, Error> {
        expression(value, &mut self.required_steps, span)
    }

    pub(super) fn term_declaration_inputs(
        &mut self,
        value: &Term,
        span: Span,
    ) -> Result<BTreeSet<DeclarationId>, Error> {
        term(value, &mut self.required_steps, span)
    }
}

fn input_facts_cover(
    expression: &n::Expr,
    facts: &BTreeMap<n::LocalId, std::sync::Arc<Term>>,
    budget: &mut comptime_runtime::Budget,
) -> Result<bool, Failure> {
    let free = nir::free_bindings::collect(expression, &mut budget.steps)?;
    budget.consume(free.len())?;
    Ok(free.iter().all(|binding| facts.contains_key(binding)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(local: u32) -> DeclarationId {
        DeclarationId { package: 1, local }
    }

    fn domain() -> Term {
        Term::OptionalValue {
            ty: Box::new(Term::Optional(Box::new(Term::Rigid {
                declaration: id(1),
                sort: Sort::Type,
            }))),
            value: Some(Box::new(Term::Rigid {
                declaration: id(2),
                sort: Sort::ComptimeValue,
            })),
        }
    }

    #[test]
    fn callable_instance_observation_requires_budget_before_reporting_independence() {
        let span = rustc_span::DUMMY_SP;
        let callable = Term::FunctionValue {
            declaration: id(3),
            arguments: vec![domain()],
        };
        for (candidate, expected) in [(id(1), true), (id(3), false)] {
            assert!(uses_any(&callable, [candidate].into_iter(), &mut 1, span).is_err());
            assert_eq!(
                uses_any(&callable, [candidate].into_iter(), &mut 1000, span).unwrap(),
                expected
            );
        }
        assert!(!uses_any(&callable, [].into_iter(), &mut 0, span).unwrap());
    }

    #[test]
    fn pending_domains_intersect_references_with_unsupplied_callee_inputs() {
        let observed = pending_domain_inputs(
            &domain(),
            [(id(3), false), (id(1), true), (id(2), false)],
            &mut 1000,
            rustc_span::DUMMY_SP,
        )
        .unwrap();
        assert_eq!(observed, [id(2)].into_iter().collect());
        assert!(
            pending_domain_inputs(
                &domain(),
                [(id(3), false), (id(2), true)],
                &mut 1000,
                rustc_span::DUMMY_SP,
            )
            .unwrap()
            .is_empty(),
            "caller input 1 is not part of this callee telescope"
        );
    }

    #[test]
    fn pending_domain_observation_never_turns_budget_failure_into_readiness() {
        let inputs = [(id(1), true), (id(2), false)];
        for budget in [0, 1, 3] {
            let mut remaining = budget;
            assert!(
                pending_domain_inputs(&domain(), inputs, &mut remaining, rustc_span::DUMMY_SP)
                    .is_err()
            );
            assert!(remaining <= budget);
        }
        assert_eq!(
            pending_domain_inputs(&domain(), inputs, &mut 1000, rustc_span::DUMMY_SP).unwrap(),
            [id(2)].into_iter().collect()
        );
    }
}
