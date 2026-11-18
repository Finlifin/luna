//! Input preparations own the request and the consumer while the region checks
//! each expression. Ordinary dependency inputs also own their staged boundary.
use super::*;
use declaration_inputs::{Completion, preparation};
use driver::{Poll, Request, Root, Step};

impl<'h, C: Completion> Root<'h, 'h> for preparation::Frame<'h, C> {
    type Output = C::Output;

    fn advance(
        &mut self,
        checker: &mut Checker<'_, '_>,
        returned: Option<CheckedBlock>,
    ) -> Result<Poll<Request<'h, 'h>, C::Output>, Error> {
        if let Some(value) = returned {
            let value = match value {
                CheckedBlock::Ordinary(value) => n::NormalFormArgument::Value(value),
                CheckedBlock::Contextual(value) => value,
            };
            self.accept(checker, Ok(value))?;
        }
        match self.poll(checker)? {
            preparation::Poll::Continue => Ok(Poll::Continue),
            preparation::Poll::Complete(value) => Ok(Poll::Complete(value)),
            preparation::Poll::Child(request) => {
                use declaration_inputs::request::Demand;
                let (input, demand) = request.into_parts();
                let destination = match demand {
                    Demand::Ordinary(expected) => Destination::from_expected(expected),
                    Demand::Input(context) => Destination::from_input(context),
                    Demand::Contextual => Destination::ContextualInput,
                };
                Ok(Poll::Child(Request::Expression(input, destination)))
            }
        }
    }

    fn recover(&mut self, checker: &mut Checker<'_, '_>, failure: Error) -> Result<(), Error> {
        // Collection probes own their failure recovery inside input preparation.
        self.reject(checker, failure)
    }

    fn unwind(&mut self, checker: &mut Checker<'_, '_>) {
        self.leave(checker);
    }
}

impl<'h, C: Completion<Output = n::Expr>> preparation::Frame<'h, C> {
    pub(super) fn step(
        &mut self,
        checker: &mut Checker<'_, '_>,
        returned: Option<CheckedBlock>,
    ) -> Result<Step<'h, 'h>, Error> {
        Ok(match self.advance(checker, returned)? {
            Poll::Continue => Step::Continue,
            Poll::Child(request) => Step::Child(request),
            Poll::Complete(value) => Step::Completed(CheckedBlock::Ordinary(value)),
        })
    }
}

impl<'b, 'h: 'b> Root<'b, 'b> for dependent_inputs::tasks::Frame<'b, 'h> {
    type Output = dependent_inputs::PreparedInputs;

    fn advance(
        &mut self,
        checker: &mut Checker<'_, '_>,
        returned: Option<CheckedBlock>,
    ) -> Result<Poll<Request<'b, 'b>, Self::Output>, Error> {
        if let Some(returned) = returned {
            let CheckedBlock::Ordinary(value) = returned else {
                unreachable!("dependent input requested an ordinary expression")
            };
            self.accept(checker, value)?;
        }
        use dependent_inputs::tasks::Poll as Input;
        match self.poll(checker)? {
            Input::Continue => Ok(Poll::Continue),
            Input::Child(child) => Ok(Poll::Child(Request::Expression(
                child.input,
                Destination::from_expected(Some(child.expected)),
            ))),
            Input::Complete(inputs) => Ok(Poll::Complete(inputs)),
        }
    }

    fn unwind(&mut self, checker: &mut Checker<'_, '_>) {
        self.unwind(checker)
    }
}
