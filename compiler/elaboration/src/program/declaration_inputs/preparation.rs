//! One input preparation owns routing, checking and caller-scope restoration.
//! Synchronous consumers drive the same child protocol through completion.
use super::*;
use request::Request;

enum Phase<'h> {
    Routing(Box<routing::Frame<'h>>),
    Checking(Box<slots::Frame<'h, 'h>>),
    Finished,
}

pub(in crate::program) enum Poll<'h, T> {
    Continue,
    Child(Request<'h, 'h>),
    Complete(T),
}

#[must_use = "drive input preparation and restore its original caller scope"]
pub(in crate::program) struct Frame<'h, C: Completion> {
    span: Span,
    interface: Option<interface::Interface<'h>>,
    defaults: Option<environment::Environment>,
    completion: Option<C>,
    scope: Option<scope::Scope>,
    phase: Phase<'h>,
    waiting: bool,
}

impl<'h, C: Completion> Frame<'h, C> {
    pub(in crate::program) fn begin<'a: 'h, 'arena: 'h>(
        checker: &mut Checker<'a, 'arena>,
        owner: OwnerId,
        inputs: &[Arg<'h>],
        span: Span,
        parameters: Vec<(DeclarationId, Sort, Term)>,
        prefix: KnownInputs,
        defaults: Option<Environment>,
        completion: C,
    ) -> Result<Self, Error> {
        let clauses = checker.type_clause_params(owner);
        if parameters.len() != clauses.len() {
            return Err(error(span, "declaration input telescope arity mismatch"));
        }
        environment::charge(&mut checker.required_steps, parameters.len(), span)?;
        Self::new(
            checker,
            inputs,
            span,
            parameters,
            prefix,
            interface::Interface::declaration(owner, &clauses),
            defaults,
            completion,
        )
    }

    pub(super) fn positional(
        checker: &mut Checker<'_, '_>,
        inputs: &[Arg<'h>],
        span: Span,
        parameters: Vec<(DeclarationId, Sort, Term)>,
        collector: Option<usize>,
        completion: C,
    ) -> Result<Self, Error> {
        environment::charge(&mut checker.required_steps, parameters.len(), span)?;
        let interface = interface::Interface::positional(parameters.len(), collector, span)?;
        Self::new(
            checker,
            inputs,
            span,
            parameters,
            KnownInputs::default(),
            interface,
            None,
            completion,
        )
    }

    fn new(
        checker: &mut Checker<'_, '_>,
        inputs: &[Arg<'h>],
        span: Span,
        parameters: Parameters,
        prefix: KnownInputs,
        interface: interface::Interface<'h>,
        defaults: Option<environment::Environment>,
        completion: C,
    ) -> Result<Self, Error> {
        let scope = scope::Scope::enter(checker, span)?;
        let routing = match routing::Frame::new(
            checker,
            inputs,
            span,
            parameters,
            prefix,
            interface.entries().to_vec(),
        ) {
            Ok(routing) => routing,
            Err(failure) => {
                scope.leave(checker);
                return Err(failure);
            }
        };
        Ok(Self {
            span,
            interface: Some(interface),
            defaults,
            completion: Some(completion),
            scope: Some(scope),
            phase: Phase::Routing(Box::new(routing)),
            waiting: false,
        })
    }

    pub(in crate::program) fn poll(
        &mut self,
        checker: &mut Checker<'_, '_>,
    ) -> Result<Poll<'h, C::Output>, Error> {
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
                        self.interface.take().expect("routing input interface"),
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
                    let (telescope, inputs) = (*frame).finish()?;
                    // Formation and result conversion belong to the caller,
                    // never to the last default provider or input observation.
                    self.leave(checker);
                    return self
                        .completion
                        .take()
                        .expect("input completion is consumed once")
                        .finish(checker, telescope, inputs, self.span)
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

    pub(in crate::program) fn accept(
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

    pub(in crate::program) fn reject(
        &mut self,
        checker: &mut Checker<'_, '_>,
        failure: Error,
    ) -> Result<(), Error> {
        if self.waiting {
            self.accept(checker, Err(failure))
        } else {
            Err(failure)
        }
    }

    pub(in crate::program) fn leave(&mut self, checker: &mut Checker<'_, '_>) {
        if let Phase::Checking(frame) = &mut self.phase {
            frame.unwind(checker);
        }
        if let Some(scope) = self.scope.take() {
            scope.leave(checker);
        }
    }
}

impl<C: Completion> std::fmt::Display for Frame<'_, C> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "(declaration-input-preparation (source {}) (waiting {}) (phase ",
            source(self.span),
            self.waiting
        )?;
        match &self.phase {
            Phase::Routing(frame) => frame.fmt(f)?,
            Phase::Checking(frame) => frame.fmt(f)?,
            Phase::Finished => f.write_str("finished")?,
        }
        f.write_str(") (interface")?;
        if let Some(interface) = &self.interface {
            write!(f, " {interface}")?;
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
