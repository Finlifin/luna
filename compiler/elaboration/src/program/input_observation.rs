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
