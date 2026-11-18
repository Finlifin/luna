//! Active source-checking work survives each child handoff as owned state.
//! This internal driver still borrows the original HIR and uses the same active
//! Checker. It is not a transportable task or an execution/Type response API.
use super::*;
use root::ExpressionResult;

pub(super) struct Child<'b, 'h> {
    pub(super) expr: &'b Expr<'h>,
    pub(super) destination: Destination,
}

impl std::fmt::Display for Child<'_, '_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "(child {} (source {}) {})",
            self.expr.hir_id,
            source(self.expr.span),
            self.destination
        )
    }
}

enum State {
    Ready(Option<CheckedBlock>),
    Waiting(Span),
    Failed(Error),
    Finished,
}

#[must_use = "drive this region to completion in its active checker"]
pub(super) struct Region<'b, 'h, R = ExpressionResult> {
    root: R,
    frames: Vec<RegionFrame<'b, 'h>>,
    state: State,
}

impl<'b, 'h> Region<'b, 'h> {
    pub(super) fn new(root: RegionFrame<'b, 'h>) -> Self {
        Self {
            root: ExpressionResult,
            frames: vec![root],
            state: State::Ready(None),
        }
    }
}

impl<'b, 'h, R: Root<'b, 'h>> Region<'b, 'h, R> {
    pub(super) fn with_consumer(root: R) -> Self {
        Self {
            root,
            frames: vec![],
            state: State::Ready(None),
        }
    }

    pub(super) fn accept(
        &mut self,
        result: Result<CheckedBlock, Error>,
        span: Span,
    ) -> Result<(), Error> {
        if !matches!(self.state, State::Waiting(_)) {
            return Err(error(span, "checking region has no pending child"));
        }
        self.state = match result {
            Ok(value) => State::Ready(Some(value)),
            Err(failure) => State::Failed(failure),
        };
        Ok(())
    }

    pub(super) fn poll<'a: 'h, 'arena: 'h>(
        &mut self,
        checker: &mut Checker<'a, 'arena>,
    ) -> Result<Poll<Child<'b, 'h>, R::Output>, Error> {
        match self.state {
            State::Waiting(span) => {
                return Err(error(span, "checking region is awaiting its child"));
            }
            State::Finished => return Err(error(Span::default(), "checking region is complete")),
            _ => {}
        }
        let state = std::mem::replace(&mut self.state, State::Ready(None));
        let result = (|| {
            let returned = match state {
                State::Ready(value) => value,
                State::Failed(failure) => return Err(failure),
                _ => unreachable!("only runnable regions advance"),
            };
            let request = if let Some(frame) = self.frames.last_mut() {
                let step = match frame {
                    RegionFrame::Block(frame) => {
                        frame.step(checker, returned, &Destination::statement())?
                    }
                    RegionFrame::Aggregate(frame) => frame.step(checker, returned)?,
                    RegionFrame::Conditional(frame) => frame.step(checker, returned)?,
                    RegionFrame::When(frame) => frame.step(checker, returned)?,
                    RegionFrame::Match(frame) => frame.step(checker, returned)?,
                    RegionFrame::Comparison(frame) => frame.step(checker, returned)?,
                    RegionFrame::Arithmetic(frame) => frame.step(checker, returned)?,
                    RegionFrame::Unary(frame) => frame.step(checker, returned)?,
                    RegionFrame::Logical(frame) => frame.step(checker, returned)?,
                    RegionFrame::NormalApplication(frame) => frame.step(checker, returned)?,
                    RegionFrame::CallableApplication(frame) => frame.step(checker, returned)?,
                };
                match step {
                    Step::Continue => return Ok(Poll::Continue),
                    Step::Tail(tail) => {
                        let RegionFrame::Block(frame) = self.frames.pop().unwrap() else {
                            unreachable!("only blocks complete a tail")
                        };
                        let value = frame.finish(checker, tail)?;
                        self.state = State::Ready(Some(value));
                        return Ok(Poll::Continue);
                    }
                    Step::Completed(value) => {
                        // The owning frame restores its remaining branch/region boundaries.
                        self.frames.pop().unwrap().unwind(checker);
                        self.state = State::Ready(Some(value));
                        return Ok(Poll::Continue);
                    }
                    Step::Child(request) => request,
                }
            } else {
                match self.root.advance(checker, returned)? {
                    Poll::Continue => return Ok(Poll::Continue),
                    Poll::Complete(value) => return Ok(Poll::Complete(value)),
                    Poll::Child(request) => request,
                }
            };
            match children::enter(&mut self.frames, checker, request)? {
                Some(child) => {
                    self.state = State::Waiting(child.expr.span);
                    Ok(Poll::Child(child))
                }
                None => Ok(Poll::Continue),
            }
        })();
        match result {
            Ok(Poll::Complete(value)) => {
                self.state = State::Finished;
                Ok(Poll::Complete(value))
            }
            Ok(progress) => Ok(progress),
            Err(failure) => {
                let recovered = recover(&mut self.frames, checker, failure)
                    .or_else(|failure| self.root.recover(checker, failure));
                match recovered {
                    Ok(()) => {
                        self.state = State::Ready(None);
                        Ok(Poll::Continue)
                    }
                    Err(failure) => {
                        self.root.unwind(checker);
                        self.state = State::Finished;
                        Err(failure)
                    }
                }
            }
        }
    }
}

fn recover(
    frames: &mut Vec<RegionFrame<'_, '_>>,
    checker: &mut Checker<'_, '_>,
    mut failure: Error,
) -> Result<(), Error> {
    // Lexical exits must run before their enclosing checkpoint.
    // Only a recoverable context request resumes a branch search.
    while let Some(mut frame) = frames.pop() {
        let recovered = match &mut frame {
            RegionFrame::Conditional(conditional) => conditional.recover_probe(checker, failure),
            RegionFrame::When(frame) => frame.recover_probe(checker, failure),
            RegionFrame::Match(frame) => frame.recover_probe(checker, failure),
            RegionFrame::Comparison(comparison) => comparison.recover(checker, failure),
            RegionFrame::Unary(frame) => frame.recover(checker, failure),
            RegionFrame::Arithmetic(frame) => frame.recover(checker, failure),
            RegionFrame::Logical(frame) => frame.recover(checker, failure),
            RegionFrame::NormalApplication(frame) => frame.recover(checker, failure),
            RegionFrame::CallableApplication(frame) => frame.recover(checker, failure),
            _ => Err(failure),
        };
        match recovered {
            Ok(()) => {
                frames.push(frame);
                return Ok(());
            }
            Err(error) => failure = error,
        }
        frame.unwind(checker);
    }
    Err(failure)
}

impl<R: std::fmt::Display> std::fmt::Display for Region<'_, '_, R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("(checking-region (state ")?;
        match &self.state {
            State::Ready(None) => f.write_str("ready")?,
            State::Ready(Some(value)) => write!(f, "(returned {value})")?,
            State::Waiting(span) => write!(f, "(waiting-child (source {}))", source(*span))?,
            State::Failed(failure) => write!(
                f,
                "(failed (source {}) (needs-context {}) (message {:?}))",
                failure.source, failure.needs_context, failure.message
            )?,
            State::Finished => f.write_str("finished")?,
        }
        write!(f, ") (root {}) (frames", self.root)?;
        for frame in &self.frames {
            write!(f, " {frame}")?;
        }
        f.write_str("))")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duplicate_delivery_preserves_the_first_result_or_failure() {
        // Only the child handoff creates Waiting in production. Exercise its
        // ownership rule independently of expression checking and VM execution.
        for fails in [false, true] {
            let mut region = Region {
                root: ExpressionResult,
                frames: vec![],
                state: State::Waiting(Span::default()),
            };
            let result = if fails {
                let mut failure = error(Span::default(), "need \"type\"\nnext");
                failure.needs_context = true;
                Err(failure)
            } else {
                Ok(CheckedBlock::Ordinary(n::Expr {
                    source: source(Span::default()),
                    ty: p(P::Bool),
                    kind: K::Literal(n::Literal::Bool(true)),
                }))
            };
            region.accept(result, Span::default()).unwrap();
            let retained = region.to_string();
            assert!(
                region
                    .accept(Err(error(Span::default(), "late reply")), Span::default())
                    .is_err()
            );
            assert_eq!(region.to_string(), retained);
            assert!(!retained.contains('\n'));
            if fails {
                assert!(retained.contains("(needs-context true)"), "{retained}");
                assert!(retained.contains("need \\\"type\\\"\\nnext"), "{retained}");
                let State::Failed(failure) = region.state else {
                    panic!("failure retained")
                };
                assert!(failure.needs_context);
            } else {
                let State::Ready(Some(CheckedBlock::Ordinary(value))) = region.state else {
                    panic!("result retained")
                };
                assert!(matches!(value.kind, K::Literal(n::Literal::Bool(true))));
            }
        }
    }

    #[test]
    fn unsolicited_delivery_cannot_create_or_restart_checking() {
        for state in [State::Ready(None), State::Finished] {
            let mut region = Region {
                root: ExpressionResult,
                frames: vec![],
                state,
            };
            let before = region.to_string();
            assert!(
                region
                    .accept(Err(error(Span::default(), "unsolicited")), Span::default())
                    .is_err()
            );
            assert_eq!(region.to_string(), before);
        }
    }
}
