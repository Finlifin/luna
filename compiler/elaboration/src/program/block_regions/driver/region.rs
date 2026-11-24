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
    pub(super) fn check(mut self, checker: &mut Checker<'_, '_>) -> Region<'h, R> {
        let expr = self.child.expr.view();
        let check =
            |checker: &mut Checker<'_, '_>| self.child.destination.check_expression(checker, &expr);
        let result = if let Some(RegionFrame::Block(frame)) = self.parent.frames.last_mut()
            && matches!(frame.progress.phase, Phase::Child)
            && matches!(frame.progress.tail, Destination::Ordinary(_))
            && let H::Let(binding) = expr.kind
            && !binding.comptime
            && let Some(init) = binding.init
            && (binding.ty.is_some() || matches!(init.kind, H::Application(..)))
            && checker.uses_lexical_operands()
        {
            let mut boundary = scoped_operands::Boundary::default();
            let parent = checker.input_facts.annotation.take();
            if binding.ty.is_some() {
                checker.input_facts.annotation =
                    Some(scoped_operands::Annotation::new(expr.hir_id));
            } else {
                boundary.enter(checker, init.hir_id);
            }
            let checked = check(checker);
            boundary.leave(checker);
            let annotation = std::mem::replace(&mut checker.input_facts.annotation, parent);
            let annotate = boundary.annotate_failure();
            checked
                .and_then(|value| {
                    let mut statement = value.ordinary(expr.span)?;
                    frame.progress.binding_scopes.retain(
                        checker,
                        boundary,
                        annotation,
                        &mut statement,
                        &mut frame.progress.statements,
                        expr.span,
                    )?;
                    Ok(CheckedBlock::Ordinary(statement))
                })
                .map_err(annotate)
        } else {
            check(checker)
        };
        self.deliver(result)
    }

    fn deliver(mut self, result: Result<CheckedBlock, Error>) -> Region<'h, R> {
        self.parent.finish_active_wait(&result);
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
        let reply = self
            .operation
            .execute(checker)
            .and_then(|reply| self.parent.root.resume(checker, reply))
            .map(|()| None);
        self.parent.finish_active_wait(&reply);
        self.parent.returned = reply;
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
    dependencies: local_dependencies::WatcherRegistry,
    root_goal: local_dependencies::GoalRef,
    frame_checkpoints: Vec<local_dependencies::Checkpoint>,
    wait_checkpoint: Option<local_dependencies::Checkpoint>,
    active_wait: Option<(local_dependencies::GoalRef, local_dependencies::WaitTicket)>,
}

impl<'h> Region<'h> {
    pub(super) fn new(root: RegionFrame<'h>) -> Self {
        let mut dependencies = local_dependencies::WatcherRegistry::new();
        let root_goal = dependencies.spawn_goal(Some("region-root".to_string()));
        let popped = dependencies.pop_ready();
        debug_assert_eq!(popped, Some(root_goal));
        let frame_checkpoints = vec![dependencies.checkpoint()];
        Self {
            root: ExpressionResult,
            frames: vec![root],
            returned: Ok(None),
            dependencies,
            root_goal,
            frame_checkpoints,
            wait_checkpoint: None,
            active_wait: None,
        }
    }
}

fn refresh_root_goal_in(
    dependencies: &mut local_dependencies::WatcherRegistry,
    root_goal: &mut local_dependencies::GoalRef,
) {
    *root_goal = dependencies
        .live_goal_in_slot(root_goal.slot)
        .unwrap_or_else(|| {
            let goal = dependencies.spawn_goal(Some("region-root".to_string()));
            let _ = dependencies.pop_ready();
            goal
        });
}

impl<'h, R> Region<'h, R> {
    fn begin_wait(&mut self, label: &str) {
        self.wait_checkpoint = Some(self.dependencies.checkpoint());
        let subgoal = self
            .dependencies
            .spawn_waiting_goal(Some(label.to_string()));
        let key = local_dependencies::DependencyKey::Goal(subgoal);
        self.dependencies.bind_producer(key.clone(), subgoal);
        let (rep, version) = self.dependencies.observe_version(&key);
        match self
            .dependencies
            .register_watch(self.root_goal, rep, version)
        {
            Ok(ticket) => self.active_wait = Some((subgoal, ticket)),
            Err(reason) => {
                self.dependencies.cancel_goal(subgoal, reason.to_string());
            }
        }
    }

    fn finish_active_wait<T>(&mut self, result: &Result<T, Error>) {
        let Some((subgoal, ticket)) = self.active_wait.take() else {
            return;
        };
        let key = local_dependencies::DependencyKey::Goal(subgoal);
        match result {
            Ok(_) => {
                self.wait_checkpoint = None;
                self.dependencies.publish_update(&key);
                self.dependencies.wake_ticket(&ticket);
                self.dependencies.complete_goal(subgoal);
                let _ = self.dependencies.pop_ready();
            }
            Err(failure) => {
                if !failure.needs_context {
                    self.dependencies.fail_producer(&key, &failure.message);
                } else {
                    self.dependencies.cancel_goal(subgoal, &failure.message);
                }
                if !self.dependencies.is_live(self.root_goal) {
                    self.root_goal = self
                        .dependencies
                        .spawn_goal(Some("region-root".to_string()));
                    let _ = self.dependencies.pop_ready();
                }
            }
        }
    }

    fn recover_root_wait(&mut self, outer_checkpoint: Option<local_dependencies::Checkpoint>) {
        let wait_checkpoint = self.wait_checkpoint.take();
        if let Some(cp) = outer_checkpoint.or(wait_checkpoint) {
            self.dependencies.rollback(cp);
        }
        refresh_root_goal_in(&mut self.dependencies, &mut self.root_goal);
    }

    fn finish_root_completion(&mut self) -> local_dependencies::FinishReport {
        self.dependencies.complete_goal(self.root_goal);
        let report = self.dependencies.finish();
        debug_assert!(report.is_clean(), "{report}");
        report
    }
}

impl<'h, R: Root<'h>> Region<'h, R> {
    pub(super) fn with_consumer(root: R) -> Self {
        let mut dependencies = local_dependencies::WatcherRegistry::new();
        let root_goal = dependencies.spawn_goal(Some("region-root".to_string()));
        let popped = dependencies.pop_ready();
        debug_assert_eq!(popped, Some(root_goal));
        Self {
            root,
            frames: vec![],
            returned: Ok(None),
            dependencies,
            root_goal,
            frame_checkpoints: Vec::new(),
            wait_checkpoint: None,
            active_wait: None,
        }
    }

    pub(super) fn with_child(root: R, child: RegionFrame<'h>) -> Self {
        let mut dependencies = local_dependencies::WatcherRegistry::new();
        let root_goal = dependencies.spawn_goal(Some("region-root".to_string()));
        let popped = dependencies.pop_ready();
        debug_assert_eq!(popped, Some(root_goal));
        let frame_checkpoints = vec![dependencies.checkpoint()];
        Self {
            root,
            frames: vec![child],
            returned: Ok(None),
            dependencies,
            root_goal,
            frame_checkpoints,
            wait_checkpoint: None,
            active_wait: None,
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
                    self.begin_wait("region-child");
                    return Ok(Handoff::Child(PendingChild {
                        parent: self,
                        child,
                    }));
                }
                Ok(Poll::Invoke(operation)) => {
                    self.begin_wait("region-invocation");
                    return Ok(Handoff::Invoke(PendingInvocation {
                        parent: self,
                        operation,
                    }));
                }
                Ok(Poll::Complete(value)) => {
                    let _ = self.finish_root_completion();
                    return Ok(Handoff::Complete(value));
                }
                Err(failure) => {
                    let mut outer_checkpoint = None;
                    let recovered = recover(
                        &mut self.frames,
                        &mut self.frame_checkpoints,
                        &mut self.dependencies,
                        &mut self.root_goal,
                        &mut outer_checkpoint,
                        checker,
                        failure,
                    );
                    let recovered = match recovered {
                        Ok(()) => {
                            self.wait_checkpoint = None;
                            Ok(())
                        }
                        Err(failure) => {
                            let recovered = self.root.recover(checker, failure);
                            if recovered.is_ok() {
                                self.recover_root_wait(outer_checkpoint);
                            }
                            recovered
                        }
                    };
                    if let Err(failure) = recovered {
                        let _ = self.dependencies.finish();
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
                    self.frame_checkpoints.pop();
                    let value = frame.finish(checker, tail)?;
                    self.returned = Ok(Some(value));
                    return Ok(Poll::Continue);
                }
                Step::Completed(value) => {
                    // The owning frame restores its remaining branch/region boundaries.
                    self.frames.pop().unwrap().unwind(checker);
                    self.frame_checkpoints.pop();
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
        let frame_count = self.frames.len();
        let checkpoint = self.dependencies.checkpoint();
        match children::enter(&mut self.frames, checker, request)? {
            Some(child) => Ok(Poll::Child(child)),
            None => {
                if self.frames.len() > frame_count {
                    self.frame_checkpoints.push(checkpoint);
                }
                Ok(Poll::Continue)
            }
        }
    }
}

fn recover(
    frames: &mut Vec<RegionFrame<'_>>,
    checkpoints: &mut Vec<local_dependencies::Checkpoint>,
    dependencies: &mut local_dependencies::WatcherRegistry,
    root_goal: &mut local_dependencies::GoalRef,
    outer_checkpoint: &mut Option<local_dependencies::Checkpoint>,
    checker: &mut Checker<'_, '_>,
    mut failure: Error,
) -> Result<(), Error> {
    // Lexical exits must run before their enclosing checkpoint.
    // Only a recoverable context request resumes a branch search.
    while let Some(mut frame) = frames.pop() {
        let checkpoint = checkpoints.pop();
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
                if let Some(cp) = checkpoint {
                    let restored_cp = cp.clone();
                    dependencies.rollback(cp);
                    refresh_root_goal_in(dependencies, root_goal);
                    checkpoints.push(restored_cp);
                }
                frames.push(frame);
                return Ok(());
            }
            Err(error) => {
                if checkpoint.is_some() {
                    *outer_checkpoint = checkpoint;
                }
                failure = error;
            }
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
        if self.active_wait.is_some() || self.dependencies.has_activity() {
            write!(f, ") (local-dependencies {}))", self.dependencies)
        } else {
            f.write_str("))")
        }
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

    #[test]
    fn root_recover_after_failed_child_rolls_back_dependencies_cleanly() {
        let mut region = Region::with_consumer(ExpressionResult);
        let field = Expr {
            hir_id: hir::HirId::INVALID,
            kind: H::Unit,
            span: Span::default(),
        };
        for needs_context in [false, true] {
            region.begin_wait("region-child");
            let pending = PendingChild {
                parent: region,
                child: Child {
                    expr: arguments::RoutedSource::Expression(std::borrow::Cow::Borrowed(&field)),
                    destination: Destination::ContextualInput,
                },
            };
            let failure = if needs_context {
                context_error(Span::default(), "probe needs context")
            } else {
                error(Span::default(), "probe failed")
            };
            region = pending.deliver(Err(failure));
            assert!(region.returned.is_err());
            region.returned = Ok(None);
            region.recover_root_wait(None);
        }
        let report = region.finish_root_completion();
        assert!(report.is_clean(), "{report}");
    }
}
