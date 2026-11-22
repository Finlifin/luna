//! Run selection and the chosen arm's producers in the source frame. Carry
//! only its ordinal and checked captures into the constructor's type scope.
use super::*;

#[derive(Default)]
struct Choice {
    initializers: Vec<n::Expr>,
    exports: Vec<Capture>,
    arms: Vec<n::MatchArm>,
}

impl Checker<'_, '_> {
    pub(super) fn capture_forward_choice(
        &mut self,
        operand: &mut n::Expr,
        schedule: &OperandSchedule,
        span: Span,
        depth: usize,
    ) -> Result<Capture, Error> {
        let mut choice = Choice::default();
        match &mut operand.kind {
            K::If {
                then_branch,
                else_branch: Some(other),
                ..
            } => {
                choice.split_arm(self, then_branch, schedule, span, depth)?;
                choice.split_arm(self, other, schedule, span, depth)?;
            }
            K::Match { arms, .. } => {
                for arm in arms {
                    choice.split_arm(self, &mut arm.body, schedule, span, depth)?;
                }
            }
            _ => unreachable!(),
        }

        let source = operand.source;
        let tag = self.spread_binding(span)?;
        let selected = n::Expr {
            source,
            ty: p(P::Integer),
            kind: std::mem::replace(
                &mut operand.kind,
                K::Match {
                    value: Box::new(local_read(tag, p(P::Integer), source)),
                    arms: choice.arms,
                },
            ),
        };
        // This walk includes observations embedded in a pattern and its guard.
        // They can use pattern bindings but cannot read a later witness.
        if !self
            .checked_declaration_inputs(&selected, span)?
            .is_disjoint(&schedule.unavailable)
            || schedule.reads_deferred(self, &selected, span)?
        {
            return Err(needs_frame(span, "branch selection needs a later input"));
        }
        if choice.exports.is_empty() {
            return Ok(Capture {
                binding: tag,
                value: selected,
            });
        }
        // The first tuple field runs the decision. Subsequent fields read
        // private cells after the chosen arm has populated its own cell.
        let mut fields = Vec::with_capacity(1 + choice.exports.len());
        fields.push(Capture {
            binding: tag,
            value: selected,
        });
        fields.extend(choice.exports);
        let (binding, packet) = self.bundle_block_captures(fields, operand, span)?;
        let ty = declaration_inputs::copy_input_domain(&packet.ty, &mut self.required_steps, span)?;
        Ok(Capture {
            binding,
            value: n::Expr {
                source,
                ty,
                kind: K::Block {
                    statements: choice.initializers,
                    result: Some(Box::new(packet)),
                },
            },
        })
    }
}

impl Choice {
    fn split_arm(
        &mut self,
        checker: &mut Checker<'_, '_>,
        body: &mut n::Expr,
        schedule: &OperandSchedule,
        span: Span,
        depth: usize,
    ) -> Result<(), Error> {
        dependent_inputs::tasks::spend(&mut checker.required_steps, 1, span)?;
        if is_never(&body.ty) {
            // This arm transfers in the original frame and cannot yield a tag.
            return Ok(());
        }
        let source = body.source;
        let mut captures = Vec::new();
        checker.capture_forward_children(body, schedule, &mut captures, span, depth + 1)?;
        let payload = match captures.len() {
            0 => None,
            1 => captures.pop(),
            _ => {
                let (binding, value) = checker.bundle_block_captures(captures, body, span)?;
                Some(Capture { binding, value })
            }
        };

        let literal = n::Literal::Integer(self.arms.len().to_string());
        let selected = n::Expr {
            source,
            ty: p(P::Integer),
            kind: K::Literal(literal.clone()),
        };
        let mut residual = std::mem::replace(body, selected);
        if let Some(Capture { binding, value }) = payload {
            let optional = Term::Optional(Box::new(declaration_inputs::copy_input_domain(
                &value.ty,
                &mut checker.required_steps,
                span,
            )?));
            let cell = checker.spread_binding(span)?;
            let export = checker.spread_binding(span)?;
            let mut copy_domain = || {
                declaration_inputs::copy_input_domain(&optional, &mut checker.required_steps, span)
            };
            self.initializers.push(n::Expr {
                source,
                ty: p(P::Void),
                kind: K::Let {
                    binding: cell,
                    value: Box::new(n::Expr {
                        source,
                        ty: copy_domain()?,
                        kind: K::OptionalAbsent,
                    }),
                },
            });
            let write = n::Expr {
                source,
                ty: p(P::Void),
                kind: K::Assign {
                    place: n::Place {
                        root: cell,
                        fields: vec![],
                    },
                    value: Box::new(n::Expr {
                        source,
                        ty: copy_domain()?,
                        kind: K::OptionalPresent(Box::new(value)),
                    }),
                },
            };
            self.exports.push(Capture {
                binding: export,
                value: local_read(cell, copy_domain()?, source),
            });
            let selected = std::mem::replace(&mut body.kind, K::Unit);
            body.kind = K::Block {
                statements: vec![write],
                result: Some(Box::new(n::Expr {
                    source,
                    ty: p(P::Integer),
                    kind: selected,
                })),
            };
            // Only the arm that wrote this cell can select this reconstruction.
            let ty = declaration_inputs::copy_input_domain(
                &residual.ty,
                &mut checker.required_steps,
                span,
            )?;
            residual = n::Expr {
                source,
                ty,
                kind: K::Match {
                    value: Box::new(local_read(export, optional, source)),
                    arms: vec![n::MatchArm {
                        pattern: n::Pattern::OptionalPresent(Box::new(n::Pattern::Binding {
                            binding,
                            inner: Box::new(n::Pattern::Wild),
                        })),
                        guard: None,
                        body: residual,
                    }],
                },
            };
        }
        self.arms.push(n::MatchArm {
            pattern: n::Pattern::Literal(literal),
            guard: None,
            body: residual,
        });
        Ok(())
    }
}

fn local_read(binding: n::LocalId, ty: Term, source: n::SourceRange) -> n::Expr {
    n::Expr {
        source,
        ty,
        kind: K::Local(binding),
    }
}
