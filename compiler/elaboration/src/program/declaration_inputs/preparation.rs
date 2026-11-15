//! One input preparation owns routing, checking and caller-scope restoration.
//! Synchronous consumers drive the same child protocol through completion.
use super::*;
use request::Request;

enum Phase<'h> {
    Routing(Box<routing::Frame<'h>>),
    Checking(Box<slots::Frame<'h, 'h>>),
    Finished,
}

enum Poll<'h, T> {
    Continue,
    Child(Request<'h, 'h>),
    Complete(T),
}

#[must_use = "drive input preparation and restore its original caller scope"]
pub(super) struct Frame<'h, C: Completion> {
    owner: OwnerId,
    span: Span,
    clauses: Option<Vec<hir::clause::ClauseParam<'h>>>,
    defaults: Option<environment::Environment>,
    parameters: Parameters,
    completion: Option<C>,
    scope: Option<scope::Scope>,
    phase: Phase<'h>,
    waiting: bool,
}

impl<'h, C: Completion> Frame<'h, C> {
    pub(super) fn new(
        checker: &mut Checker<'_, '_>,
        owner: OwnerId,
        inputs: &[Arg<'h>],
        span: Span,
        parameters: Parameters,
        clauses: Vec<hir::clause::ClauseParam<'h>>,
        defaults: Option<environment::Environment>,
        completion: C,
    ) -> Result<Self, Error> {
        let scope = scope::Scope::enter(checker, span)?;
        let routing =
            match routing::Frame::new(checker, owner, inputs, span, parameters.clone(), &clauses) {
                Ok(routing) => routing,
                Err(failure) => {
                    scope.leave(checker);
                    return Err(failure);
                }
            };
        Ok(Self {
            owner,
            span,
            clauses: Some(clauses),
            defaults,
            parameters,
            completion: Some(completion),
            scope: Some(scope),
            phase: Phase::Routing(Box::new(routing)),
            waiting: false,
        })
    }

    fn poll(&mut self, checker: &mut Checker<'_, '_>) -> Result<Poll<'h, C::Output>, Error> {
        if self.waiting {
            return Err(error(
                self.span,
                "declaration input preparation awaits its child",
            ));
        }
        let child = match &mut self.phase {
            Phase::Routing(frame) => match frame.poll(checker)? {
                routing::Poll::Continue => return Ok(Poll::Continue),
                routing::Poll::Child(request) => request,
                routing::Poll::Complete => {
                    let Phase::Routing(frame) = std::mem::replace(&mut self.phase, Phase::Finished)
                    else {
                        unreachable!()
                    };
                    let frame = (*frame).finish(
                        checker,
                        self.owner,
                        self.clauses.take().expect("routing clause telescope"),
                        self.defaults.take(),
                        C::CHECKING,
                    )?;
                    self.phase = Phase::Checking(Box::new(frame));
                    return Ok(Poll::Continue);
                }
            },
            Phase::Checking(frame) => match frame.poll(checker)? {
                slots::Poll::Continue => return Ok(Poll::Continue),
                slots::Poll::Child(request) => request,
                slots::Poll::Complete => {
                    let Phase::Checking(mut frame) =
                        std::mem::replace(&mut self.phase, Phase::Finished)
                    else {
                        unreachable!()
                    };
                    frame.unwind(checker);
                    let inputs = (*frame).finish()?;
                    // Formation and result conversion belong to the caller,
                    // never to the last default provider or input observation.
                    self.leave(checker);
                    return self
                        .completion
                        .take()
                        .expect("input completion is consumed once")
                        .finish(checker, &self.parameters, inputs, self.span)
                        .map(Poll::Complete);
                }
            },
            Phase::Finished => {
                return Err(error(
                    self.span,
                    "declaration input preparation is complete",
                ));
            }
        };
        self.waiting = true;
        Ok(Poll::Child(child))
    }

    fn accept(
        &mut self,
        checker: &mut Checker<'_, '_>,
        result: Result<n::NormalFormArgument, Error>,
    ) -> Result<(), Error> {
        if !std::mem::take(&mut self.waiting) {
            return Err(error(
                self.span,
                "declaration input preparation has no pending child",
            ));
        }
        match &mut self.phase {
            Phase::Routing(frame) => frame.accept(checker, result),
            Phase::Checking(frame) => frame.accept_child(checker, result),
            Phase::Finished => Err(error(
                self.span,
                "declaration input preparation is complete",
            )),
        }
    }

    fn leave(&mut self, checker: &mut Checker<'_, '_>) {
        if let Phase::Checking(frame) = &mut self.phase {
            frame.unwind(checker);
        }
        if let Some(scope) = self.scope.take() {
            scope.leave(checker);
        }
    }

    pub(super) fn drive(mut self, checker: &mut Checker<'_, '_>) -> Result<C::Output, Error> {
        let result = (|| loop {
            match self.poll(checker)? {
                Poll::Continue => {}
                Poll::Child(request) => {
                    let result = request.check(checker);
                    self.accept(checker, result)?;
                }
                Poll::Complete(inputs) => break Ok(inputs),
            }
        })();
        self.leave(checker);
        result
    }
}

impl<C: Completion> std::fmt::Display for Frame<'_, C> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "(declaration-input-preparation (owner {}) (source {}) (waiting {}) (phase ",
            self.owner,
            source(self.span),
            self.waiting
        )?;
        match &self.phase {
            Phase::Routing(frame) => frame.fmt(f)?,
            Phase::Checking(frame) => frame.fmt(f)?,
            Phase::Finished => f.write_str("finished")?,
        }
        f.write_str(") (caller")?;
        if let Some(scope) = &self.scope {
            write!(f, " {scope}")?;
        }
        f.write_str(") (default-provider")?;
        if let Some(defaults) = &self.defaults {
            write!(f, " {defaults}")?;
        }
        f.write_str(") (completion")?;
        if let Some(completion) = &self.completion {
            write!(f, " {completion}")?;
        }
        f.write_str("))")
    }
}
