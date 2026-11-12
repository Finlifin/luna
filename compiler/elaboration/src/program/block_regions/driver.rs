//! Block checking owns its unfinished children and lexical exits.
//! Nested block rules use this same stack; other expression rules remain atomic
//! adapters until their continuations can be handed back to the region.
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

struct Frame<'b, 'h, 't> {
    progress: Progress<'b, 'h, Destination<'t>>,
    scope: LexicalScope,
    boundary: Option<ExpressionBoundary>,
}
impl std::fmt::Display for Frame<'_, '_, '_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "(block-frame {} {} (delivery", self.progress, self.scope)?;
        if let Some(boundary) = &self.boundary {
            write!(f, " {boundary}")?;
        }
        f.write_str("))")
    }
}
impl Frame<'_, '_, '_> {
    fn finish(
        self,
        checker: &mut Checker<'_, '_>,
        value: destinations::CheckedTail,
    ) -> Result<CheckedBlock, Error> {
        let expected = self.progress.tail.expected();
        let result = self.progress.finish(checker, value);
        self.scope.leave(checker);
        if let Some(boundary) = self.boundary {
            boundary.leave(checker);
            let value = result?.ordinary(boundary.span)?;
            if !boundary.preserve_effect {
                return checker
                    .run_computation(value, expected, boundary.span)
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
        self.check_block_region(block, Destination::Ordinary(tail))?
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

    fn check_block_region(
        &mut self,
        block: &hir::Block<'_>,
        tail: Destination<'_>,
    ) -> Result<CheckedBlock, Error> {
        self.charge_block_frame(block.span)?;
        let scope = LexicalScope::enter(self, block.span)?;
        let mut frames = vec![Frame {
            progress: Progress::new(block, tail),
            scope,
            boundary: None,
        }];
        let result = (|| {
            let mut returned: Option<CheckedBlock> = None;
            loop {
                let frame = frames.last_mut().expect("active block frame");
                if let Some(value) = returned.take() {
                    match frame.progress.phase {
                        Phase::Child => frame
                            .progress
                            .accept(self, value.ordinary(frame.progress.block.span)?)?,
                        Phase::TailChild => {
                            let checked = frame.progress.tail.complete(
                                self,
                                frame.progress.block,
                                frame.progress.diverges,
                                Some(value),
                            )?;
                            let value = frames.pop().unwrap().finish(self, checked)?;
                            if frames.is_empty() {
                                return Ok(value);
                            }
                            returned = Some(value);
                        }
                        _ => {
                            return Err(error(
                                frame.progress.block.span,
                                "block received an unsolicited child",
                            ));
                        }
                    }
                    continue;
                }
                let request = match frame.progress.phase {
                    Phase::Statements => match frame.progress.next()? {
                        Some(statement) => {
                            self.required_steps =
                                self.required_steps.checked_sub(1).ok_or_else(|| {
                                    error(statement.span, "block checking budget exhausted")
                                })?;
                            (
                                statement,
                                Destination::Ordinary(OrdinaryTail {
                                    expected: None,
                                    partial: None,
                                    input: None,
                                }),
                            )
                        }
                        None => continue,
                    },
                    Phase::Tail => {
                        let tail = *frame.progress.request_tail()?;
                        match tail.prepare(self, frame.progress.block, frame.progress.diverges)? {
                            tails::TailCheck::Checked(value) => {
                                let checked = tail.complete(
                                    self,
                                    frame.progress.block,
                                    frame.progress.diverges,
                                    value,
                                )?;
                                let value = frames.pop().unwrap().finish(self, checked)?;
                                if frames.is_empty() {
                                    return Ok(value);
                                }
                                returned = Some(value);
                                continue;
                            }
                            tails::TailCheck::Child => (frame.progress.block.expr.unwrap(), tail),
                        }
                    }
                    _ => {
                        return Err(error(
                            frame.progress.block.span,
                            "block child result is missing",
                        ));
                    }
                };
                let (expr, destination) = request;
                // Plain blocks retain their ordinary or contextual destination.
                // Partial structural input and unsafe adaptation still enter
                // their existing rules; they cannot masquerade as this rule.
                if let Some(nested) = destination.nested(expr)? {
                    let H::Block(block) = expr.kind else {
                        unreachable!("nested block destination")
                    };
                    let boundary = matches!(nested, Destination::Ordinary(_))
                        .then(|| ExpressionBoundary::enter(self, expr, block));
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
                    frames.push(Frame {
                        progress: Progress::new(block, nested),
                        scope,
                        boundary,
                    });
                } else {
                    returned = Some(destination.check_expression(self, expr)?);
                }
            }
        })();
        // A diagnostic consumes the task too. Restore every entered lexical
        // boundary exactly once without resurrecting invalidated outer facts.
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
        let mut frame = Frame {
            progress: Progress::new(
                &block,
                Destination::Ordinary(OrdinaryTail {
                    expected: None,
                    partial: None,
                    input: None,
                }),
            ),
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
