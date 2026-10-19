//! Runtime slice endpoints retain source evaluation; only their width is static.
use super::*;

impl Checker<'_, '_> {
    /// Prepare the location and selectors before observing its old payload.
    /// Both the read and final write use these private, immutable values.
    pub(super) fn bit_compound_writeback(
        &mut self,
        expr: &Expr<'_>,
        op: &BinOp,
        pointer: n::Expr,
        input_width: usize,
        indices: &[&Expr<'_>],
        rhs: &Expr<'_>,
        expected: Option<&Term>,
    ) -> Result<n::Expr, Error> {
        let saved_locals = self.locals.clone();
        let rhs_source = rhs;
        let result = (|| {
            let (mut selections, width, rhs, ty) =
                self.bit_writeback_plan(expr, input_width, indices, rhs, expected, true)?;
            let mut statements = Vec::new();
            let pointer = self.prepare_bit_operand(pointer, &mut statements, expr.span)?;
            for selection in &mut selections {
                match selection {
                    n::BitSelection::Static { .. } => {}
                    n::BitSelection::Index(index) => {
                        **index = self.prepare_bit_operand(
                            (**index).clone(),
                            &mut statements,
                            expr.span,
                        )?;
                    }
                    n::BitSelection::Window { start, end, .. } => {
                        **start = self.prepare_bit_operand(
                            (**start).clone(),
                            &mut statements,
                            expr.span,
                        )?;
                        **end =
                            self.prepare_bit_operand((**end).clone(), &mut statements, expr.span)?;
                    }
                }
            }
            let Term::Pointer(storage_type) = &pointer.ty else {
                unreachable!()
            };
            let read = n::Expr {
                ty: (**storage_type).clone(),
                source: source(expr.span),
                kind: K::ReadPointer(Box::new(pointer.clone())),
            };
            let mut read = n::Expr {
                ty: p(P::BitVec(input_width)),
                source: source(expr.span),
                kind: K::TakeBits(Box::new(read)),
            };
            let mut current_width = input_width;
            for selection in &selections {
                current_width = selection
                    .output_width(current_width)
                    .ok_or_else(|| error(expr.span, "invalid prepared BitVec selection"))?;
                let kind = match selection {
                    n::BitSelection::Static { start, end } => K::BitSlice {
                        value: Box::new(read),
                        start: *start,
                        end: *end,
                    },
                    n::BitSelection::Index(index) => K::BitIndex(Box::new(read), index.clone()),
                    n::BitSelection::Window { start, end, .. } => K::BitWindow {
                        value: Box::new(read),
                        start: start.clone(),
                        end: end.clone(),
                    },
                };
                read = n::Expr {
                    ty: p(P::BitVec(current_width)),
                    source: source(expr.span),
                    kind,
                };
            }
            if statements.iter().any(|statement| is_never(&statement.ty)) {
                read.ty = p(P::Never);
            }
            let old = self.prepare_bit_operand(read, &mut statements, expr.span)?;
            let prepared_rhs = rhs
                .map(|rhs| self.prepare_bit_operand(rhs, &mut statements, expr.span))
                .transpose()?;
            let proxy = |value: &n::Expr| {
                let K::Local(binding) = value.kind else {
                    unreachable!()
                };
                let id =
                    hir::HirId::new(hir::OwnerId::INVALID, hir::ItemLocalId::new(binding.index));
                Expr {
                    hir_id: id,
                    span: expr.span,
                    kind: H::ResolvedLocal(hir::Symbol::invalid(), id),
                }
            };
            let left = proxy(&old);
            let right = prepared_rhs.as_ref().map(proxy);
            let binary = Expr {
                kind: H::Binary(*op, &left, right.as_ref().unwrap_or(rhs_source)),
                ..expr.clone()
            };
            let computed = self.expr(&binary, Some(&p(P::BitVec(width))))?;
            let ty = if is_never(&computed.ty) {
                p(P::Never)
            } else {
                ty
            };
            if let Some(expected) = expected {
                self.require_expression(&ty, expected, expr.span)?;
            }
            Ok(n::Expr {
                ty: ty.clone(),
                source: source(expr.span),
                kind: K::Block {
                    statements,
                    result: Some(Box::new(n::Expr {
                        ty,
                        source: source(expr.span),
                        kind: K::BitWritePointer {
                            pointer: Box::new(pointer),
                            selections,
                            value: Box::new(computed),
                        },
                    })),
                },
            })
        })();
        self.locals = saved_locals;
        result
    }

    fn prepare_bit_operand(
        &mut self,
        value: n::Expr,
        statements: &mut Vec<n::Expr>,
        span: Span,
    ) -> Result<n::Expr, Error> {
        let binding = self.spread_binding(span)?;
        let ty = value.ty.clone();
        self.locals.insert(binding, ty.clone());
        let index = u32::try_from(self.expansion_sources.len())
            .map_err(|_| error(span, "BitVec preparation source space exhausted"))?;
        self.expansion_sources.push(n::ExpansionSource {
            original: source(span),
            site: source(span),
        });
        let generated = n::SourceRange::generated(index);
        statements.push(n::Expr {
            ty: if is_never(&ty) {
                p(P::Never)
            } else {
                p(P::Void)
            },
            source: generated,
            kind: K::Let {
                binding,
                value: Box::new(value),
            },
        });
        Ok(n::Expr {
            ty,
            source: generated,
            kind: K::Local(binding),
        })
    }

    pub(super) fn bit_window(
        &mut self,
        expr: &Expr<'_>,
        value: n::Expr,
        start: &Expr<'_>,
        end: &Expr<'_>,
        expected: Option<&Term>,
    ) -> Result<n::Expr, Error> {
        let lower = self.bit_read_index(start)?;
        let upper = self.bit_read_index(end)?;
        self.finish_bit_window(expr, value, (start.span, end.span), lower, upper, expected)
    }

    fn finish_bit_window(
        &mut self,
        expr: &Expr<'_>,
        value: n::Expr,
        spans: (Span, Span),
        (mut lower, lo): (n::Expr, Option<usize>),
        (mut upper, hi): (n::Expr, Option<usize>),
        expected: Option<&Term>,
    ) -> Result<n::Expr, Error> {
        let Term::Primitive(P::BitVec(input_width)) = self.structural_type(&value.ty, expr.span)?
        else {
            return Err(error(expr.span, "BitVec slice requires a BitVec value"));
        };
        if let (Some(start), Some(end)) = (lo, hi) {
            if start > end || end > input_width {
                return Err(error(expr.span, "BitVec index or slice is out of bounds"));
            }
            let result = n::Expr {
                source: source(expr.span),
                ty: p(P::BitVec(end - start)),
                kind: K::BitSlice {
                    value: Box::new(value),
                    start,
                    end,
                },
            };
            if let Some(expected) = expected {
                self.require_expression(&result.ty, expected, expr.span)?;
            }
            return Ok(result);
        }
        // Closed comptime endpoints become literals; they must not be replayed
        // in the runtime node. A neighboring dynamic endpoint supplies its domain.
        if let Some(position) = lo {
            let target = if upper.ty == p(P::Never) {
                p(P::U128)
            } else {
                upper.ty.clone()
            };
            lower = self.integer_proxy(&position.to_string(), false, Some(&target), spans.0)?;
        }
        if let Some(position) = hi {
            let target = if lower.ty == p(P::Never) {
                p(P::U128)
            } else {
                lower.ty.clone()
            };
            upper = self.integer_proxy(&position.to_string(), false, Some(&target), spans.1)?;
        }
        let divergent = lower.ty == p(P::Never) || upper.ty == p(P::Never);
        let contextual = expected
            .map(|ty| self.structural_type(ty, expr.span))
            .transpose()?;
        let width = if divergent {
            0
        } else if let Some(Term::Primitive(P::BitVec(width))) = contextual {
            width
        } else {
            let mut limits = crate::verification::Limits::default();
            limits.arithmetic.steps = limits.arithmetic.steps.min(self.required_steps as usize);
            let before = limits.arithmetic.steps;
            let width =
                crate::verification::constant_integer_difference(&lower, &upper, &mut limits);
            self.required_steps -= (before - limits.arithmetic.steps) as u64;
            let width = width.map_err(|e|error(expr.span,e.to_string()))?
                    .ok_or_else(||context_error(expr.span,"BitVec slice width is not comptime-known; provide a BitVec<N> result context and establish the endpoint difference"))?;
            width.to_u128().ok().and_then(|width|usize::try_from(width).ok())
                    .ok_or_else(||error(expr.span,"BitVec slice width must be nonnegative and fit the implementation size limit"))?
        };
        if width > input_width {
            return Err(error(
                expr.span,
                "BitVec slice width exceeds the input width",
            ));
        }
        let result = n::Expr {
            source: source(expr.span),
            ty: if divergent {
                p(P::Never)
            } else {
                p(P::BitVec(width))
            },
            kind: K::BitWindow {
                value: Box::new(value),
                start: Box::new(lower),
                end: Box::new(upper),
            },
        };
        if let Some(expected) = expected {
            self.require_expression(&result.ty, expected, expr.span)?;
        }
        Ok(result)
    }

    pub(super) fn bit_writeback(
        &mut self,
        expr: &Expr<'_>,
        place: n::Place,
        storage_type: Term,
        input_width: usize,
        indices: &[&Expr<'_>],
        value: &Expr<'_>,
        expected: Option<&Term>,
    ) -> Result<n::Expr, Error> {
        let (selections, width, value, ty) =
            self.bit_writeback_plan(expr, input_width, indices, value, expected, false)?;
        let value = value.expect("ordinary bit write checks its RHS");
        let mut offset = 0usize;
        let mut all_static = true;
        for selection in &selections {
            match selection {
                n::BitSelection::Static { start, .. } => {
                    offset = offset
                        .checked_add(*start)
                        .ok_or_else(|| error(expr.span, "BitVec writeback offset overflow"))?
                }
                _ => {
                    all_static = false;
                    break;
                }
            }
        }
        let kind = if all_static {
            K::BitAssign {
                place,
                storage_type,
                start: offset,
                end: offset + width,
                value: Box::new(value),
            }
        } else {
            K::BitUpdate {
                place,
                storage_type,
                selections,
                value: Box::new(value),
            }
        };
        Ok(n::Expr {
            source: source(expr.span),
            ty,
            kind,
        })
    }
    fn bit_writeback_plan(
        &mut self,
        expr: &Expr<'_>,
        input_width: usize,
        indices: &[&Expr<'_>],
        value: &Expr<'_>,
        expected: Option<&Term>,
        compound: bool,
    ) -> Result<(Vec<n::BitSelection>, usize, Option<n::Expr>, Term), Error> {
        let mut checked_rhs = None;
        let mut width = input_width;
        let mut selections = Vec::new();
        let mut divergent = false;
        for (position, index) in indices.iter().enumerate() {
            let selection = match &index.kind {
                H::Range {
                    start: Some(lo),
                    end: Some(hi),
                    inclusive: false,
                } => {
                    // Use the same endpoint checker as reads, without observing
                    // storage. The placeholder is discarded with the typed plan.
                    let view = n::Expr {
                        source: source(index.span),
                        ty: p(P::BitVec(width)),
                        kind: K::Unit,
                    };
                    let lower = self.bit_read_index(lo)?;
                    let upper = self.bit_read_index(hi)?;
                    // Only after every endpoint has been checked may RHS staging
                    // supply a missing width. Prepared endpoints are never replayed.
                    if position + 1 == indices.len() && (lower.1.is_none() || upper.1.is_none()) {
                        checked_rhs = self.argument_attempt(value, None, None)?;
                    }
                    let rhs_context = checked_rhs
                        .as_ref()
                        .map(|value| self.structural_type(&value.ty, expr.span))
                        .transpose()?;
                    let context = rhs_context
                        .as_ref()
                        .filter(|ty| matches!(ty, Term::Primitive(P::BitVec(_))));
                    let window = self.finish_bit_window(
                        index,
                        view,
                        (lo.span, hi.span),
                        lower,
                        upper,
                        context,
                    )?;
                    divergent |= window.ty == p(P::Never);
                    match window.kind {
                        K::BitSlice { start, end, .. } => n::BitSelection::Static { start, end },
                        K::BitWindow { start, end, .. } => {
                            let selected_width = match window.ty {
                                Term::Primitive(P::BitVec(width)) => width,
                                _ => 0,
                            };
                            n::BitSelection::Window {
                                start,
                                end,
                                width: selected_width,
                            }
                        }
                        _ => unreachable!("slice checker returns a selection"),
                    }
                }
                H::Range { .. } => {
                    return Err(error(
                        index.span,
                        "BitVec slicing currently requires an explicit half-open range",
                    ));
                }
                _ => {
                    let (value, constant) = self.bit_read_index(index)?;
                    divergent |= value.ty == p(P::Never);
                    if let Some(start) = constant {
                        let end = start
                            .checked_add(1)
                            .ok_or_else(|| error(index.span, "BitVec index overflow"))?;
                        n::BitSelection::Static { start, end }
                    } else {
                        n::BitSelection::Index(Box::new(value))
                    }
                }
            };
            width = selection
                .output_width(width)
                .ok_or_else(|| error(index.span, "BitVec index or slice is out of bounds"))?;
            selections.push(selection);
        }
        if compound && checked_rhs.is_none() {
            checked_rhs = self.argument_attempt(value, None, None)?;
        }
        let value = match checked_rhs {
            Some(value) => {
                if !compound {
                    self.require_expression(&value.ty, &p(P::BitVec(width)), expr.span)?;
                }
                Some(value)
            }
            // A compound RHS may require the operator's heterogeneous input
            // context. Keep it pending until the selected old bit type is known.
            None if compound => None,
            None => Some(self.expr(value, Some(&p(P::BitVec(width))))?),
        };
        let ty = p(
            if divergent || value.as_ref().is_some_and(|value| is_never(&value.ty)) {
                P::Never
            } else {
                P::Void
            },
        );
        if let Some(expected) = expected.filter(|_| !compound) {
            self.require_expression(&ty, expected, expr.span)?;
        }

        Ok((selections, width, value, ty))
    }
    pub(super) fn bit_pointer_writeback(
        &mut self,
        expr: &Expr<'_>,
        pointer: n::Expr,
        width: usize,
        indices: &[&Expr<'_>],
        value: &Expr<'_>,
        expected: Option<&Term>,
    ) -> Result<n::Expr, Error> {
        let (selections, _, value, ty) =
            self.bit_writeback_plan(expr, width, indices, value, expected, false)?;
        let value = value.expect("ordinary bit write checks its RHS");
        Ok(n::Expr {
            source: source(expr.span),
            ty,
            kind: K::BitWritePointer {
                pointer: Box::new(pointer),
                selections,
                value: Box::new(value),
            },
        })
    }
}
