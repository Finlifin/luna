//! Shared child routing for retained declaration applications. Completion owns
//! the distinction between opaque heads and ordinary callable instances.
use super::*;
use declaration_inputs::{Completion, preparation};
use driver::{Request, Step};

/// Both opaque-head and ordinary callable inputs share this region adapter.
/// Their completion rules remain separate; routing and child checks do not.
pub(super) struct Frame<'h, C: Completion<Output = n::Expr>> {
    span: Span,
    preparation: preparation::Frame<'h, C>,
}

impl<'h, C: Completion<Output = n::Expr>> Frame<'h, C> {
    pub(super) fn from_preparation(span: Span, preparation: preparation::Frame<'h, C>) -> Self {
        Self { span, preparation }
    }

    pub(super) fn step(
        &mut self,
        checker: &mut Checker<'_, '_>,
        returned: Option<CheckedBlock>,
    ) -> Result<Step<'h, 'h>, Error> {
        if let Some(value) = returned {
            let value = match value {
                CheckedBlock::Ordinary(value) => n::NormalFormArgument::Value(value),
                CheckedBlock::Contextual(value) => value,
            };
            self.preparation.accept(checker, Ok(value))?;
        }
        match self.preparation.poll(checker)? {
            preparation::Poll::Continue => Ok(Step::Continue),
            preparation::Poll::Complete(value) => {
                Ok(Step::Completed(CheckedBlock::Ordinary(value)))
            }
            preparation::Poll::Child(request) => {
                use declaration_inputs::request::Demand;
                let (input, demand) = request.into_parts();
                let destination = match demand {
                    Demand::Ordinary(expected) => Destination::ordinary(
                        checker,
                        OrdinaryTail {
                            expected: expected.as_ref(),
                            partial: None,
                            input: None,
                        },
                        input.span,
                    )?,
                    Demand::Input(context) => Destination::ordinary(
                        checker,
                        OrdinaryTail {
                            expected: None,
                            partial: None,
                            input: Some(&context),
                        },
                        input.span,
                    )?,
                    // The old request enters dependent_normal_argument at depth 0.
                    Demand::Contextual => Destination::ContextualInput,
                };
                Ok(Step::Child(Request::Expression(input, destination)))
            }
        }
    }

    pub(super) fn recover(
        &mut self,
        checker: &mut Checker<'_, '_>,
        failure: Error,
    ) -> Result<(), Error> {
        // Collection probes own their failure recovery inside input preparation.
        self.preparation.reject(checker, failure)
    }

    pub(super) fn unwind(mut self, checker: &mut Checker<'_, '_>) {
        self.preparation.leave(checker);
    }
}

impl<C: Completion<Output = n::Expr>> std::fmt::Display for Frame<'_, C> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "(declaration-application-frame (source {}) {})",
            source(self.span),
            self.preparation
        )
    }
}
