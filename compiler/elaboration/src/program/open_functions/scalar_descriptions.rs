//! Scalar description rules share the ordinary checker without retaining the
//! large call/control dispatcher frame at every operand nesting level.
use super::*;

impl Checker<'_, '_> {
    #[inline(never)]
    pub(super) fn describe_open_scalar(
        &mut self,
        expr: &Expr<'_>,
        locals: &mut BTreeMap<n::LocalId, D>,
        returns: Option<&D>,
        expected: Option<&D>,
        depth: usize,
    ) -> Result<D, Error> {
        let result = match &expr.kind {
            H::Unary(UnOp::Not | UnOp::Neg, operand) if !is_numeric_literal(operand) => {
                self.describe_open_unary(expr, locals, returns, expected, depth)?
            }
            H::Binary(
                op @ (BinOp::Eq
                | BinOp::Ne
                | BinOp::Lt
                | BinOp::Le
                | BinOp::Gt
                | BinOp::Ge
                | BinOp::Add
                | BinOp::Sub
                | BinOp::Mul
                | BinOp::Div
                | BinOp::Rem
                | BinOp::Concat),
                left,
                right,
            ) if !self.open_condition_has_type_evidence(expr, 0)
                && !(is_numeric_literal(left) && is_numeric_literal(right)) =>
            {
                let arithmetic = match op {
                    BinOp::Add => Some(crate::arithmetic::Operator::Add),
                    BinOp::Sub => Some(crate::arithmetic::Operator::Subtract),
                    BinOp::Mul => Some(crate::arithmetic::Operator::Multiply),
                    BinOp::Div => Some(crate::arithmetic::Operator::Divide),
                    BinOp::Rem => Some(crate::arithmetic::Operator::Remainder),
                    _ => None,
                };
                let protocol = arithmetic.and_then(|op| self.protocols.arithmetic[op as usize]);
                // A literal left operand has no independent protocol receiver
                // domain yet. Use the ordinary arithmetic peer hint rather than
                // defaulting an open right subtree before its destination arrives.
                let peer_context = if arithmetic.is_some() && is_numeric_literal(left) {
                    match expected {
                        Some(D::Known(ty)) => self
                            .arithmetic_operand_hint(Some(ty), expr.span)?
                            .map(D::Known),
                        _ => None,
                    }
                } else {
                    None
                };
                let operand_context = if arithmetic.is_some() && protocol.is_none() {
                    expected
                } else {
                    peer_context.as_ref()
                };
                // Keep numeric literals in the probe so peer inference, exact
                // range checks and signed literal formation remain unchanged.
                if is_numeric_literal(left) {
                    let description = self.describe_open_expression(
                        right,
                        locals,
                        returns,
                        operand_context,
                        depth + 1,
                    )?;
                    let D::Known(ty) = description else {
                        return Err(error(
                            right.span,
                            "binary operation requires established operand types",
                        ));
                    };
                    self.with_open_operand(ty, right.span, |this, right| {
                        let probe = Expr {
                            hir_id: expr.hir_id,
                            span: expr.span,
                            kind: H::Binary(*op, left, right),
                        };
                        this.describe_known_region(&probe, expected)
                    })?
                } else {
                    let description = self.describe_open_expression(
                        left,
                        locals,
                        returns,
                        operand_context,
                        depth + 1,
                    )?;
                    let D::Known(left_type) = description else {
                        return Err(error(
                            left.span,
                            "binary operation requires established operand types",
                        ));
                    };
                    if left_type == p(P::Type) {
                        return self.describe_known_region(expr, expected);
                    }
                    if is_numeric_literal(right) {
                        self.with_open_operand(left_type, left.span, |this, left| {
                            let probe = Expr {
                                hir_id: expr.hir_id,
                                span: expr.span,
                                kind: H::Binary(*op, left, right),
                            };
                            this.describe_known_region(&probe, expected)
                        })?
                    } else {
                        let context =
                            (protocol.is_none() && *op != BinOp::Concat && !is_never(&left_type))
                                .then(|| D::Known(left_type.clone()));
                        let description = self.with_open_reachability(
                            !is_never(&left_type),
                            right.span,
                            |this| {
                                this.describe_open_expression(
                                    right,
                                    locals,
                                    returns,
                                    context.as_ref(),
                                    depth + 1,
                                )
                            },
                        )?;
                        let D::Known(right_type) = description else {
                            return Err(error(
                                right.span,
                                "binary operation requires established operand types",
                            ));
                        };
                        self.with_open_operand(left_type, left.span, |this, left| {
                            this.with_open_operand(right_type, right.span, |this, right| {
                                let probe = Expr {
                                    hir_id: expr.hir_id,
                                    span: expr.span,
                                    kind: H::Binary(*op, left, right),
                                };
                                this.describe_known_region(&probe, expected)
                            })
                        })?
                    }
                }
            }
            H::Binary(op @ (BinOp::And | BinOp::Or), left, right) => {
                let left_description =
                    self.describe_open_expression(left, locals, returns, expected, depth + 1)?;
                let D::Known(left_type) = left_description else {
                    return Err(error(left.span, "and/or require established operand types"));
                };
                let shape = self.structural_type(&left_type, left.span)?;
                if !matches!(shape, Term::Primitive(P::Bool | P::BitVec(_) | P::Never)) {
                    return Err(error(
                        expr.span,
                        "and/or require bool or equal-width BitVec operands",
                    ));
                }
                let choice = control::source_boolean(left, &mut self.required_steps, 0);
                self.required_steps = self
                    .required_steps
                    .checked_sub(self.open_loop_transfers.len() as u64)
                    .ok_or_else(|| {
                        error(expr.span, "loop completion description budget exhausted")
                    })?;
                let left_transfers = self.open_loop_transfers.clone();
                let right_type = if is_never(&shape) {
                    p(P::Bool)
                } else {
                    shape.clone()
                };
                let mut check = |this: &mut Self| {
                    this.describe_open_expression(
                        right,
                        locals,
                        returns,
                        Some(&D::Known(right_type.clone())),
                        depth + 1,
                    )
                };
                let right_description =
                    if shape == p(P::Bool) && self.open_condition_has_type_evidence(left, 0) {
                        let positive = *op == BinOp::And;
                        let scope = if self.in_staged_input {
                            let fact = self.staged_condition_fact(left, positive, 0)?;
                            self.enter_condition_fact(fact)
                        } else {
                            let predicate = self.expr(left, Some(&p(P::Bool)))?;
                            self.enter_source_type_condition(
                                Some(left),
                                &predicate,
                                positive,
                                right.span,
                            )?
                        };
                        let result = check(self);
                        scope.leave(self);
                        result?
                    } else {
                        check(self)?
                    };
                let D::Known(right_description) = right_description else {
                    return Err(error(
                        right.span,
                        "and/or require established operand types",
                    ));
                };
                if is_never(&shape) || (shape == p(P::Bool) && choice == Some(*op != BinOp::And)) {
                    self.open_loop_transfers = left_transfers;
                }
                let guaranteed_right = matches!(shape, Term::Primitive(P::BitVec(_)))
                    || choice == Some(*op == BinOp::And);
                D::Known(
                    if is_never(&shape) || (guaranteed_right && is_never(&right_description)) {
                        p(P::Never)
                    } else {
                        shape
                    },
                )
            }
            _ => self.describe_known_region(expr, expected)?,
        };
        self.require_description(&result, expected, expr.span)?;
        Ok(result)
    }
}
