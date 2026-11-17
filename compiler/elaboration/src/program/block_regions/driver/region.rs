//! Active source-checking work survives each child handoff as owned state.
//! This internal driver still borrows the original HIR and uses the same active
//! Checker. It is not a transportable task or an execution/Type response API.
use super::*;

pub(super) struct Child<'b, 'h> {
    pub(super) expr: &'b Expr<'h>,
    pub(super) destination: Destination,
}

pub(super) enum Poll<'b, 'h> {
    Continue,
    Child(Child<'b, 'h>),
    Completed(CheckedBlock),
}

enum State {
    Ready(Option<CheckedBlock>),
    Waiting(Span),
    Failed(Error),
    Finished,
}

#[must_use = "drive this region to completion in its active checker"]
pub(super) struct Region<'b, 'h> {
    frames: Vec<RegionFrame<'b, 'h>>,
    state: State,
}

impl<'b, 'h> Region<'b, 'h> {
    pub(super) fn new(root: RegionFrame<'b, 'h>) -> Self {
        Self {
            frames: vec![root],
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
    ) -> Result<Poll<'b, 'h>, Error> {
        match self.state {
            State::Waiting(span) => {
                return Err(error(span, "checking region is awaiting its child"));
            }
            State::Finished => return Err(error(Span::default(), "checking region is complete")),
            _ => {}
        }
        let state = std::mem::replace(&mut self.state, State::Ready(None));
        let result = (|| {
            let mut returned = match state {
                State::Ready(value) => value,
                State::Failed(failure) => return Err(failure),
                _ => unreachable!("only runnable regions advance"),
            };
            let step = match self.frames.last_mut().expect("active checking frame") {
                RegionFrame::Block(frame) => {
                    frame.step(checker, returned.take(), &Destination::statement())?
                }
                RegionFrame::Conditional(frame) => frame.step(checker, returned.take())?,
                RegionFrame::When(frame) => frame.step(checker, returned.take())?,
                RegionFrame::Match(frame) => frame.step(checker, returned.take())?,
                RegionFrame::Comparison(frame) => frame.step(checker, returned.take())?,
                RegionFrame::Arithmetic(frame) => frame.step(checker, returned.take())?,
                RegionFrame::Unary(frame) => frame.step(checker, returned.take())?,
                RegionFrame::Logical(frame) => frame.step(checker, returned.take())?,
                RegionFrame::NormalApplication(frame) => frame.step(checker, returned.take())?,
                RegionFrame::CallableApplication(frame) => frame.step(checker, returned.take())?,
            };
            let request = match step {
                Step::Continue => return Ok(Poll::Continue),
                Step::Tail(tail) => {
                    let RegionFrame::Block(frame) = self.frames.pop().unwrap() else {
                        unreachable!("only blocks complete a tail")
                    };
                    let value = frame.finish(checker, tail)?;
                    if self.frames.is_empty() {
                        return Ok(Poll::Completed(value));
                    }
                    self.state = State::Ready(Some(value));
                    return Ok(Poll::Continue);
                }
                Step::Completed(value) => {
                    // The owning frame restores its remaining branch/region boundaries.
                    self.frames.pop().unwrap().unwind(checker);
                    if self.frames.is_empty() {
                        return Ok(Poll::Completed(value));
                    }
                    self.state = State::Ready(Some(value));
                    return Ok(Poll::Continue);
                }
                Step::Child(request) => request,
            };
            let (block, destination, boundary) = match request {
                Request::Block(block, destination) => (block, destination, None),
                Request::Expression(expr, destination) => {
                    if let H::NFApplication(head, inputs) = expr.kind
                        && let H::ResolvedDef { def_id, .. } = head.kind
                        && checker.data_consumer_call(expr)?.is_none()
                        && let Some(view) = destination.ordinary_view()
                        && view.input.is_none()
                        && view.expected.is_none_or(|ty| {
                            matches!(ty, Term::Primitive(_)) || ty.is_comptime_callable()
                        })
                    {
                        // Captured declaration inputs still go through their
                        // existing lexical/default supply before NF preparation.
                        let source_owner = checker.package.definition_owner(def_id);
                        let captured = if let Some(owner) = source_owner {
                            checker
                                .nominal_captures(owner, expr.span)?
                                .iter()
                                .any(|id| checker.alias_value_bindings.contains_key(id))
                        } else {
                            false
                        };
                        let owner =
                            if captured {
                                None
                            } else if let Some(owner) = checker.normal_form_constant_target(head)? {
                                Some(owner)
                            } else {
                                source_owner.filter(|owner| matches!(
                                    checker.package.item(*owner).map(|item| &item.kind),
                                    Some(ItemKind::Fn(sig, _)) if sig.modifiers.is_normal_form_head
                                ))
                            };
                        if let Some(owner) = owner
                            && checker.prepare_region_callable_delivery(
                                expr,
                                view.expected,
                                view.partial,
                            )?
                        {
                            self.frames.push(RegionFrame::NormalApplication(
                                normal_applications::Frame::new(
                                    checker,
                                    owner,
                                    inputs,
                                    expr.span,
                                    destination,
                                    true,
                                )?,
                            ));
                            return Ok(Poll::Continue);
                        }
                        if let Some(owner) = source_owner
                            && !captured
                            && !checker.runtime_value_context()
                            && !checker.in_code_body
                            && matches!(checker.package.item(owner).map(|item| &item.kind),
                                    Some(ItemKind::Fn(sig, _) | ItemKind::FnDeclaration(sig))
                                    if sig.modifiers.is_comptime && !sig.modifiers.is_normal_form_head)
                            && checker.prepare_region_callable_delivery(
                                expr,
                                view.expected,
                                view.partial,
                            )?
                        {
                            checker.ensure_signature(owner, expr.span)?;
                            let prefix = checker.lexical_function_arguments(owner, expr.span)?;
                            let preserve_effect = checker.preserve_effect_value;
                            if let Some(preparation) = checker.begin_comptime_function_instance(
                                expr,
                                owner,
                                inputs,
                                &prefix,
                                view.expected,
                                false,
                                Some(preserve_effect),
                            )? {
                                checker.preserve_effect_value = false;
                                self.frames.push(RegionFrame::CallableApplication(
                                    declaration_applications::Frame::from_preparation(
                                        expr.span,
                                        preparation,
                                    ),
                                ));
                                return Ok(Poll::Continue);
                            }
                        }
                    }
                    // Nonprimitive targets first pass through ordinary
                    // delivery adaptation, which enters the same frame.
                    if (comparisons::is_comparison(expr)
                        || logical::is_logical(expr)
                        || arithmetic::operator(expr).is_some()
                        || unary::operator(expr).is_some())
                        && let Some(view) = destination.ordinary_view()
                        && view.input.is_none()
                        && view
                            .expected
                            .is_none_or(|ty| matches!(ty, Term::Primitive(_)))
                    {
                        let frame = if unary::operator(expr).is_some() {
                            RegionFrame::Unary(unary::Frame::new(checker, expr, destination, true)?)
                        } else if logical::is_logical(expr) {
                            RegionFrame::Logical(logical::Frame::new(
                                checker,
                                expr,
                                destination,
                                true,
                            )?)
                        } else if arithmetic::operator(expr).is_some() {
                            RegionFrame::Arithmetic(arithmetic::Frame::new(
                                checker,
                                expr,
                                destination,
                                true,
                            )?)
                        } else {
                            RegionFrame::Comparison(comparisons::Frame::new(
                                checker,
                                expr,
                                destination,
                                true,
                            )?)
                        };
                        self.frames.push(frame);
                        return Ok(Poll::Continue);
                    }
                    // Retain the input rule and its delivery boundary;
                    // probes use the same child driver as final checks.
                    if destination.structural_input().is_some()
                        && matches!(expr.kind, H::If(_, _, Some(_)))
                    {
                        self.frames
                            .push(RegionFrame::Conditional(conditionals::Frame::partial(
                                checker,
                                expr,
                                destination,
                                true,
                            )?));
                        return Ok(Poll::Continue);
                    }
                    if destination.structural_input().is_some() {
                        let frame = match expr.kind {
                            H::When(_) => Some(RegionFrame::When(when::Frame::ordinary(
                                checker,
                                expr,
                                destination.clone(),
                                true,
                            )?)),
                            H::Match(..) => Some(RegionFrame::Match(matches::Frame::ordinary(
                                checker,
                                expr,
                                destination.clone(),
                                true,
                            )?)),
                            _ => None,
                        };
                        if let Some(frame) = frame {
                            self.frames.push(frame);
                            return Ok(Poll::Continue);
                        }
                    }
                    if let Some(depth) = destination.contextual_depth()
                        && matches!(expr.kind, H::Match(..))
                    {
                        self.frames.push(RegionFrame::Match(matches::Frame::new(
                            checker, expr, depth,
                        )?));
                        return Ok(Poll::Continue);
                    }
                    if let Some(depth) = destination.contextual_depth()
                        && let H::When(arms) = expr.kind
                        && arms.iter().any(|arm| arm.cond.is_none())
                    {
                        self.frames
                            .push(RegionFrame::When(when::Frame::new(checker, expr, depth)?));
                        return Ok(Poll::Continue);
                    }
                    // Ordinary branches enter after delivery prepares the
                    // complete target, as in expr_context.
                    if let Some(depth) = destination.contextual_depth()
                        && matches!(expr.kind, H::If(_, _, Some(_)))
                    {
                        self.frames
                            .push(RegionFrame::Conditional(conditionals::Frame::new(
                                checker, expr, depth,
                            )?));
                        return Ok(Poll::Continue);
                    }
                    if matches!(expr.kind, H::Match(..))
                        && let Some(view) = destination.ordinary_view()
                        && (view.input.is_none() || view.expected.is_some())
                        && checker.prepare_region_branch_delivery(
                            expr,
                            view.expected,
                            view.partial,
                        )?
                    {
                        self.frames
                            .push(RegionFrame::Match(matches::Frame::ordinary(
                                checker,
                                expr,
                                destination,
                                true,
                            )?));
                        return Ok(Poll::Continue);
                    }
                    if matches!(expr.kind, H::When(_))
                        && let Some(view) = destination.ordinary_view()
                        && (view.input.is_none() || view.expected.is_some())
                        && checker.prepare_region_branch_delivery(
                            expr,
                            view.expected,
                            view.partial,
                        )?
                    {
                        self.frames.push(RegionFrame::When(when::Frame::ordinary(
                            checker,
                            expr,
                            destination,
                            true,
                        )?));
                        return Ok(Poll::Continue);
                    }
                    if matches!(expr.kind, H::If(..))
                        && let Some(view) = destination.ordinary_view()
                        && (view.input.is_none() || view.expected.is_some())
                        && checker.prepare_region_branch_delivery(
                            expr,
                            view.expected,
                            view.partial,
                        )?
                    {
                        self.frames
                            .push(RegionFrame::Conditional(conditionals::Frame::ordinary(
                                checker,
                                expr,
                                destination,
                                true,
                            )?));
                        return Ok(Poll::Continue);
                    }
                    if let Some(nested) = destination.nested(expr)? {
                        let H::Block(block) = expr.kind else {
                            unreachable!("nested block destination")
                        };
                        let boundary = matches!(nested, Destination::Ordinary(_)).then(|| {
                            ExpressionBoundary::enter(
                                checker,
                                expr,
                                block,
                                nested.structural_input().is_some(),
                            )
                        });
                        (block, nested, boundary)
                    } else {
                        self.state = State::Waiting(expr.span);
                        return Ok(Poll::Child(Child { expr, destination }));
                    }
                }
            };
            let scope = match checker
                .charge_block_frame(block.span)
                .and_then(|()| LexicalScope::enter(checker, block.span))
            {
                Ok(scope) => scope,
                Err(error) => {
                    if let Some(boundary) = &boundary {
                        boundary.leave(checker);
                    }
                    return Err(error);
                }
            };
            self.frames.push(RegionFrame::Block(BlockFrame {
                progress: Progress::new(block, destination),
                scope,
                boundary,
            }));
            Ok(Poll::Continue)
        })();
        match result {
            Ok(Poll::Completed(value)) => {
                self.state = State::Finished;
                Ok(Poll::Completed(value))
            }
            Ok(progress) => Ok(progress),
            Err(failure) => {
                let mut failure = Some(failure);
                // Lexical exits must run before their enclosing checkpoint.
                // Only a recoverable context request resumes a branch search.
                while let Some(mut frame) = self.frames.pop() {
                    let recovered = match &mut frame {
                        RegionFrame::Conditional(conditional) => {
                            Some(conditional.recover_probe(checker, failure.take().unwrap()))
                        }
                        RegionFrame::When(frame) => {
                            Some(frame.recover_probe(checker, failure.take().unwrap()))
                        }
                        RegionFrame::Match(frame) => {
                            Some(frame.recover_probe(checker, failure.take().unwrap()))
                        }
                        RegionFrame::Comparison(comparison) => {
                            Some(comparison.recover(checker, failure.take().unwrap()))
                        }
                        RegionFrame::Unary(frame) => {
                            Some(frame.recover(checker, failure.take().unwrap()))
                        }
                        RegionFrame::Arithmetic(frame) => {
                            Some(frame.recover(checker, failure.take().unwrap()))
                        }
                        RegionFrame::Logical(frame) => {
                            Some(frame.recover(checker, failure.take().unwrap()))
                        }
                        RegionFrame::NormalApplication(frame) => {
                            Some(frame.recover(checker, failure.take().unwrap()))
                        }
                        RegionFrame::CallableApplication(frame) => {
                            Some(frame.recover(checker, failure.take().unwrap()))
                        }
                        _ => None,
                    };
                    if let Some(recovered) = recovered {
                        match recovered {
                            Ok(()) => {
                                self.frames.push(frame);
                                self.state = State::Ready(None);
                                return Ok(Poll::Continue);
                            }
                            Err(error) => failure = Some(error),
                        }
                    }
                    frame.unwind(checker);
                }
                self.state = State::Finished;
                Err(failure.expect("unrecovered checking failure"))
            }
        }
    }
}

impl std::fmt::Display for Region<'_, '_> {
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
        f.write_str(") (frames")?;
        for frame in &self.frames {
            write!(f, " {frame}")?;
        }
        f.write_str("))")
    }
}

impl std::fmt::Display for Poll<'_, '_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Continue => f.write_str("(checking-progress continue)"),
            Self::Child(child) => write!(
                f,
                "(checking-progress (child {} (source {}) {}))",
                child.expr.hir_id,
                source(child.expr.span),
                child.destination
            ),
            Self::Completed(value) => write!(f, "(checking-progress (completed {value}))"),
        }
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
