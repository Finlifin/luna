//! Bounded, read-only observations of checked declaration inputs. These do not
//! establish stage availability, scope admission, or permission to invoke code.
use super::*;

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
        let mut facts = BTreeMap::new();
        let before = usize::try_from(self.required_steps).unwrap_or(usize::MAX);
        let mut fuel = before;
        let copied = initial.into_iter().try_for_each(|(binding, identity)| {
            nir::type_computations::charge_term(identity, &mut fuel)?;
            facts.insert(binding, std::sync::Arc::new(identity.clone()));
            Ok::<_, nir::type_computations::Error>(())
        });
        self.required_steps -= (before - fuel) as u64;
        copied.map_err(|failure| error(span, failure.to_string()))?;
        if let Some(identity) = self.checked_normal_input_in(producer, &facts, span)? {
            return Ok(Some(identity));
        }
        let mut result = producer;
        loop {
            self.required_steps = self
                .required_steps
                .checked_sub(1)
                .ok_or_else(|| error(span, "input result observation budget exhausted"))?;
            let (statements, tail) = match &result.kind {
                K::Block { statements, result } | K::CallPreparation { statements, result } => {
                    (statements, result)
                }
                _ => {
                    return if self.input_facts_cover(result, &facts, span)? {
                        self.checked_normal_input_in(result, &facts, span)
                    } else {
                        Ok(None)
                    };
                }
            };
            for statement in statements {
                if !self.input_facts_cover(statement, &facts, span)? {
                    facts.clear();
                    continue;
                }
                let observed = if matches!(statement.kind, K::Let { .. } | K::LetPattern { .. }) {
                    let before = usize::try_from(self.required_steps).unwrap_or(usize::MAX);
                    let mut budget = comptime_runtime::Budget {
                        steps: before,
                        max_limbs: 4096,
                    };
                    let binding =
                        self.observe_normal_binding(statement, 0, &mut budget, &mut facts);
                    self.required_steps -= (before - budget.steps) as u64;
                    normal::observation::optional(binding)
                        .map_err(|failure| {
                            error(
                                span,
                                format!("static binding observation failed: {failure}"),
                            )
                        })?
                        .is_some()
                } else {
                    self.checked_normal_input_in(statement, &facts, span)?
                        .is_some()
                };
                if !observed {
                    // Unknown calls or writes can invalidate aliases as well as
                    // their apparent operands. Keep no pre-effect local facts.
                    facts.clear();
                }
            }
            let Some(tail) = tail else { return Ok(None) };
            result = tail;
        }
    }

    fn input_facts_cover(
        &mut self,
        expression: &n::Expr,
        facts: &BTreeMap<n::LocalId, std::sync::Arc<Term>>,
        span: Span,
    ) -> Result<bool, Error> {
        let before = usize::try_from(self.required_steps).unwrap_or(usize::MAX);
        let mut fuel = before;
        let free = nir::free_bindings::collect(expression, &mut fuel);
        self.required_steps -= (before - fuel) as u64;
        let free = free.map_err(|failure| error(span, failure.to_string()))?;
        self.required_steps = self
            .required_steps
            .checked_sub(free.len() as u64)
            .ok_or_else(|| error(span, "input fact lookup budget exhausted"))?;
        Ok(free.iter().all(|binding| facts.contains_key(binding)))
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
