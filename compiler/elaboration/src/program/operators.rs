//! Scalar operator recursion stays outside the large general expression dispatcher.
//! Keep these entry frames separate even in unoptimized compiler builds.
use super::*;

pub(super) fn native_arithmetic_operand(op: &BinOp, shape: &Term) -> bool {
    match shape {
        Term::Primitive(P::F32 | P::F64) => *op != BinOp::Rem,
        Term::Primitive(P::Integer | P::Float) => !matches!(op, BinOp::Div | BinOp::Rem),
        Term::Primitive(primitive) => primitive
            .fixed_integer_layout()
            .is_some_and(|(signed, _)| !signed || !matches!(op, BinOp::Div | BinOp::Rem)),
        _ => false,
    }
}

impl Checker<'_, '_> {
    /// Divergence contributes no argument type. Recover a uniquely established
    /// protocol input instead of querying for an implementation on Never.
    pub(super) fn finish_binary_protocol_call(
        &mut self,
        expr: &Expr<'_>,
        protocol: DeclarationId,
        method: &str,
        receiver: n::Expr,
        argument: n::Expr,
        expected: Option<&Term>,
    ) -> Result<n::Expr, Error> {
        let argument_type = if is_never(&argument.ty) {
            self.protocol_argument_context(&receiver.ty, protocol, expr.span)?
                .ok_or_else(|| {
                    error(
                        expr.span,
                        "diverging operator argument has no unique protocol context",
                    )
                })?
        } else {
            argument.ty.clone()
        };
        let callee = self
            .selected_method(
                receiver.ty.clone(),
                Some(n::InterfaceInstance {
                    declaration: protocol,
                    arguments: vec![argument_type],
                }),
                method,
                expr.span,
            )?
            .ok_or_else(|| error(expr.span, "operator protocol has no selected method"))?;
        self.finish_call(expr, callee, &[], vec![receiver, argument], expected, None)
    }

    #[inline(never)]
    pub(super) fn unary_expression(
        &mut self,
        expr: &Expr<'_>,
        expected: Option<&Term>,
    ) -> Result<n::Expr, Error> {
        let (operator, operand) = match &expr.kind {
            H::Unary(UnOp::Neg, value) => (crate::arithmetic::Operator::Negate, *value),
            H::Unary(UnOp::Not, value) => (crate::arithmetic::Operator::Invert, *value),
            _ => unreachable!("unary operator dispatcher"),
        };
        // Keep signed literal formation intact: the positive magnitude of a
        // minimum signed value need not fit its final negative type.
        let signed_literal =
            operator == crate::arithmetic::Operator::Negate && is_numeric_literal(operand);
        let mut checked = None;
        if !signed_literal {
            if let Some(protocol) = self.protocols.arithmetic[operator as usize] {
                checked = self.argument_attempt(operand, None, None)?;
                if let Some(value) = checked.as_ref().filter(|value| !is_never(&value.ty)) {
                    let shape = match self
                        .context
                        .structural_view_metered(&value.ty, crate::ptc::Budget::default())
                        .0
                    {
                        Ok(view) => view.map(|(ty, _)| ty).unwrap_or_else(|| value.ty.clone()),
                        Err(crate::ptc::UnknownReason::UnvalidatedModel) => value.ty.clone(),
                        Err(reason) => {
                            return Err(error(
                                expr.span,
                                format!("unary operand view remains unresolved: {reason:?}"),
                            ));
                        }
                    };
                    let native = match operator {
                        crate::arithmetic::Operator::Negate => {
                            matches!(
                                shape,
                                Term::Primitive(P::Integer | P::Float | P::F32 | P::F64)
                            ) || matches!(shape,Term::Primitive(primitive) if primitive.fixed_integer_layout().is_some())
                        }
                        crate::arithmetic::Operator::Invert => {
                            matches!(shape, Term::Primitive(P::Bool | P::BitVec(_)))
                        }
                        _ => unreachable!(),
                    };
                    if !native {
                        let callee = self
                            .selected_method(
                                value.ty.clone(),
                                Some(n::InterfaceInstance {
                                    declaration: protocol,
                                    arguments: vec![],
                                }),
                                operator.method(),
                                expr.span,
                            )?
                            .ok_or_else(|| {
                                error(expr.span, "unary operator protocol has no selected method")
                            })?;
                        return self.finish_call(
                            expr,
                            callee,
                            &[],
                            vec![checked.take().unwrap()],
                            expected,
                            None,
                        );
                    }
                }
            }
        }
        let (ty, kind) = match &expr.kind {
            H::Unary(UnOp::Neg, value)
                if matches!(
                    &value.kind,
                    H::Lit(hir::Lit {
                        kind: hir::LitKind::Integer(_),
                        ..
                    })
                ) =>
            {
                let H::Lit(hir::Lit {
                    kind: hir::LitKind::Integer(spelling),
                    ..
                }) = &value.kind
                else {
                    unreachable!()
                };
                let value = self.integer_proxy(spelling, true, expected, expr.span)?;
                (value.ty, value.kind)
            }
            H::Unary(UnOp::Neg, value)
                if matches!(
                    &value.kind,
                    H::Lit(hir::Lit {
                        kind: hir::LitKind::Float(_),
                        ..
                    })
                ) =>
            {
                let H::Lit(hir::Lit {
                    kind: hir::LitKind::Float(spelling),
                    ..
                }) = &value.kind
                else {
                    unreachable!()
                };
                let value = self.float_proxy(spelling, true, expected, expr.span)?;
                (value.ty, value.kind)
            }
            H::Unary(UnOp::Neg, value) => {
                let value = match checked.take() {
                    Some(value) => value,
                    None => self.expr(value, expected)?,
                };
                let shape = self.structural_type(&value.ty, expr.span)?;
                let kind = if matches!(shape, Term::Primitive(P::Float | P::F32 | P::F64)) {
                    K::FloatNegate(Box::new(value))
                } else {
                    if !matches!(shape, Term::Primitive(p) if p.fixed_integer_layout().is_some()) {
                        self.require(&value.ty, &p(P::Integer), expr.span)?;
                    }
                    K::IntegerNegate(Box::new(value))
                };
                (shape, kind)
            }
            H::Unary(UnOp::Not, value) => {
                let mut value = match checked.take() {
                    Some(value) => value,
                    None => self.expr(value, expected)?,
                };
                let ty = self.structural_type(&value.ty, expr.span)?;
                if !matches!(ty, Term::Primitive(P::Bool | P::BitVec(_) | P::Never)) {
                    return Err(error(expr.span, "not requires bool or BitVec"));
                }
                // Preserve the declaration's type, but expose its proved
                // operand shape to NIR scalar operations and program analysis.
                value.ty = ty.clone();
                (ty, K::Not(Box::new(value)))
            }
            _ => unreachable!("operator dispatcher selected an incompatible expression"),
        };
        if let Some(expected) = expected {
            self.require_expression(&ty, expected, expr.span)?;
        }
        Ok(n::Expr {
            ty,
            source: source(expr.span),
            kind,
        })
    }
    #[inline(never)]
    pub(super) fn logical_expression(
        &mut self,
        expr: &Expr<'_>,
        expected: Option<&Term>,
    ) -> Result<n::Expr, Error> {
        let (ty, kind) = match &expr.kind {
            H::Binary(op @ (BinOp::And | BinOp::Or), a, b) => {
                let mut a = self.expr(a, expected)?;
                let operand_ty = self.structural_type(&a.ty, expr.span)?;
                if !matches!(
                    operand_ty,
                    Term::Primitive(P::Bool | P::BitVec(_) | P::Never)
                ) {
                    return Err(error(
                        expr.span,
                        "and/or require bool or equal-width BitVec operands",
                    ));
                }
                let right_ty = if is_never(&operand_ty) {
                    p(P::Bool)
                } else {
                    operand_ty.clone()
                };
                let mut b = if operand_ty == p(P::Bool) {
                    self.under_type_condition(&a, *op == BinOp::And, expr.span, |checker| {
                        checker.expr(b, Some(&right_ty))
                    })?
                } else {
                    self.expr(b, Some(&right_ty))?
                };
                if !is_never(&a.ty) {
                    a.ty = operand_ty.clone();
                }
                if !is_never(&b.ty) {
                    b.ty = right_ty;
                }
                // Bool short-circuits; BitVec evaluates both operands.
                let evaluates_right = if operand_ty == p(P::Bool) {
                    super::control::boolean(&a) == Some(*op == BinOp::And)
                } else {
                    matches!(operand_ty, Term::Primitive(P::BitVec(_)))
                };
                let ty = if is_never(&a.ty) || (evaluates_right && is_never(&b.ty)) {
                    p(P::Never)
                } else {
                    operand_ty
                };
                let kind = if *op == BinOp::And {
                    K::And(Box::new(a), Box::new(b))
                } else {
                    K::Or(Box::new(a), Box::new(b))
                };
                (ty, kind)
            }
            _ => unreachable!("operator dispatcher selected an incompatible expression"),
        };
        if let Some(expected) = expected {
            self.require_expression(&ty, expected, expr.span)?;
        }
        Ok(n::Expr {
            ty,
            source: source(expr.span),
            kind,
        })
    }
    #[inline(never)]
    pub(super) fn comparison_expression(
        &mut self,
        expr: &Expr<'_>,
        expected: Option<&Term>,
    ) -> Result<n::Expr, Error> {
        let (ty, kind) = match &expr.kind {
            H::Binary(op @ (BinOp::Eq | BinOp::Ne), a, b) => {
                let (a, b) = self.peer_operands(a, b, None, None)?;
                let ty =
                    self.structural_type(if is_never(&a.ty) { &b.ty } else { &a.ty }, expr.span)?;
                let mut result = self.equality_expression(a, b, &ty, expr.span)?;
                if *op == BinOp::Ne {
                    result = n::Expr {
                        ty: result.ty.clone(),
                        source: result.source,
                        kind: K::Not(Box::new(result)),
                    };
                }
                if let Some(expected) = expected {
                    self.require(&result.ty, expected, expr.span)?;
                }
                return Ok(result);
            }

            H::Binary(op @ (BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge), left, right) => {
                let fallback =
                    (is_numeric_literal(left) && !is_float_literal(left)).then(|| p(P::Integer));
                let (left, right) = self.peer_operands(left, right, None, fallback.as_ref())?;
                let shape = self.structural_type(
                    if is_never(&left.ty) {
                        &right.ty
                    } else {
                        &left.ty
                    },
                    expr.span,
                )?;
                let result_ty = p(if is_never(&left.ty) || is_never(&right.ty) {
                    P::Never
                } else {
                    P::Bool
                });
                if shape == p(P::Integer) {
                    (
                        result_ty,
                        K::IntegerBinary {
                            op: match op {
                                BinOp::Lt => n::IntegerOp::Less,
                                BinOp::Le => n::IntegerOp::LessEqual,
                                BinOp::Gt => n::IntegerOp::Greater,
                                _ => n::IntegerOp::GreaterEqual,
                            },
                            left: Box::new(left),
                            right: Box::new(right),
                        },
                    )
                } else {
                    if !matches!(shape, Term::Primitive(primitive) if matches!(primitive, P::Char | P::Float | P::F32 | P::F64 | P::Never) || primitive.fixed_integer_layout().is_some())
                    {
                        return Err(error(
                            expr.span,
                            "ordering protocol for this type is not implemented",
                        ));
                    }
                    // IEEE unordered operands prohibit deriving > from not <=.
                    // Keep source operand order for every comparison direction.
                    let left = Box::new(left);
                    let right = Box::new(right);
                    let kind = match op {
                        BinOp::Lt => K::ScalarLess(left, right),
                        BinOp::Le => K::ScalarLessEqual(left, right),
                        BinOp::Gt => K::ScalarGreater(left, right),
                        _ => K::ScalarGreaterEqual(left, right),
                    };
                    (result_ty, kind)
                }
            }
            _ => unreachable!("operator dispatcher selected an incompatible expression"),
        };
        if let Some(expected) = expected {
            self.require_expression(&ty, expected, expr.span)?;
        }
        Ok(n::Expr {
            ty,
            source: source(expr.span),
            kind,
        })
    }
    #[inline(never)]
    pub(super) fn concat_expression(
        &mut self,
        expr: &Expr<'_>,
        expected: Option<&Term>,
    ) -> Result<n::Expr, Error> {
        let (ty, kind) = match &expr.kind {
            H::Binary(BinOp::Concat, a, b) => {
                let a = self.expr(a, None)?;
                let b = self.expr(b, None)?;
                let Term::Primitive(P::BitVec(m)) = self.structural_type(&a.ty, expr.span)? else {
                    return Err(error(expr.span, "concatenation requires BitVec operands"));
                };
                let Term::Primitive(P::BitVec(n)) = self.structural_type(&b.ty, expr.span)? else {
                    return Err(error(expr.span, "concatenation requires BitVec operands"));
                };
                let width = m
                    .checked_add(n)
                    .ok_or_else(|| error(expr.span, "BitVec width overflow"))?;
                (p(P::BitVec(width)), K::BitConcat(Box::new(a), Box::new(b)))
            }
            _ => unreachable!("operator dispatcher selected an incompatible expression"),
        };
        if let Some(expected) = expected {
            self.require_expression(&ty, expected, expr.span)?;
        }
        Ok(n::Expr {
            ty,
            source: source(expr.span),
            kind,
        })
    }
}

impl Checker<'_, '_> {
    /// Obtain a unique RHS description from existing capability evidence or
    /// subject-determined implementation heads. This never checks/evaluates
    /// the operand in candidate-local contexts or chooses by method spelling.
    pub(super) fn protocol_argument_context(
        &self,
        subject: &Term,
        protocol: DeclarationId,
        span: Span,
    ) -> Result<Option<Term>, Error> {
        use crate::ptc::{Budget, Formula, Outcome};
        let scope = self.call_inference_scope();
        let frames = if scope.is_empty() {
            vec![]
        } else {
            vec![scope.iter().map(|p| p.sort).collect()]
        };
        let mut choices = Vec::new();
        for capability in crate::capabilities::capability_sources_in_scope(
            &self.checked_traits,
            &self.capability_assumptions(),
            Budget::default(),
            &frames,
        )
        .map_err(|reason| error(span, reason))?
        {
            if capability.interface != protocol {
                continue;
            }
            if capability.subject != *subject
                && !matches!(
                    self.context.prove(
                        Formula::equal(capability.subject.clone(), subject.clone()),
                        Budget::default()
                    ),
                    Outcome::Entailed(_)
                )
            {
                continue;
            }
            if let [rhs] = capability.interface_arguments.as_slice() {
                if !choices.contains(rhs) {
                    choices.push(rhs.clone());
                }
            }
        }
        // Use the same scoped capability precedence as ordinary member lookup.
        if !choices.is_empty() {
            return Ok(if choices.len() == 1 {
                choices.pop()
            } else {
                None
            });
        }
        let interface = self
            .checked_traits
            .iter()
            .find(|item| item.declaration == protocol)
            .ok_or_else(|| error(span, "operator protocol declaration is unavailable"))?;
        let [rhs] = interface.type_parameters.as_slice() else {
            return Err(error(
                span,
                "operator protocol requires one argument type parameter",
            ));
        };
        // This rigid is an index wildcard only, never an argument submitted to
        // implementation selection or a fresh type-equality hypothesis.
        let wildcard = Term::Rigid {
            declaration: *rhs,
            sort: Sort::Type,
        };
        let candidates = self
            .implementation_index
            .candidates(Some(protocol), &[wildcard], subject, Budget::default())
            .map_err(|reason| {
                error(
                    span,
                    format!("operator context candidate query failed: {reason:?}"),
                )
            })?;
        if candidates.declarations().len() > 128 {
            return Err(error(
                span,
                "operator context candidate budget exhausted; annotate the argument",
            ));
        }
        for id in candidates.declarations() {
            let implementation = self
                .checked_implementations
                .iter()
                .find(|item| item.declaration == *id)
                .ok_or_else(|| error(span, "operator candidate has no checked implementation"))?;
            if implementation.interface != Some(protocol) {
                return Ok(None);
            }
            let [rhs] = implementation.interface_arguments.as_slice() else {
                return Ok(None);
            };
            let signature = Term::Function {
                parameters: vec![Parameter {
                    sort: Sort::Runtime,
                    ty: implementation
                        .subject
                        .abstract_parameters(&[])
                        .map_err(|e| error(span, e.to_string()))?,
                }],
                result: Box::new(
                    rhs.abstract_parameters(&[])
                        .map_err(|e| error(span, e.to_string()))?,
                ),
            }
            .abstract_types(&implementation.type_parameters)
            .map_err(|e| error(span, e.to_string()))?;
            let scheme = crate::calls::Scheme {
                type_parameters: implementation.type_parameters.len(),
                signature,
            };
            // Fresh candidate inference cannot leak substitutions into another
            // implementation. An unresolved alternative prevents guessing.
            let candidate = scheme
                .infer_scoped_types(
                    &[Some(subject.clone())],
                    None,
                    &self.context,
                    Budget::default(),
                    &scope,
                )
                .map_err(|reason| {
                    error(
                        span,
                        format!("operator argument context inference failed: {reason:?}"),
                    )
                })?;
            if candidate.engine().has_refuted_obligation() {
                continue;
            }
            let inferred = match candidate.finish() {
                Ok(inferred) => inferred,
                Err(crate::calls::Error::NeedsContext)
                | Err(crate::calls::Error::Obligations(crate::obligations::Error::Inference(
                    crate::InferenceError::Unresolved(_),
                ))) => return Ok(None),
                Err(reason) => {
                    return Err(error(
                        span,
                        format!("operator argument context inference failed: {reason:?}"),
                    ));
                }
            };
            let instance = n::InterfaceInstance {
                declaration: protocol,
                arguments: vec![inferred.result.clone()],
            };
            let resolution = self
                .resolve_trait_capability(&instance, subject, Budget::default())
                .map_err(|reason| error(span, reason))?;
            match resolution.selection {
                crate::implementation::Selection::Selected(_) => {
                    if !choices.contains(&inferred.result) {
                        choices.push(inferred.result);
                    }
                }
                crate::implementation::Selection::NoApplicableCandidate => {}
                _ => return Ok(None),
            }
        }
        Ok(if choices.len() == 1 {
            choices.pop()
        } else {
            None
        })
    }
}
