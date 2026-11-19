//! Input preparations own the request and the consumer while the region checks
//! each expression. Ordinary dependency inputs also own their staged boundary.
use super::*;
use declaration_inputs::{Completion, preparation};
use driver::{Poll, Request, Root, Step};

impl<'h, C: Completion> Root<'h> for preparation::Frame<'h, C> {
    type Output = C::Output;
    type Invocation = std::convert::Infallible;
    fn resume(
        &mut self,
        _: &mut Checker<'_, '_>,
        reply: std::convert::Infallible,
    ) -> Result<(), Error> {
        match reply {}
    }

    fn advance(
        &mut self,
        checker: &mut Checker<'_, '_>,
        returned: Option<CheckedBlock>,
    ) -> Result<Poll<Request<'h>, C::Output>, Error> {
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
                Ok(Poll::Child(Request::expression(input, destination)))
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
    ) -> Result<Step<'h>, Error> {
        Ok(match self.advance(checker, returned)? {
            Poll::Continue => Step::Continue,
            Poll::Child(request) => Step::Child(request),
            Poll::Complete(value) => Step::Completed(CheckedBlock::Ordinary(value)),
        })
    }
}

pub(super) struct Dependencies<'s, 'h> {
    pub(super) site: &'s dependent_inputs::CallSite<'h>,
    pub(super) frame: dependent_inputs::tasks::Frame,
}

impl std::fmt::Display for Dependencies<'_, '_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "(call-dependencies {} {})", self.site, self.frame)
    }
}

impl<'h> Root<'h> for Dependencies<'_, 'h> {
    type Output = dependent_inputs::PreparedInputs;
    type Invocation = required::Invocation;
    fn resume(
        &mut self,
        checker: &mut Checker<'_, '_>,
        reply: required::CompletedInvocation,
    ) -> Result<(), Error> {
        self.frame.accept_required(checker, reply)
    }
    fn advance(
        &mut self,
        checker: &mut Checker<'_, '_>,
        returned: Option<CheckedBlock>,
    ) -> Result<Poll<Request<'h>, Self::Output, Self::Invocation>, Error> {
        self.frame.advance(checker, self.site, returned)
    }
    fn unwind(&mut self, checker: &mut Checker<'_, '_>) {
        self.frame.unwind(checker)
    }
}

impl dependent_inputs::tasks::Frame {
    pub(super) fn advance<'h>(
        &mut self,
        checker: &mut Checker<'_, '_>,
        site: &dependent_inputs::CallSite<'h>,
        returned: Option<CheckedBlock>,
    ) -> Result<Poll<Request<'h>, dependent_inputs::PreparedInputs, required::Invocation>, Error>
    {
        let next = if let Some(returned) = returned {
            let CheckedBlock::Ordinary(value) = returned else {
                unreachable!("dependent input requested an ordinary expression")
            };
            self.accept(checker, value, site)?
        } else {
            self.poll(checker, site)?
        };
        use dependent_inputs::tasks::Poll as Input;
        match next {
            Input::Invoke(work) => Ok(Poll::Invoke(work)),
            Input::Continue => Ok(Poll::Continue),
            Input::Child(child) => Ok(Poll::Child(Request::routed(child.input, child.expected))),
            Input::Complete(inputs) => Ok(Poll::Complete(inputs)),
        }
    }
}

impl driver::Execution for required::Invocation {
    type Reply = required::CompletedInvocation;
    fn execute(self, checker: &mut Checker<'_, '_>) -> Result<Self::Reply, Error> {
        self.execute(checker)
    }
}
