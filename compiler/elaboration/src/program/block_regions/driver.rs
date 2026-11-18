//! Block checking owns its unfinished children and lexical exits.
//! Blocks, branches, comparisons and logical operators share this stack.
//! Other expression rules
//! remain atomic adapters until their continuations can join the region.
use super::*;
mod children;
mod region;
mod root;
pub(super) use root::Root;

struct ExpressionBoundary {
    callee: Option<hir::HirId>,
    preserve_effect: bool,
    partial_input: bool,
    span: Span,
}
impl ExpressionBoundary {
    fn enter(
        checker: &mut Checker<'_, '_>,
        expr: &Expr<'_>,
        block: &hir::Block<'_>,
        partial_input: bool,
    ) -> Self {
        let callee = checker.comptime_callee_result;
        if callee == Some(expr.hir_id) {
            checker.comptime_callee_result = block.expr.map(|tail| tail.hir_id);
        }
        Self {
            callee,
            preserve_effect: partial_input || std::mem::take(&mut checker.preserve_effect_value),
            partial_input,
            span: expr.span,
        }
    }
    fn leave(&self, checker: &mut Checker<'_, '_>) {
        checker.comptime_callee_result = self.callee;
    }
}
impl std::fmt::Display for ExpressionBoundary {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "(block-delivery (source {}) (partial-input {}) (preserve-effect {}) (callee",
            source(self.span),
            self.partial_input,
            self.preserve_effect
        )?;
        if let Some(callee) = self.callee {
            write!(f, " {callee}")?;
        }
        f.write_str("))")
    }
}

struct BlockFrame<'h> {
    progress: Progress<'h, Destination>,
    scope: LexicalScope,
    boundary: Option<ExpressionBoundary>,
}
impl std::fmt::Display for BlockFrame<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "(block-frame {} {} (delivery", self.progress, self.scope)?;
        if let Some(boundary) = &self.boundary {
            write!(f, " {boundary}")?;
        }
        f.write_str("))")
    }
}
impl BlockFrame<'_> {
    fn finish(
        self,
        checker: &mut Checker<'_, '_>,
        value: destinations::CheckedTail,
    ) -> Result<CheckedBlock, Error> {
        let destination = self.progress.tail.clone();
        let result = self.progress.finish(checker, value);
        self.scope.leave(checker);
        if let Some(boundary) = self.boundary {
            boundary.leave(checker);
            let value = result?.ordinary(boundary.span)?;
            if boundary.partial_input {
                return destination
                    .deliver_input(checker, value, boundary.span)
                    .map(CheckedBlock::Ordinary);
            }
            if !boundary.preserve_effect {
                return checker
                    .run_computation(value, destination.expected(), boundary.span)
                    .map(CheckedBlock::Ordinary);
            }
            return Ok(CheckedBlock::Ordinary(value));
        }
        result
    }
    fn unwind(self, checker: &mut Checker<'_, '_>) {
        self.scope.leave(checker);
        if let Some(boundary) = self.boundary {
            boundary.leave(checker);
        }
    }
}

/// Requests distinguish a source block rule from an expression whose delivery
/// protocol still has to run. A then-branch is a block, not a synthetic expr.
pub(super) enum Request<'h> {
    Expression(std::borrow::Cow<'h, Expr<'h>>, Destination),
    Block(&'h hir::Block<'h>, Destination),
}

impl<'h> Request<'h> {
    pub(super) fn expression(input: &'h Expr<'h>, destination: Destination) -> Self {
        Self::Expression(std::borrow::Cow::Borrowed(input), destination)
    }
}

pub(super) enum Poll<Q, T> {
    Continue,
    Child(Q),
    Complete(T),
}

impl<Q: std::fmt::Display, T: std::fmt::Display> std::fmt::Display for Poll<Q, T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Continue => f.write_str("(checking-progress continue)"),
            Self::Child(child) => write!(f, "(checking-progress {child})"),
            Self::Complete(value) => write!(f, "(checking-progress (completed {value}))"),
        }
    }
}

pub(super) enum Step<'h> {
    Continue,
    Child(Request<'h>),
    Tail(destinations::CheckedTail),
    Completed(CheckedBlock),
}

impl std::fmt::Display for Request<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Expression(expr, destination) => write!(
                f,
                "(check-expression {} (source {}) {destination})",
                expr.hir_id,
                source(expr.span)
            ),
            Self::Block(block, destination) => write!(
                f,
                "(check-block {} (source {}) {destination})",
                block.hir_id,
                source(block.span)
            ),
        }
    }
}

impl std::fmt::Display for Step<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Continue => f.write_str("(check-step continue)"),
            Self::Child(request) => write!(f, "(check-step {request})"),
            Self::Tail(value) => write!(f, "(check-step (tail {value}))"),
            Self::Completed(value) => write!(f, "(check-step (completed {value}))"),
        }
    }
}

enum RegionFrame<'h> {
    Aggregate(aggregates::Frame<'h>),
    Block(BlockFrame<'h>),
    Conditional(conditionals::Frame<'h>),
    When(when::Frame<'h>),
    Match(matches::Frame<'h>),
    Unary(unary::Frame<'h>),
    Arithmetic(arithmetic::Frame<'h>),
    Comparison(comparisons::Frame<'h>),
    Logical(logical::Frame<'h>),
    NormalApplication(normal_applications::Frame<'h>),
    CallableApplication(
        declaration_inputs::preparation::Frame<'h, dynamic_instances::InstanceCompletion<'h>>,
    ),
}

impl RegionFrame<'_> {
    fn unwind(self, checker: &mut Checker<'_, '_>) {
        match self {
            Self::Block(frame) => frame.unwind(checker),
            Self::Conditional(frame) => frame.unwind(checker),
            Self::When(frame) => frame.unwind(checker),
            Self::Match(frame) => frame.unwind(checker),
            Self::Comparison(_) | Self::Arithmetic(_) | Self::Unary(_) | Self::Aggregate(_) => {}
            Self::Logical(frame) => frame.unwind(checker),
            Self::NormalApplication(mut frame) => frame.unwind(checker),
            Self::CallableApplication(mut frame) => frame.unwind(checker),
        }
    }
}

impl std::fmt::Display for RegionFrame<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Block(frame) => frame.fmt(f),
            Self::Aggregate(frame) => frame.fmt(f),
            Self::Conditional(frame) => frame.fmt(f),
            Self::When(frame) => frame.fmt(f),
            Self::Match(frame) => frame.fmt(f),
            Self::Comparison(frame) => frame.fmt(f),
            Self::Arithmetic(frame) => frame.fmt(f),
            Self::Unary(frame) => frame.fmt(f),
            Self::Logical(frame) => frame.fmt(f),
            Self::NormalApplication(frame) => frame.fmt(f),
            Self::CallableApplication(frame) => frame.fmt(f),
        }
    }
}

impl<'h> BlockFrame<'h> {
    fn step(
        &mut self,
        checker: &mut Checker<'_, '_>,
        returned: Option<CheckedBlock>,
        statement_destination: &Destination,
    ) -> Result<Step<'h>, Error> {
        if let Some(value) = returned {
            return match self.progress.phase {
                Phase::Child => {
                    self.progress
                        .accept(checker, value.ordinary(self.progress.block.span)?)?;
                    Ok(Step::Continue)
                }
                Phase::TailChild => self
                    .progress
                    .tail
                    .complete(
                        checker,
                        &self.progress.block,
                        self.progress.diverges,
                        Some(value),
                    )
                    .map(Step::Tail),
                _ => Err(error(
                    self.progress.block.span,
                    "block received an unsolicited child",
                )),
            };
        }
        match self.progress.phase {
            Phase::Statements => match self.progress.next()? {
                Some(statement) => {
                    checker.required_steps = checker
                        .required_steps
                        .checked_sub(1)
                        .ok_or_else(|| error(statement.span, "block checking budget exhausted"))?;
                    Ok(Step::Child(Request::expression(
                        statement,
                        statement_destination.clone(),
                    )))
                }
                None => Ok(Step::Continue),
            },
            Phase::Tail => {
                let tail = self.progress.request_tail()?.clone();
                match tail.prepare(checker, &self.progress.block, self.progress.diverges)? {
                    tails::TailCheck::Checked(value) => tail
                        .complete(checker, &self.progress.block, self.progress.diverges, value)
                        .map(Step::Tail),
                    tails::TailCheck::Child => Ok(Step::Child(Request::expression(
                        self.progress.block.expr.unwrap(),
                        tail,
                    ))),
                }
            }
            _ => Err(error(
                self.progress.block.span,
                "block child result is missing",
            )),
        }
    }
}

impl<'a, 'arena> Checker<'a, 'arena> {
    pub(in crate::program) fn logical_expression(
        &mut self,
        expr: &Expr<'_>,
        expected: Option<&Term>,
    ) -> Result<n::Expr, Error> {
        let destination = Destination::ordinary(
            self,
            OrdinaryTail {
                expected,
                partial: None,
                input: None,
            },
            expr.span,
        )?;
        let frame = logical::Frame::new(self, expr, destination, false)?;
        self.drive_check_region(RegionFrame::Logical(frame))?
            .ordinary(expr.span)
    }

    pub(in crate::program) fn comparison_expression(
        &mut self,
        expr: &Expr<'_>,
        expected: Option<&Term>,
    ) -> Result<n::Expr, Error> {
        let destination = Destination::ordinary(
            self,
            OrdinaryTail {
                expected,
                partial: None,
                input: None,
            },
            expr.span,
        )?;
        let frame = comparisons::Frame::new(self, expr, destination, false)?;
        self.drive_check_region(RegionFrame::Comparison(frame))?
            .ordinary(expr.span)
    }

    pub(in crate::program) fn check_contextual_aggregate(
        &mut self,
        expr: &Expr<'_>,
        depth: usize,
    ) -> Result<n::NormalFormArgument, Error> {
        let frame = aggregates::Frame::contextual(expr, depth)?;
        self.drive_check_region(RegionFrame::Aggregate(frame))?
            .contextual(expr.span)
    }

    pub(in crate::program) fn check_partial_tuple(
        &mut self,
        expr: &Expr<'_>,
        items: &[Expr<'_>],
        context: &crate::calls::InputContext,
    ) -> Result<n::Expr, Error> {
        let destination = Destination::ordinary(
            self,
            OrdinaryTail {
                expected: None,
                partial: None,
                input: Some(context),
            },
            expr.span,
        )?;
        let frame = aggregates::Frame::partial_tuple(expr, items, destination)?;
        self.drive_check_region(RegionFrame::Aggregate(frame))?
            .ordinary(expr.span)
    }

    pub(in crate::program) fn check_tuple_fields(
        &mut self,
        expr: &Expr<'_>,
        items: &[Expr<'_>],
        expected: Option<&Term>,
    ) -> Result<n::Expr, Error> {
        let frame = aggregates::Frame::tuple(self, expr, items, expected)?;
        self.drive_check_region(RegionFrame::Aggregate(frame))?
            .ordinary(expr.span)
    }

    pub(in crate::program) fn check_list_fields(
        &mut self,
        expr: &Expr<'_>,
        items: &[Expr<'_>],
        expected: &Term,
        element: &Term,
    ) -> Result<n::Expr, Error> {
        let frame = aggregates::Frame::list(self, expr, items, expected, element)?;
        self.drive_check_region(RegionFrame::Aggregate(frame))?
            .ordinary(expr.span)
    }

    pub(in crate::program) fn unary_expression(
        &mut self,
        expr: &Expr<'_>,
        expected: Option<&Term>,
    ) -> Result<n::Expr, Error> {
        let destination = Destination::ordinary(
            self,
            OrdinaryTail {
                expected,
                partial: None,
                input: None,
            },
            expr.span,
        )?;
        let frame = unary::Frame::new(self, expr, destination, false)?;
        self.drive_check_region(RegionFrame::Unary(frame))?
            .ordinary(expr.span)
    }

    pub(in crate::program) fn arithmetic_expression(
        &mut self,
        expr: &Expr<'_>,
        expected: Option<&Term>,
    ) -> Result<n::Expr, Error> {
        let destination = Destination::ordinary(
            self,
            OrdinaryTail {
                expected,
                partial: None,
                input: None,
            },
            expr.span,
        )?;
        let frame = arithmetic::Frame::new(self, expr, destination, false)?;
        self.drive_check_region(RegionFrame::Arithmetic(frame))?
            .ordinary(expr.span)
    }

    pub(in crate::program) fn check_normal_application_region(
        &mut self,
        owner: OwnerId,
        inputs: &[Arg<'_>],
        span: Span,
        expected: Option<&Term>,
    ) -> Result<n::Expr, Error> {
        let destination = Destination::ordinary(
            self,
            OrdinaryTail {
                expected,
                partial: None,
                input: None,
            },
            span,
        )?;
        let frame = normal_applications::Frame::begin_normal_application(
            self,
            owner,
            inputs,
            span,
            destination,
            false,
        )?;
        self.drive_check_region(RegionFrame::NormalApplication(frame))?
            .ordinary(span)
    }

    fn charge_block_frame(&mut self, span: Span) -> Result<(), Error> {
        self.required_steps = self
            .required_steps
            .checked_sub(1)
            .ok_or_else(|| error(span, "block checking budget exhausted"))?;
        Ok(())
    }
    pub(in crate::program) fn check_ordinary_block_region(
        &mut self,
        block: &hir::Block<'_>,
        tail: OrdinaryTail<'_>,
    ) -> Result<n::Expr, Error> {
        let destination = Destination::ordinary(self, tail, block.span)?;
        self.check_block_region(block, destination)?
            .ordinary(block.span)
    }

    pub(in crate::program) fn check_contextual_block_region(
        &mut self,
        block: &hir::Block<'_>,
        tail: ContextualTail,
    ) -> Result<n::NormalFormArgument, Error> {
        self.check_block_region(block, Destination::Contextual(tail))?
            .contextual(block.span)
    }

    pub(in crate::program) fn check_contextual_if_region(
        &mut self,
        expr: &Expr<'_>,
        depth: usize,
    ) -> Result<n::NormalFormArgument, Error> {
        let frame = conditionals::Frame::new(self, expr, depth)?;
        self.drive_check_region(RegionFrame::Conditional(frame))?
            .contextual(expr.span)
    }

    pub(in crate::program) fn check_contextual_match_region(
        &mut self,
        expr: &Expr<'_>,
        depth: usize,
    ) -> Result<n::NormalFormArgument, Error> {
        let frame = matches::Frame::new(self, expr, depth)?;
        self.drive_check_region(RegionFrame::Match(frame))?
            .contextual(expr.span)
    }

    pub(in crate::program) fn check_ordinary_match_region(
        &mut self,
        expr: &Expr<'_>,
        expected: Option<&Term>,
        partial: Option<&crate::calls::CallableContext>,
        input: Option<&crate::calls::InputContext>,
    ) -> Result<n::Expr, Error> {
        let destination = Destination::ordinary(
            self,
            OrdinaryTail {
                expected,
                partial,
                input,
            },
            expr.span,
        )?;
        let frame = matches::Frame::ordinary(self, expr, destination, false)?;
        self.drive_check_region(RegionFrame::Match(frame))?
            .ordinary(expr.span)
    }

    pub(in crate::program) fn check_ordinary_when_region(
        &mut self,
        expr: &Expr<'_>,
        expected: Option<&Term>,
        partial: Option<&crate::calls::CallableContext>,
        input: Option<&crate::calls::InputContext>,
    ) -> Result<n::Expr, Error> {
        let destination = Destination::ordinary(
            self,
            OrdinaryTail {
                expected,
                partial,
                input,
            },
            expr.span,
        )?;
        let frame = when::Frame::ordinary(self, expr, destination, false)?;
        self.drive_check_region(RegionFrame::When(frame))?
            .ordinary(expr.span)
    }

    pub(in crate::program) fn check_contextual_when_region(
        &mut self,
        expr: &Expr<'_>,
        depth: usize,
    ) -> Result<n::NormalFormArgument, Error> {
        let frame = when::Frame::new(self, expr, depth)?;
        self.drive_check_region(RegionFrame::When(frame))?
            .contextual(expr.span)
    }

    pub(in crate::program) fn check_ordinary_if_region(
        &mut self,
        expr: &Expr<'_>,
        expected: Option<&Term>,
        partial: Option<&crate::calls::CallableContext>,
    ) -> Result<n::Expr, Error> {
        let destination = Destination::ordinary(
            self,
            OrdinaryTail {
                expected,
                partial,
                input: None,
            },
            expr.span,
        )?;
        let frame = conditionals::Frame::ordinary(self, expr, destination, false)?;
        self.drive_check_region(RegionFrame::Conditional(frame))?
            .ordinary(expr.span)
    }

    pub(in crate::program) fn check_partial_if_region(
        &mut self,
        expr: &Expr<'_>,
        input: &crate::calls::InputContext,
    ) -> Result<n::Expr, Error> {
        let destination = Destination::ordinary(
            self,
            OrdinaryTail {
                expected: None,
                partial: None,
                input: Some(input),
            },
            expr.span,
        )?;
        let frame = conditionals::Frame::partial(self, expr, destination, false)?;
        self.drive_check_region(RegionFrame::Conditional(frame))?
            .ordinary(expr.span)
    }

    fn check_block_region(
        &mut self,
        block: &hir::Block<'_>,
        tail: Destination,
    ) -> Result<CheckedBlock, Error> {
        self.charge_block_frame(block.span)?;
        let scope = LexicalScope::enter(self, block.span)?;
        self.drive_check_region(RegionFrame::Block(BlockFrame {
            progress: Progress::new(block, tail),
            scope,
            boundary: None,
        }))
    }

    fn drive_check_region<'h>(&mut self, root: RegionFrame<'h>) -> Result<CheckedBlock, Error>
    where
        'a: 'h,
        'arena: 'h,
    {
        self.drive_region(region::Region::new(root))
    }

    pub(in crate::program) fn check_declaration_inputs_region<
        'h,
        C: declaration_inputs::Completion,
    >(
        &mut self,
        preparation: declaration_inputs::preparation::Frame<'h, C>,
    ) -> Result<C::Output, Error>
    where
        'a: 'h,
        'arena: 'h,
    {
        self.drive_region(region::Region::with_consumer(preparation))
    }

    pub(super) fn drive_input_consumer<'b, R: Root<'b>>(
        &mut self,
        root: R,
    ) -> Result<R::Output, Error>
    where
        'a: 'b,
        'arena: 'b,
    {
        self.drive_region(region::Region::with_consumer(root))
    }

    pub(in crate::program) fn check_call_dependencies_region<'b>(
        &mut self,
        frame: dependent_inputs::tasks::Frame<'b, '_>,
    ) -> Result<dependent_inputs::PreparedInputs, Error>
    where
        'a: 'b,
        'arena: 'b,
    {
        self.drive_region(region::Region::with_consumer(frame))
    }

    fn drive_region<'h, R: Root<'h>>(
        &mut self,
        mut region: region::Region<'h, R>,
    ) -> Result<R::Output, Error>
    where
        'a: 'h,
        'arena: 'h,
    {
        loop {
            match region.advance(self)? {
                region::Handoff::Child(pending) => region = pending.check(self),
                region::Handoff::Complete(value) => return Ok(value),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn block_frame_dump_retains_parent_scope_and_child_delivery_separately() {
        let id = hir::HirId::new(
            hir::OwnerId::new(hir::LocalDefId::new(1)),
            hir::ItemLocalId::new(0),
        );
        let block = hir::Block {
            hir_id: id,
            stmts: &[],
            expr: None,
            span: Span::default(),
        };
        let scope = LexicalScope {
            locals: LocalBindings::from([(
                n::LocalId { owner: 1, index: 7 },
                Term::NormalForm {
                    head: DeclarationId {
                        package: 3,
                        local: 5,
                    },
                    sort: Sort::Type,
                    arguments: vec![Term::String("scope\"\n".into())],
                },
            )]),
            code: LocalBindings::default(),
            patterns: LocalBindings::default(),
            context: crate::ptc::Context::from_package(&n::Package::default()).unwrap(),
            inputs: input_facts::State::default().lexical_boundary(),
        };
        let mut frame = BlockFrame {
            progress: Progress::new(&block, Destination::statement()),
            scope,
            boundary: Some(ExpressionBoundary {
                callee: Some(id),
                preserve_effect: true,
                partial_input: false,
                span: block.span,
            }),
        };
        assert!(frame.progress.next().unwrap().is_none());
        frame.progress.request_tail().unwrap();
        let dump = frame.to_string();
        assert!(dump.starts_with("(block-frame (block-check"), "{dump}");
        assert!(dump.contains("(phase tail-child)"), "{dump}");
        assert!(dump.contains("(lexical-parent (locals"), "{dump}");
        assert!(dump.contains("(preserve-effect true) (callee"), "{dump}");
        assert!(dump.contains("scope\\\"\\n"), "{dump}");
        assert!(!dump.contains('\n'), "{dump}");
        let region = region::Region::new(RegionFrame::Block(frame));
        assert_eq!(
            region.to_string(),
            format!("(checking-region (state ready) (root (expression-result)) (frames {dump}))")
        );
        assert_eq!(
            tails::TailCheck::<n::Expr>::Child.to_string(),
            "(tail-check child)"
        );
    }
}
