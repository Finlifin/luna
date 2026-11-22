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
        let branches = self.split_forward_branches(operand, schedule, span, depth)?;
        let pair_type = self.common_forward_packet(&branches, span)?;
        let mut choice = Choice::default();
        for (body, capture) in branches {
            choice.install_arm(self, body, capture, pair_type.as_ref(), span)?;
        }
        let source = operand.source;
        let tag = self.spread_binding(span)?;
        let ty = pair_type.unwrap_or_else(|| p(P::Integer));
        let selected = n::Expr {
            source,
            ty: declaration_inputs::copy_input_domain(&ty, &mut self.required_steps, span)?,
            kind: std::mem::replace(
                &mut operand.kind,
                K::Match {
                    value: Box::new(local_read(tag, ty, source)),
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

    fn split_forward_branches<'b>(
        &mut self,
        operand: &'b mut n::Expr,
        schedule: &OperandSchedule,
        span: Span,
        depth: usize,
    ) -> Result<Vec<(&'b mut n::Expr, Option<Capture>)>, Error> {
        let mut branches = Vec::new();
        let mut split = |body: &'b mut n::Expr| -> Result<(), Error> {
            dependent_inputs::tasks::spend(&mut self.required_steps, 1, span)?;
            if is_never(&body.ty) {
                // Transfers stay in the source decision and never return a tag.
                return Ok(());
            }
            let mut captures = Vec::new();
            self.capture_forward_children(body, schedule, &mut captures, span, depth + 1)?;
            let payload = match captures.len() {
                0 | 1 => captures.pop(),
                _ => {
                    let (binding, value) = self.bundle_block_captures(captures, body, span)?;
                    Some(Capture { binding, value })
                }
            };
            branches.push((body, payload));
            Ok(())
        };
        match &mut operand.kind {
            K::If {
                then_branch,
                else_branch: Some(other),
                ..
            } => {
                split(then_branch)?;
                split(other)?;
            }
            K::Match { arms, .. } => {
                for arm in arms {
                    split(&mut arm.body)?;
                }
            }
            _ => unreachable!(),
        }
        Ok(branches)
    }

    // Compare complete types. Equal layouts cannot authorize reusing a value
    // at a different nominal identity or a distinct invocation-dependent type.
    fn common_forward_packet(
        &mut self,
        branches: &[(&mut n::Expr, Option<Capture>)],
        span: Span,
    ) -> Result<Option<Term>, Error> {
        let Some((_, Some(first))) = branches.first() else {
            return Ok(None);
        };
        let domain = &first.value.ty;
        let same = self.with_observation_budget(span, |_, budget| {
            for (_, capture) in &branches[1..] {
                let Some(capture) = capture else {
                    return Ok(false);
                };
                nir::type_computations::charge_term(&capture.value.ty, &mut budget.steps)?;
                if capture.value.ty != *domain {
                    return Ok(false);
                }
            }
            Ok(true)
        })?;
        if !same {
            return Ok(None);
        }
        let payload =
            declaration_inputs::copy_input_domain(domain, &mut self.required_steps, span)?;
        self.tuple_value_type(vec![p(P::Integer), payload], span)
            .map(Some)
    }
}

impl Choice {
    fn install_arm(
        &mut self,
        checker: &mut Checker<'_, '_>,
        body: &mut n::Expr,
        payload: Option<Capture>,
        pair_type: Option<&Term>,
        span: Span,
    ) -> Result<(), Error> {
        let source = body.source;
        let literal = n::Literal::Integer(self.arms.len().to_string());
        let selected = n::Expr {
            source,
            ty: p(P::Integer),
            kind: K::Literal(literal.clone()),
        };
        if let Some(pair_type) = pair_type {
            let Capture { binding, value } = payload.expect("uniform payload");
            let pair = n::Expr {
                source,
                ty: declaration_inputs::copy_input_domain(
                    pair_type,
                    &mut checker.required_steps,
                    span,
                )?,
                kind: K::Tuple(vec![selected, value]),
            };
            self.arms.push(n::MatchArm {
                pattern: n::Pattern::Tuple(vec![
                    n::Pattern::Literal(literal),
                    n::Pattern::Binding {
                        binding,
                        inner: Box::new(n::Pattern::Wild),
                    },
                ]),
                guard: None,
                body: std::mem::replace(body, pair),
            });
            return Ok(());
        }
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
