//! Separate source evaluation from checked data construction when an operand's
//! result type mentions a witness supplied later in the same call.
use super::*;

struct Capture {
    binding: n::LocalId,
    value: n::Expr,
}

impl Checker<'_, '_> {
    /// Leave the data constructor on the normalized call. Its independent
    /// children (including local reads) must run at the original operand slot.
    pub(super) fn prepare_forward_operand(
        &mut self,
        operand: &mut n::Expr,
        schedule: &mut OperandSchedule,
        span: Span,
    ) -> Result<(), Error> {
        let mut captures = Vec::new();
        self.capture_forward_children(operand, schedule, &mut captures, span, 0)?;
        for Capture { binding, value } in captures {
            schedule.enter(self, &value, span)?;
            schedule
                .current
                .preparations
                .push(n::NormalFormPreparation {
                    before: schedule.current.arguments.len(),
                    expression: n::Expr {
                        source: value.source,
                        ty: if is_never(&value.ty) {
                            p(P::Never)
                        } else {
                            p(P::Void)
                        },
                        kind: K::Let {
                            binding,
                            value: Box::new(value),
                        },
                    },
                });
        }
        Ok(())
    }

    fn capture_forward_children(
        &mut self,
        operand: &mut n::Expr,
        schedule: &OperandSchedule,
        captures: &mut Vec<Capture>,
        span: Span,
        depth: usize,
    ) -> Result<(), Error> {
        if depth > 128 {
            return Err(error(
                span,
                "forward operand construction nesting exhausted",
            ));
        }
        if matches!(operand.kind, K::Local(id) if schedule.deferred_bindings.contains(&id)) {
            return Ok(());
        }
        if self
            .checked_declaration_inputs(operand, span)?
            .is_disjoint(&schedule.unavailable)
            && !schedule.reads_deferred(self, operand, span)?
        {
            let binding = self.spread_binding(span)?;
            let read = n::Expr {
                source: operand.source,
                ty: declaration_inputs::copy_input_domain(
                    &operand.ty,
                    &mut self.required_steps,
                    span,
                )?,
                kind: K::Local(binding),
            };
            captures.push(Capture {
                binding,
                value: std::mem::replace(operand, read),
            });
            return Ok(());
        }

        match &mut operand.kind {
            K::OptionalAbsent => Ok(()),
            K::OptionalPresent(inner) => {
                self.capture_forward_children(inner, schedule, captures, span, depth + 1)
            }
            K::Tuple(fields) | K::AnonymousRecord { fields } => {
                for field in fields {
                    self.capture_forward_children(field, schedule, captures, span, depth + 1)?;
                }
                Ok(())
            }
            K::Block {
                statements,
                result: Some(result),
            } => {
                // Keep the original scope intact, including early exits and
                // locals used by the tail. Only selected values leave the block.
                for statement in statements.iter() {
                    if !self
                        .checked_declaration_inputs(statement, span)?
                        .is_disjoint(&schedule.unavailable)
                        || schedule.reads_deferred(self, statement, span)?
                    {
                        return Err(needs_frame(span, "statement needs a later type input"));
                    }
                }
                let mut children = Vec::new();
                self.capture_forward_children(result, schedule, &mut children, span, depth + 1)?;
                let source = operand.source;
                let (binding, payload) = match children.len() {
                    0 => (
                        self.spread_binding(span)?,
                        n::Expr {
                            source,
                            ty: p(P::Void),
                            kind: K::Unit,
                        },
                    ),
                    1 => {
                        let capture = children.pop().unwrap();
                        (capture.binding, capture.value)
                    }
                    _ => self.bundle_block_captures(children, result, span)?,
                };
                let ty = if statements.iter().any(|s| is_never(&s.ty)) {
                    p(P::Never)
                } else {
                    declaration_inputs::copy_input_domain(
                        &payload.ty,
                        &mut self.required_steps,
                        span,
                    )?
                };
                let residual = std::mem::replace(result.as_mut(), payload);
                let mut block = std::mem::replace(operand, residual);
                block.ty = ty;
                captures.push(Capture {
                    binding,
                    value: block,
                });
                Ok(())
            }
            _ => Err(needs_frame(span, "operand needs a later type input")),
        }
    }

    /// Multiple captured children leave their lexical block as one value.
    /// Unpack them inside the residual constructor after the witness arrives.
    fn bundle_block_captures(
        &mut self,
        children: Vec<Capture>,
        result: &mut n::Expr,
        span: Span,
    ) -> Result<(n::LocalId, n::Expr), Error> {
        let binding = self.spread_binding(span)?;
        let source = result.source;
        let mut fields = Vec::with_capacity(children.len());
        let mut domains = Vec::with_capacity(children.len());
        let mut pattern = Vec::with_capacity(children.len());
        for child in children {
            domains.push(declaration_inputs::copy_input_domain(
                &child.value.ty,
                &mut self.required_steps,
                span,
            )?);
            fields.push(child.value);
            pattern.push(n::Pattern::Binding {
                binding: child.binding,
                inner: Box::new(n::Pattern::Wild),
            });
        }
        let ty = self.tuple_value_type(domains, span)?;
        let read = n::Expr {
            source,
            ty: declaration_inputs::copy_input_domain(&ty, &mut self.required_steps, span)?,
            kind: K::Local(binding),
        };
        let unpack = n::Expr {
            source,
            ty: p(P::Void),
            kind: K::LetPattern {
                pattern: n::Pattern::Tuple(pattern),
                value: Box::new(read),
                guard: None,
                otherwise: None,
            },
        };
        let original = n::Expr {
            source,
            ty: declaration_inputs::copy_input_domain(&result.ty, &mut self.required_steps, span)?,
            kind: std::mem::replace(&mut result.kind, K::Unit),
        };
        result.kind = K::Block {
            statements: vec![unpack],
            result: Some(Box::new(original)),
        };
        Ok((
            binding,
            n::Expr {
                source,
                ty,
                kind: K::Tuple(fields),
            },
        ))
    }
}
