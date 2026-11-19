//! The child owns its parent continuation until it returns an expression or
//! a failure. Frames retain source headers and borrow their original children
//! within the active HIR snapshot. Lexical exits still use the same checker.
use super::*;
use root::ExpressionResult;

pub(super) struct Child<'h> {
    pub(super) expr: arguments::RoutedSource<'h>,
    pub(super) destination: Destination,
}

impl std::fmt::Display for Child<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let expr = self.expr.view();
        write!(
            f,
            "(child {} (source {}) {})",
            expr.hir_id,
            source(expr.span),
            self.destination
        )
    }
}

/// An unfinished parent travels with its unique requested child. Completing
/// this handoff consumes it, so another region cannot be advanced by its reply.
pub(super) struct PendingChild<'h, R> {
    parent: Region<'h, R>,
    child: Child<'h>,
}

impl<'h, R> PendingChild<'h, R> {
    pub(super) fn check(self, checker: &mut Checker<'_, '_>) -> Region<'h, R> {
        let result = self
            .child
            .destination
            .check_expression(checker, &self.child.expr.view());
        self.deliver(result)
    }

    fn deliver(mut self, result: Result<CheckedBlock, Error>) -> Region<'h, R> {
        self.parent.returned = result.map(Some);
        self.parent
    }
}

impl<R: std::fmt::Display> std::fmt::Display for PendingChild<'_, R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "(pending-child {} {})", self.child, self.parent)
    }
}

/// The prepared operation leaves with its only consumer. Failure re-enters
/// normal region recovery before any further source child can be requested.
pub(super) struct PendingInvocation<'h, R: Root<'h>> {
    parent: Region<'h, R>,
    operation: R::Invocation,
}

impl<'h, R: Root<'h>> PendingInvocation<'h, R> {
    pub(super) fn execute(mut self, checker: &mut Checker<'_, '_>) -> Region<'h, R> {
        self.parent.returned = self
            .operation
            .execute(checker)
            .and_then(|reply| self.parent.root.resume(checker, reply))
            .map(|()| None);
        self.parent
    }
}

impl<'h, R: Root<'h>> std::fmt::Display for PendingInvocation<'h, R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "(pending-invocation {} {})", self.operation, self.parent)
    }
}

pub(super) enum Handoff<'h, R: Root<'h>> {
    Child(PendingChild<'h, R>),
    Invoke(PendingInvocation<'h, R>),
    Complete(R::Output),
}

#[must_use = "drive this region to completion in its active checker"]
pub(super) struct Region<'h, R = ExpressionResult> {
    root: R,
    frames: Vec<RegionFrame<'h>>,
    returned: Result<Option<CheckedBlock>, Error>,
}

impl<'h> Region<'h> {
    pub(super) fn new(root: RegionFrame<'h>) -> Self {
        Self {
            root: ExpressionResult,
            frames: vec![root],
            returned: Ok(None),
        }
    }
}

impl<'h, R: Root<'h>> Region<'h, R> {
    pub(super) fn with_consumer(root: R) -> Self {
        Self {
            root,
            frames: vec![],
            returned: Ok(None),
        }
    }

    pub(super) fn with_child(root: R, child: RegionFrame<'h>) -> Self {
        Self {
            root,
            frames: vec![child],
            returned: Ok(None),
        }
    }

    pub(super) fn advance<'a: 'h, 'arena: 'h>(
        mut self,
        checker: &mut Checker<'a, 'arena>,
    ) -> Result<Handoff<'h, R>, Error> {
        loop {
            match self.step(checker) {
                Ok(Poll::Continue) => {}
                Ok(Poll::Child(child)) => {
                    return Ok(Handoff::Child(PendingChild {
                        parent: self,
                        child,
                    }));
                }
                Ok(Poll::Invoke(operation)) => {
                    return Ok(Handoff::Invoke(PendingInvocation {
                        parent: self,
                        operation,
                    }));
                }
                Ok(Poll::Complete(value)) => return Ok(Handoff::Complete(value)),
                Err(failure) => {
                    let recovered = recover(&mut self.frames, checker, failure)
                        .or_else(|failure| self.root.recover(checker, failure));
                    if let Err(failure) = recovered {
                        self.root.unwind(checker);
                        return Err(failure);
                    }
                }
            }
        }
    }

    fn step<'a: 'h, 'arena: 'h>(
        &mut self,
        checker: &mut Checker<'a, 'arena>,
    ) -> Result<Poll<Child<'h>, R::Output, R::Invocation>, Error> {
        let returned = std::mem::replace(&mut self.returned, Ok(None))?;
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
                    self.returned = Ok(Some(value));
                    return Ok(Poll::Continue);
                }
                Step::Completed(value) => {
                    // The owning frame restores its remaining branch/region boundaries.
                    self.frames.pop().unwrap().unwind(checker);
                    self.returned = Ok(Some(value));
                    return Ok(Poll::Continue);
                }
                Step::Child(request) => request,
            }
        } else {
            match self.root.advance(checker, returned)? {
                Poll::Continue => return Ok(Poll::Continue),
                Poll::Invoke(operation) => return Ok(Poll::Invoke(operation)),
                Poll::Complete(value) => return Ok(Poll::Complete(value)),
                Poll::Child(request) => request,
            }
        };
        match children::enter(&mut self.frames, checker, request)? {
            Some(child) => Ok(Poll::Child(child)),
            None => Ok(Poll::Continue),
        }
    }
}

fn recover(
    frames: &mut Vec<RegionFrame<'_>>,
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

impl<R: std::fmt::Display> std::fmt::Display for Region<'_, R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("(checking-region (state ")?;
        match &self.returned {
            Ok(None) => f.write_str("ready")?,
            Ok(Some(value)) => write!(f, "(returned {value})")?,
            Err(failure) => write!(
                f,
                "(failed (source {}) (needs-context {}) (message {:?}))",
                failure.source, failure.needs_context, failure.message
            )?,
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
    fn routed_header_can_expire_before_its_child_returns() {
        let field = Expr {
            hir_id: hir::HirId::INVALID,
            kind: H::Unit,
            span: Span::default(),
        };
        let items = [field];
        let frame = {
            let header = Expr {
                hir_id: hir::HirId::INVALID,
                kind: H::Tuple(&items),
                span: Span::default(),
            };
            aggregates::Frame::contextual(&header, 0).unwrap()
        };
        let pending = PendingChild {
            parent: Region::new(RegionFrame::Aggregate(frame)),
            child: Child {
                expr: arguments::RoutedSource::Expression(std::borrow::Cow::Borrowed(&items[0])),
                destination: Destination::ContextualInput,
            },
        };
        let waiting = pending.to_string();
        assert!(waiting.contains("(pending-child (child"), "{waiting}");
        assert!(
            waiting.contains("(aggregate-frame (kind tuple)"),
            "{waiting}"
        );
        let returned = pending.deliver(Err(context_error(Span::default(), "need \"type\"\nnext")));
        let dump = returned.to_string();
        assert!(dump.contains("(needs-context true)"), "{dump}");
        assert!(dump.contains(r#"need \"type\"\nnext"#), "{dump}");
        assert!(dump.contains("(aggregate-frame (kind tuple)"), "{dump}");
    }
}
