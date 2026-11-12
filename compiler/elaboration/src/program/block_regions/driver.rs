//! Block checking owns its unfinished children and lexical exits.
//! Blocks and conditionals share this stack. Other expression rules
//! remain atomic adapters until their continuations can join the region.
use super::*;

struct ExpressionBoundary {
    callee: Option<hir::HirId>,
    preserve_effect: bool,
    span: Span,
}
impl ExpressionBoundary {
    fn enter(checker: &mut Checker<'_, '_>, expr: &Expr<'_>, block: &hir::Block<'_>) -> Self {
        let callee = checker.comptime_callee_result;
        if callee == Some(expr.hir_id) {
            checker.comptime_callee_result = block.expr.map(|tail| tail.hir_id);
        }
        Self {
            callee,
            preserve_effect: std::mem::take(&mut checker.preserve_effect_value),
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
            "(block-delivery (source {}) (preserve-effect {}) (callee",
            source(self.span),
            self.preserve_effect
        )?;
        if let Some(callee) = self.callee {
            write!(f, " {callee}")?;
        }
        f.write_str("))")
    }
}

struct BlockFrame<'b, 'h> {
    progress: Progress<'b, 'h, Destination>,
    scope: LexicalScope,
    boundary: Option<ExpressionBoundary>,
}
impl std::fmt::Display for BlockFrame<'_, '_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "(block-frame {} {} (delivery", self.progress, self.scope)?;
        if let Some(boundary) = &self.boundary {
            write!(f, " {boundary}")?;
        }
        f.write_str("))")
    }
}
impl BlockFrame<'_, '_> {
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
pub(super) enum Request<'b, 'h> {
    Expression(&'b Expr<'h>, Destination),
    Block(&'b hir::Block<'h>, Destination),
}

pub(super) enum Step<'b, 'h> {
    Continue,
    Child(Request<'b, 'h>),
    Tail(destinations::CheckedTail),
    Conditional(CheckedBlock),
}

impl std::fmt::Display for Request<'_, '_> {
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

impl std::fmt::Display for Step<'_, '_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Continue => f.write_str("(check-step continue)"),
            Self::Child(request) => write!(f, "(check-step {request})"),
            Self::Tail(value) => write!(f, "(check-step (tail {value}))"),
            Self::Conditional(value) => write!(f, "(check-step (conditional {value}))"),
        }
    }
}

enum RegionFrame<'b, 'h> {
    Block(BlockFrame<'b, 'h>),
    Conditional(conditionals::Frame<'b, 'h>),
}

impl RegionFrame<'_, '_> {
    fn unwind(self, checker: &mut Checker<'_, '_>) {
        match self {
            Self::Block(frame) => frame.unwind(checker),
            Self::Conditional(frame) => frame.unwind(checker),
        }
    }
}

impl std::fmt::Display for RegionFrame<'_, '_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Block(frame) => frame.fmt(f),
            Self::Conditional(frame) => frame.fmt(f),
        }
    }
}

impl<'b, 'h> BlockFrame<'b, 'h> {
    fn step(
        &mut self,
        checker: &mut Checker<'_, '_>,
        returned: Option<CheckedBlock>,
        statement_destination: &Destination,
    ) -> Result<Step<'b, 'h>, Error> {
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
                        self.progress.block,
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
                    Ok(Step::Child(Request::Expression(
                        statement,
                        statement_destination.clone(),
                    )))
                }
                None => Ok(Step::Continue),
            },
            Phase::Tail => {
                let tail = self.progress.request_tail()?.clone();
                match tail.prepare(checker, self.progress.block, self.progress.diverges)? {
                    tails::TailCheck::Checked(value) => tail
                        .complete(checker, self.progress.block, self.progress.diverges, value)
                        .map(Step::Tail),
                    tails::TailCheck::Child => Ok(Step::Child(Request::Expression(
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

impl Checker<'_, '_> {
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

    fn drive_check_region(&mut self, root: RegionFrame<'_, '_>) -> Result<CheckedBlock, Error> {
        let mut frames = vec![root];
        let statement_destination = Destination::statement();
        let result = (|| {
            let mut returned: Option<CheckedBlock> = None;
            loop {
                let step = match frames.last_mut().expect("active checking frame") {
                    RegionFrame::Block(frame) => {
                        frame.step(self, returned.take(), &statement_destination)?
                    }
                    RegionFrame::Conditional(frame) => frame.step(self, returned.take())?,
                };
                let request = match step {
                    Step::Continue => continue,
                    Step::Tail(tail) => {
                        let RegionFrame::Block(frame) = frames.pop().unwrap() else {
                            unreachable!("only blocks complete a tail")
                        };
                        let value = frame.finish(self, tail)?;
                        if frames.is_empty() {
                            return Ok(value);
                        }
                        returned = Some(value);
                        continue;
                    }
                    Step::Conditional(value) => {
                        // Completion has already left the final branch premise.
                        frames.pop().unwrap().unwind(self);
                        if frames.is_empty() {
                            return Ok(value);
                        }
                        returned = Some(value);
                        continue;
                    }
                    Step::Child(request) => request,
                };
                let (block, destination, boundary) = match request {
                    Request::Block(block, destination) => (block, destination, None),
                    Request::Expression(expr, destination) => {
                        // Partial structural inputs retain their own rule.
                        // Ordinary branches enter only after delivery prepares
                        // the complete target, as in expr_context.
                        if let Destination::Contextual(tail) = &destination
                            && matches!(expr.kind, H::If(_, _, Some(_)))
                        {
                            frames.push(RegionFrame::Conditional(conditionals::Frame::new(
                                self,
                                expr,
                                tail.depth + 1,
                            )?));
                            continue;
                        }
                        if matches!(expr.kind, H::If(..))
                            && let Some(view) = destination.ordinary_view()
                            && view.input.is_none()
                            && self.prepare_region_if_delivery(expr, view.expected, view.partial)?
                        {
                            frames.push(RegionFrame::Conditional(conditionals::Frame::ordinary(
                                self,
                                expr,
                                destination,
                                true,
                            )?));
                            continue;
                        }
                        if let Some(nested) = destination.nested(expr)? {
                            let H::Block(block) = expr.kind else {
                                unreachable!("nested block destination")
                            };
                            let boundary = matches!(nested, Destination::Ordinary(_))
                                .then(|| ExpressionBoundary::enter(self, expr, block));
                            (block, nested, boundary)
                        } else {
                            returned = Some(destination.check_expression(self, expr)?);
                            continue;
                        }
                    }
                };
                let scope = match self
                    .charge_block_frame(block.span)
                    .and_then(|()| LexicalScope::enter(self, block.span))
                {
                    Ok(scope) => scope,
                    Err(error) => {
                        if let Some(boundary) = &boundary {
                            boundary.leave(self);
                        }
                        return Err(error);
                    }
                };
                frames.push(RegionFrame::Block(BlockFrame {
                    progress: Progress::new(block, destination),
                    scope,
                    boundary,
                }));
            }
        })();
        // Unwind inner lexical scopes before their enclosing branch premises.
        // This restores checking context without rolling back outer writes.
        while let Some(frame) = frames.pop() {
            frame.unwind(self);
        }
        result
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
            locals: BTreeMap::from([(
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
            code: BTreeMap::new(),
            patterns: BTreeMap::new(),
            context: crate::ptc::Context::from_package(&n::Package::default()).unwrap(),
            inputs: input_facts::State::default().lexical_boundary(),
        };
        let mut frame = BlockFrame {
            progress: Progress::new(&block, Destination::statement()),
            scope,
            boundary: Some(ExpressionBoundary {
                callee: Some(id),
                preserve_effect: true,
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
        assert_eq!(
            tails::TailCheck::<n::Expr>::Child.to_string(),
            "(tail-check child)"
        );
    }
}
