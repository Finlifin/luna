//! Comparisons retain operand inference and rollback in the enclosing region.
use super::*;
use driver::{Request, Step};
use operators::peers;

pub(super) fn is_comparison(expr: &Expr<'_>) -> bool {
    matches!(
        expr.kind,
        H::Binary(
            BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge,
            _,
            _
        )
    )
}

pub(super) struct Frame<'h> {
    source: Expr<'h>,
    destination: Destination,
    progress: Option<peers::Progress<'h, 'h>>,
    attempt: Option<checking_attempts::Attempt>,
    preserve_effect: Option<bool>,
}

impl<'h> Frame<'h> {
    pub(super) fn new(
        checker: &mut Checker<'_, '_>,
        source: &Expr<'h>,
        destination: Destination,
        own_effect_delivery: bool,
    ) -> Result<Self, Error> {
        let H::Binary(op, left, right) = source.kind else {
            return Err(error(
                source.span,
                "comparison requires two source operands",
            ));
        };
        if !is_comparison(source) || destination.ordinary_view().is_none() {
            return Err(error(
                source.span,
                "comparison requires an ordinary destination",
            ));
        }
        let fallback = (!matches!(op, BinOp::Eq | BinOp::Ne)
            && is_numeric_literal(left)
            && !is_float_literal(left))
        .then(|| p(P::Integer));
        let progress = peers::Progress::new(checker, [left, right], None, fallback.as_ref(), None)?;
        Ok(Self {
            source: source.clone(),
            destination,
            progress: Some(progress),
            attempt: None,
            preserve_effect: own_effect_delivery
                .then(|| std::mem::take(&mut checker.preserve_effect_value)),
        })
    }

    pub(super) fn step(
        &mut self,
        checker: &mut Checker<'_, '_>,
        returned: Option<CheckedBlock>,
    ) -> Result<Step<'h>, Error> {
        let progress = self
            .progress
            .as_mut()
            .ok_or_else(|| error(self.source.span, "comparison already completed"))?;
        if let Some(value) = returned {
            let value = value.ordinary(self.source.span)?;
            let value = match self.attempt.take() {
                Some(attempt) => attempt.finish(checker, Ok(value))?,
                None => Some(value),
            };
            progress.accept(value)?;
        }
        if let Some(request) = progress.next()? {
            // Capture the destination before entering the attempt. Allocation
            // failure cannot leave a half-entered numeric-default boundary.
            let destination = Destination::ordinary(
                checker,
                OrdinaryTail {
                    expected: request.expected,
                    partial: None,
                    input: None,
                },
                request.input.span,
            )?;
            if request.attempt {
                self.attempt = Some(checking_attempts::Attempt::begin(
                    checker,
                    request.input.span,
                    false,
                    CheckingDisposition::Commit,
                )?);
            }
            return Ok(Step::Child(Request::expression(request.input, destination)));
        }
        let (left, right) = self.progress.take().unwrap().finish()?;
        let mut value = checker.finish_comparison_expression(
            &self.source,
            self.destination.expected(),
            left,
            right,
        )?;
        if self.preserve_effect == Some(false) {
            value =
                checker.run_computation(value, self.destination.expected(), self.source.span)?;
        }
        Ok(Step::Completed(CheckedBlock::Ordinary(value)))
    }

    pub(super) fn recover(
        &mut self,
        checker: &mut Checker<'_, '_>,
        failure: Error,
    ) -> Result<(), Error> {
        let Some(attempt) = self.attempt.take() else {
            return Err(failure);
        };
        let value = attempt.finish::<n::Expr>(checker, Err(failure))?;
        self.progress
            .as_mut()
            .expect("pending operand")
            .accept(value)
    }
}

impl std::fmt::Display for Frame<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "(comparison-frame (source {}) {} (effect-delivery",
            source(self.source.span),
            self.destination
        )?;
        if let Some(preserve) = self.preserve_effect {
            write!(f, " (preserve {preserve})")?;
        }
        f.write_str(") (progress")?;
        if let Some(progress) = &self.progress {
            write!(f, " {progress}")?;
        }
        f.write_str(") (attempt")?;
        if let Some(attempt) = &self.attempt {
            write!(f, " {attempt}")?;
        }
        f.write_str("))")
    }
}
