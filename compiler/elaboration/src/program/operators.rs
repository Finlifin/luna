//! Scalar operator recursion stays outside the large general expression dispatcher.
//! Keep these entry frames separate even in unoptimized compiler builds.
use super::*;

pub(super) fn native_arithmetic_operand(
    op: &BinOp,
    shape: &Term,
    target: Option<nir::target::TargetData>,
) -> bool {
    match shape {
        Term::Primitive(P::F32 | P::F64) => *op != BinOp::Rem,
        Term::Primitive(P::Integer | P::Float) => !matches!(op, BinOp::Div | BinOp::Rem),
        Term::Primitive(primitive) => target
            .map_or_else(
                || primitive.fixed_integer_layout(),
                |target| target.integer_layout(*primitive),
            )
            .is_some_and(|(signed, _)| !signed || !matches!(op, BinOp::Div | BinOp::Rem)),
        _ => false,
    }
}

impl Checker<'_, '_> {
    pub(super) fn proved_integer_family(&self, ty: &Term, unsigned: bool) -> bool {
        if unsigned {
            self.proved_primitive_family(ty, &[P::U8, P::U16, P::U32, P::U64, P::U128, P::Usize])
        } else {
            self.proved_primitive_family(
                ty,
                &[
                    P::I8,
                    P::I16,
                    P::I32,
                    P::I64,
                    P::I128,
                    P::U8,
                    P::U16,
                    P::U32,
                    P::U64,
                    P::U128,
                    P::Usize,
                    P::Isize,
                ],
            )
        }
    }

    fn proved_ordered_scalar_family(&self, ty: &Term) -> bool {
        // Every member uses the same scalar comparison node. The chosen
        // concrete domain still determines signedness and IEEE unordered cases.
        self.proved_primitive_family(
            ty,
            &[
                P::Usize,
                P::Isize,
                P::Char,
                P::F32,
                P::F64,
                P::I8,
                P::I16,
                P::I32,
                P::I64,
                P::I128,
                P::U8,
                P::U16,
                P::U32,
                P::U64,
                P::U128,
            ],
        )
    }

    pub(super) fn proved_float_family(&self, ty: &Term) -> bool {
        self.proved_primitive_family(ty, &[P::F32, P::F64])
    }

    fn proved_signed_integer_family(&self, ty: &Term) -> bool {
        self.proved_primitive_family(ty, &[P::I8, P::I16, P::I32, P::I64, P::I128, P::Isize])
    }

    pub(super) fn native_arithmetic_type(&self, op: &BinOp, shape: &Term) -> bool {
        native_arithmetic_operand(op, shape, self.target)
            || self.proved_integer_family(shape, matches!(op, BinOp::Div | BinOp::Rem))
            || (*op != BinOp::Rem && self.proved_float_family(shape))
    }
    /// Classify an existing rigid type from proved alternatives. This does not
    /// add requirements or infer a domain from the calls seen so far; concrete
    /// operations still retain their substituted types during instantiation.
    pub(super) fn proved_primitive_family(&self, ty: &Term, family: &[P]) -> bool {
        matches!(
            ty,
            Term::Rigid {
                sort: Sort::Type,
                ..
            }
        ) && matches!(
            self.context.prove(
                crate::ptc::Formula::Or(
                    family
                        .iter()
                        .map(|primitive| { crate::ptc::Formula::equal(ty.clone(), p(*primitive)) })
                        .collect()
                ),
                crate::ptc::Budget::default(),
            ),
            crate::ptc::Outcome::Entailed(_)
        )
    }
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
                if checked.as_ref().is_some_and(|value| !is_never(&value.ty)) {
                    if let Some(result) = self.unary_protocol_result(
                        expr,
                        expected,
                        operator,
                        protocol,
                        &mut checked,
                    )? {
                        return Ok(result);
                    }
                }
            }
        }
        // Only an immediate literal is formed with its sign as one operation.
        // `- -1` still checks the inner negation as an ordinary operand, even
        // though literal recognition suppresses protocol lookup for both.
        let immediate_literal = operator == crate::arithmetic::Operator::Negate
            && matches!(
                operand.kind,
                H::Lit(hir::Lit {
                    kind: hir::LitKind::Integer(_) | hir::LitKind::Float(_),
                    ..
                })
            );
        if checked.is_none() && !immediate_literal {
            checked = Some(self.expr(operand, expected)?);
        }
        self.native_unary_expression(expr, expected, checked)
    }

    #[inline(never)]
    fn native_unary_expression(
        &mut self,
        expr: &Expr<'_>,
        expected: Option<&Term>,
        mut checked: Option<n::Expr>,
    ) -> Result<n::Expr, Error> {
        // The operand was elaborated in its original context, including all
        // control-flow and effect checks. When it cannot return, no unary
        // operation executes and there is no numeric/boolean input domain.
        if checked.as_ref().is_some_and(|value| is_never(&value.ty)) {
            let value = checked.take().expect("checked diverging unary operand");
            if let Some(expected) = expected {
                self.require_expression(&value.ty, expected, expr.span)?;
            }
            return Ok(value);
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
            H::Unary(UnOp::Neg, _) => {
                let value = checked.take().expect("checked unary operand");
                let shape = self.structural_type(&value.ty, expr.span)?;
                let kind = if matches!(shape, Term::Primitive(P::Float | P::F32 | P::F64))
                    || self.proved_float_family(&shape)
                {
                    K::FloatNegate(Box::new(value))
                } else {
                    if !matches!(shape, Term::Primitive(p) if self.integer_layout(p).is_some())
                        && !self.proved_signed_integer_family(&shape)
                    {
                        self.require(&value.ty, &p(P::Integer), expr.span)?;
                    }
                    K::IntegerNegate(Box::new(value))
                };
                (shape, kind)
            }
            H::Unary(UnOp::Not, _) => {
                let mut value = checked.take().expect("checked unary operand");
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
    fn unary_protocol_result(
        &mut self,
        expr: &Expr<'_>,
        expected: Option<&Term>,
        operator: crate::arithmetic::Operator,
        protocol: DeclarationId,
        checked: &mut Option<n::Expr>,
    ) -> Result<Option<n::Expr>, Error> {
        let value = checked.as_ref().expect("checked protocol operand");
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
                ) || matches!(shape,Term::Primitive(primitive) if self.integer_layout(primitive).is_some())
                    // Protocol availability must not change the established
                    // builtin operation for a proved generic scalar family.
                    || self.proved_signed_integer_family(&shape)
                    || self.proved_float_family(&shape)
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
            return self
                .finish_call(
                    expr,
                    callee,
                    &[],
                    vec![checked.take().expect("checked protocol operand")],
                    expected,
                    None,
                )
                .map(Some);
        }
        Ok(None)
    }

    #[inline(never)]
    pub(super) fn logical_expression(
        &mut self,
        expr: &Expr<'_>,
        expected: Option<&Term>,
    ) -> Result<n::Expr, Error> {
        // Check left-associated prefixes bottom-up. Keep the first prefix in
        // the ordinary input transaction: if its domain needs a later peer,
        // roll it back and retain the existing peer-inference path. A ready
        // prefix supplies exactly the evidence the recursive left check did.
        let mut pending = Vec::new();
        let mut current = expr;
        while let H::Binary(BinOp::And | BinOp::Or, left, _) = &current.kind {
            pending.push(current);
            current = left;
        }
        if pending.len() <= 1 {
            return self.logical_expression_step(expr, expected, None);
        }
        let first = pending.pop().unwrap();
        let attempted = self.checking_attempt(false, true, |checker| {
            checker.logical_expression_step(first, expected, None)
        })?;
        let Some(mut checked) = attempted else {
            return self.logical_expression_step(expr, expected, None);
        };
        while let Some(node) = pending.pop() {
            checked = self.logical_expression_step(node, expected, Some(checked))?;
        }
        Ok(checked)
    }

    #[inline(never)]
    fn logical_expression_step(
        &mut self,
        expr: &Expr<'_>,
        expected: Option<&Term>,
        checked_left: Option<n::Expr>,
    ) -> Result<n::Expr, Error> {
        let (ty, kind) = match &expr.kind {
            H::Binary(op @ (BinOp::And | BinOp::Or), a, b) => {
                // Do not default a leading integer mask before the other
                // operand can supply its BitVec width. Keep the ordinary Bool
                // path separate so its right side still sees short-circuit
                // type premises from the already checked left side.
                let left_source = a;
                let attempted = match checked_left {
                    Some(value) => Some(value),
                    None => self.argument_attempt(a, expected, None)?,
                };
                let (a, ready_right) = match attempted {
                    Some(a) => (a, None),
                    None => {
                        let (a, b) = self.peer_operands(a, b, expected, None)?;
                        (a, Some(b))
                    }
                };
                // A diverging left operand supplies no operation domain. Let
                // the right peer (or its contextual literal type) select Bool
                // versus BitVec without rechecking the already lowered prefix.
                let (mut a, ready_right, operand_ty) = if is_never(&a.ty) {
                    // The whole expression never delivers a result, so its
                    // expected type cannot constrain an already typed peer.
                    // Retain it only for a peer that still needs context.
                    let ready_right = match ready_right {
                        Some(value) => Some(value),
                        None => self.argument_attempt(b, None, None)?,
                    };
                    let (left, right) = self.peer_operands_with_ready(
                        left_source,
                        b,
                        expected,
                        None,
                        Some(a),
                        ready_right,
                    )?;
                    let domain = self.structural_type(&right.ty, expr.span)?;
                    (left, Some(right), domain)
                } else {
                    let domain = self.structural_type(&a.ty, expr.span)?;
                    (a, ready_right, domain)
                };
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
                let mut b = if let Some(b) = ready_right {
                    b
                } else if operand_ty == p(P::Bool) {
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
                // Both operands have been checked. A nonreturning left side
                // supplies no equality domain and prevents the RHS and operator
                // from executing, including the not wrapper used for !=.
                if is_never(&a.ty) {
                    if let Some(expected) = expected {
                        self.require_expression(&a.ty, expected, expr.span)?;
                    }
                    return Ok(a);
                }
                let ty = self.structural_type(&a.ty, expr.span)?;
                if !self.native_equality_shape(&ty) {
                    let protocol = self.protocols.equality.ok_or_else(|| error(expr.span,
                        "equality protocol for this type is not implemented without an Eq binding"))?;
                    let method = if *op == BinOp::Eq { "eq" } else { "ne" };
                    let callee = self
                        .selected_method(
                            a.ty.clone(),
                            Some(n::InterfaceInstance {
                                declaration: protocol,
                                arguments: vec![],
                            }),
                            method,
                            expr.span,
                        )?
                        .ok_or_else(|| {
                            error(expr.span, "Eq protocol has no selected comparison method")
                        })?;
                    let result =
                        self.finish_call(expr, callee, &[], vec![a, b], Some(&p(P::Bool)), None)?;
                    if let Some(expected) = expected {
                        self.require_expression(&result.ty, expected, expr.span)?;
                    }
                    return Ok(result);
                }
                let mut result = self.equality_expression(a, b, &ty, expr.span)?;
                // With a nonreturning RHS, equality still carries the left
                // evaluation, but no boolean value reaches a negation.
                if *op == BinOp::Ne && !is_never(&result.ty) {
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
                if is_never(&left.ty) {
                    if let Some(expected) = expected {
                        self.require_expression(&left.ty, expected, expr.span)?;
                    }
                    return Ok(left);
                }
                let shape = self.structural_type(&left.ty, expr.span)?;
                let result_ty = p(if is_never(&right.ty) {
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
                    // Comparing two already-typed platform integers does not
                    // select a layout. Concrete execution still requires a target.
                    if !matches!(shape, Term::Primitive(primitive) if matches!(primitive, P::Char | P::Float | P::F32 | P::F64 | P::Usize | P::Isize | P::Never) || self.integer_layout(primitive).is_some())
                        && !self.proved_ordered_scalar_family(&shape)
                    {
                        let protocol = self.protocols.ordering.ok_or_else(|| {
                            error(expr.span, "ordering protocol for this type is not implemented without an Ord binding")
                        })?;
                        let method = match op {
                            BinOp::Lt => "lt",
                            BinOp::Le => "le",
                            BinOp::Gt => "gt",
                            _ => "ge",
                        };
                        let callee = self
                            .selected_method(
                                left.ty.clone(),
                                Some(n::InterfaceInstance {
                                    declaration: protocol,
                                    arguments: vec![],
                                }),
                                method,
                                expr.span,
                            )?
                            .ok_or_else(|| {
                                error(expr.span, "Ord protocol has no selected comparison method")
                            })?;
                        let result = self.finish_call(
                            expr,
                            callee,
                            &[],
                            vec![left, right],
                            Some(&p(P::Bool)),
                            None,
                        )?;
                        if let Some(expected) = expected {
                            self.require_expression(&result.ty, expected, expr.span)?;
                        }
                        return Ok(result);
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
        let scope = self.call_inference_domains();
        let frames = if scope.is_empty() {
            vec![]
        } else {
            vec![scope.iter().map(|p| p.0).collect()]
        };
        let mut choices = Vec::new();
        for capability in crate::capabilities::capability_sources_with_layouts(
            &self.checked_traits,
            &self.capability_assumptions(),
            Budget::default(),
            &frames,
            self.implementation_index.parameter_domains(),
            Some(self.implementation_index.nominal_layouts()),
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
                parameter_sorts: vec![],
                type_parameters: implementation.type_parameters.len(),
                signature,
            };
            // Fresh candidate inference cannot leak substitutions into another
            // implementation. An unresolved alternative prevents guessing.
            let candidate = scheme
                .infer_sparse_scoped_types(
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
