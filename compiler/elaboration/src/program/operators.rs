//! Scalar selection and lowering share one operand-context search. Comparisons
//! and arithmetic obtain checked children from the same region driver.
use super::*;

pub(in crate::program) mod peers;

pub(super) fn is_local_place_expr(expr: &n::Expr) -> bool {
    let mut current = expr;
    loop {
        match &current.kind {
            K::Local(_) => return true,
            K::Field { value, .. } | K::AnonymousField { value, .. } => {
                current = value;
            }
            _ => return false,
        }
    }
}

pub(super) fn builtin_signed_integer_suffix(suffix: &str) -> Option<P> {
    match suffix {
        "i8" => Some(P::I8),
        "i16" => Some(P::I16),
        "i32" => Some(P::I32),
        "i64" => Some(P::I64),
        "i128" => Some(P::I128),
        "isize" => Some(P::Isize),
        _ => None,
    }
}

pub(super) fn native_arithmetic_operand(
    op: &BinOp,
    shape: &Term,
    target: Option<nir::target::TargetData>,
) -> bool {
    match shape {
        Term::Primitive(P::F16 | P::Bf16 | P::F32 | P::F64) => *op != BinOp::Rem,
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
    /// The result can supply a scalar peer context before numeric defaults are
    /// committed. Open descriptions and concrete checking must use the same hint.
    pub(super) fn arithmetic_operand_hint(
        &self,
        expected: Option<&Term>,
        span: Span,
    ) -> Result<Option<Term>, Error> {
        let hint = expected
            .map(|ty| self.structural_type(ty, span))
            .transpose()?;
        Ok(hint.filter(|ty| {
            matches!(
                ty,
                Term::Primitive(P::Float | P::F16 | P::Bf16 | P::F32 | P::F64 | P::Integer)
            ) || matches!(ty, Term::Primitive(p) if self.integer_layout(*p).is_some())
                || self.proved_integer_family(ty, false)
                || self.proved_float_family(ty)
        }))
    }

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

    pub(super) fn proved_ordered_scalar_family(&self, ty: &Term) -> bool {
        // Every member uses the same scalar comparison node. The chosen
        // concrete domain still determines signedness and IEEE unordered cases.
        self.proved_primitive_family(
            ty,
            &[
                P::Usize,
                P::Isize,
                P::Char,
                P::F16,
                P::Bf16,
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
        self.proved_primitive_family(ty, &[P::F16, P::Bf16, P::F32, P::F64])
    }

    pub(super) fn atomic_negative_signed_literal<'h>(
        &self,
        operand: &'h Expr<'h>,
        expected: Option<&Term>,
        span: Span,
    ) -> Result<Option<(&'h str, P)>, Error> {
        let H::LiteralConversion { literal, suffix } = &operand.kind else {
            return Ok(None);
        };
        let H::Lit(hir::Lit {
            kind: hir::LitKind::Integer(spelling),
            ..
        }) = &literal.kind
        else {
            return Ok(None);
        };
        let Some(prim) = builtin_signed_integer_suffix(suffix.name.as_str()) else {
            return Ok(None);
        };
        let Some(protocol) = self.protocols.literal_into else {
            return Ok(None);
        };
        if let Some(exp) = expected
            && *exp != p(prim)
        {
            let custom = self
                .resolve_trait_capability(
                    &n::InterfaceInstance {
                        declaration: protocol,
                        arguments: vec![Term::Symbol(suffix.name.as_str().into()), exp.clone()],
                    },
                    &p(P::Integer),
                    crate::ptc::Budget::default(),
                )
                .map_err(|reason| error(span, reason))?;
            if matches!(
                custom.selection,
                crate::implementation::Selection::Selected(_)
            ) {
                return Ok(None);
            }
        }
        let standard = self
            .resolve_trait_capability(
                &n::InterfaceInstance {
                    declaration: protocol,
                    arguments: vec![Term::Symbol(suffix.name.as_str().into()), p(prim)],
                },
                &p(P::Integer),
                crate::ptc::Budget::default(),
            )
            .map_err(|reason| error(span, reason))?;
        if matches!(
            standard.selection,
            crate::implementation::Selection::Selected(_)
        ) {
            Ok(Some((spelling, prim)))
        } else {
            Ok(None)
        }
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
    pub(super) fn native_unary_expression(
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
                if self
                    .atomic_negative_signed_literal(value, expected, expr.span)?
                    .is_some() =>
            {
                let (spelling, prim) = self
                    .atomic_negative_signed_literal(value, expected, expr.span)?
                    .expect("validated atomic negative signed integer literal");
                let value = self.integer_proxy(spelling, true, Some(&p(prim)), expr.span)?;
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
                let kind = if matches!(
                    shape,
                    Term::Primitive(P::Float | P::F16 | P::Bf16 | P::F32 | P::F64)
                ) || self.proved_float_family(&shape)
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
    pub(super) fn unary_protocol_result(
        &mut self,
        expr: &Expr<'_>,
        expected: Option<&Term>,
        operator: crate::arithmetic::Operator,
        protocol: DeclarationId,
        checked: &mut Option<n::Expr>,
    ) -> Result<Option<n::Expr>, Error> {
        let value = checked.as_ref().expect("checked protocol operand");
        let shape = if matches!(
            value.ty,
            Term::Bound { .. }
                | Term::AssociatedType { .. }
                | Term::Rigid { .. }
                | Term::NormalForm { .. }
                | Term::Hole { .. }
        ) {
            match self
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
            }
        } else {
            value.ty.clone()
        };
        let native = match operator {
            crate::arithmetic::Operator::Negate => {
                matches!(
                    shape,
                    Term::Primitive(P::Integer | P::Float | P::F16 | P::Bf16 | P::F32 | P::F64)
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

    pub(super) fn finish_logical_expression(
        &mut self,
        expr: &Expr<'_>,
        expected: Option<&Term>,
        mut left: n::Expr,
        mut right: n::Expr,
        operand_ty: Term,
    ) -> Result<n::Expr, Error> {
        let H::Binary(op, _, _) = expr.kind else {
            unreachable!("validated logical expression")
        };
        let right_ty = if is_never(&operand_ty) {
            p(P::Bool)
        } else {
            operand_ty.clone()
        };
        if !is_never(&left.ty) {
            left.ty = operand_ty.clone();
        }
        if !is_never(&right.ty) {
            right.ty = right_ty;
        }
        // Bool short-circuits; BitVec evaluates both operands. Checking a
        // child's domain does not authorize changing its execution position.
        let evaluates_right = if operand_ty == p(P::Bool) {
            match op {
                BinOp::And => super::control::boolean(&left) == Some(true),
                BinOp::Or => super::control::boolean(&left) == Some(false),
                BinOp::Implies => super::control::boolean(&left) == Some(true),
                _ => unreachable!(),
            }
        } else {
            matches!(operand_ty, Term::Primitive(P::BitVec(_)))
        };
        let ty = if is_never(&left.ty) || (evaluates_right && is_never(&right.ty)) {
            p(P::Never)
        } else {
            operand_ty
        };
        if let Some(expected) = expected {
            self.require_expression(&ty, expected, expr.span)?;
        }
        let kind = match op {
            BinOp::And => K::And(Box::new(left), Box::new(right)),
            BinOp::Or => K::Or(Box::new(left), Box::new(right)),
            BinOp::Implies => {
                let negated_left = n::Expr {
                    ty: p(P::Bool),
                    source: left.source,
                    kind: K::Not(Box::new(left)),
                };
                K::Or(Box::new(negated_left), Box::new(right))
            }
            _ => unreachable!(),
        };
        Ok(n::Expr {
            ty,
            source: source(expr.span),
            kind,
        })
    }

    pub(super) fn selected_implies_protocol(
        &self,
        subject: &Term,
        span: Span,
    ) -> Result<DeclarationId, Error> {
        use crate::ptc::{Budget, Formula, Outcome};
        let comptime = !self.runtime_value_context() && !self.in_code_body;
        if comptime && let Some(ct_proto) = self.protocols.comptime_implies {
            let scope = self.call_inference_domains();
            let frames = if scope.is_empty() {
                vec![]
            } else {
                vec![scope.iter().map(|p| p.0).collect()]
            };
            let has_assumption = crate::capabilities::capability_sources_with_layouts(
                &self.checked_traits,
                &self.capability_assumptions(),
                Budget::default(),
                &frames,
                self.implementation_index.parameter_domains(),
                Some(self.implementation_index.nominal_layouts()),
            )
            .map_err(|reason| error(span, reason))?
            .into_iter()
            .any(|capability| {
                capability.interface == ct_proto
                    && (capability.subject == *subject
                        || matches!(
                            self.context.prove(
                                Formula::equal(capability.subject, subject.clone()),
                                Budget::default(),
                            ),
                            Outcome::Entailed(_)
                        ))
            });
            let has_impl = if has_assumption {
                true
            } else if let Some(interface) = self.checked_traits.by_id(ct_proto)
                && let [rhs] = interface.type_parameters.as_slice()
            {
                let wildcard = Term::Rigid {
                    declaration: *rhs,
                    sort: Sort::Type,
                };
                !self
                    .implementation_index
                    .candidates(Some(ct_proto), &[wildcard], subject, Budget::default())
                    .map_err(|reason| {
                        error(span, format!("Implies candidate query failed: {reason:?}"))
                    })?
                    .declarations()
                    .is_empty()
            } else {
                false
            };
            if has_impl || self.protocols.implies.is_none() {
                return Ok(ct_proto);
            }
        }
        self.protocols.implies.ok_or_else(|| {
            error(
                span,
                "`==>` on non-bool operands requires a bound Implies protocol",
            )
        })
    }

    pub(super) fn selected_comparison_protocol(
        &self,
        ordering: bool,
        subject: &Term,
        span: Span,
    ) -> Result<DeclarationId, Error> {
        use crate::ptc::{Budget, Formula, Outcome};
        let comptime = !self.runtime_value_context() && !self.in_code_body;
        let (ct_opt, rt_opt, missing_msg) = if ordering {
            (
                self.protocols.comptime_ordering,
                self.protocols.ordering,
                "ordering protocol for this type is not implemented without an Ord binding",
            )
        } else {
            (
                self.protocols.comptime_equality,
                self.protocols.equality,
                "equality protocol for this type is not implemented without an Eq binding",
            )
        };
        if comptime && let Some(ct_proto) = ct_opt {
            let scope = self.call_inference_domains();
            let frames = if scope.is_empty() {
                vec![]
            } else {
                vec![scope.iter().map(|p| p.0).collect()]
            };
            let has_assumption = crate::capabilities::capability_sources_with_layouts(
                &self.checked_traits,
                &self.capability_assumptions(),
                Budget::default(),
                &frames,
                self.implementation_index.parameter_domains(),
                Some(self.implementation_index.nominal_layouts()),
            )
            .map_err(|reason| error(span, reason))?
            .into_iter()
            .any(|capability| {
                capability.interface == ct_proto
                    && (capability.subject == *subject
                        || matches!(
                            self.context.prove(
                                Formula::equal(capability.subject, subject.clone()),
                                Budget::default(),
                            ),
                            Outcome::Entailed(_)
                        ))
            });
            let has_impl = if has_assumption {
                true
            } else if let Some(interface) = self.checked_traits.by_id(ct_proto) {
                let wildcards = interface
                    .type_parameters
                    .iter()
                    .map(|declaration| Term::Rigid {
                        declaration: *declaration,
                        sort: Sort::Type,
                    })
                    .collect::<Vec<_>>();
                !self
                    .implementation_index
                    .candidates(Some(ct_proto), &wildcards, subject, Budget::default())
                    .map_err(|reason| {
                        error(
                            span,
                            format!("comparison candidate query failed: {reason:?}"),
                        )
                    })?
                    .declarations()
                    .is_empty()
            } else {
                false
            };
            if has_impl || rt_opt.is_none() {
                return Ok(ct_proto);
            }
        }
        rt_opt.ok_or_else(|| error(span, missing_msg))
    }

    pub(super) fn borrow_operand_for_pointer_arg(
        &mut self,
        value: n::Expr,
        statements: &mut Vec<n::Expr>,
        span: Span,
    ) -> Result<n::Expr, Error> {
        if is_never(&value.ty) {
            return Ok(value);
        }
        if let K::ReadPointer(pointer) = value.kind {
            return Ok(*pointer);
        }
        if is_local_place_expr(&value) {
            return Ok(n::Expr {
                source: source(span),
                ty: Term::Pointer(Box::new(value.ty.clone())),
                kind: K::AddressOf(Box::new(value)),
            });
        }
        let ty = value.ty.clone();
        let val_source = value.source;
        let binding = self.spread_binding(span)?;
        statements.push(n::Expr {
            ty: p(P::Void),
            source: val_source,
            kind: K::Let {
                binding,
                value: Box::new(value),
            },
        });
        Ok(n::Expr {
            source: source(span),
            ty: Term::Pointer(Box::new(ty.clone())),
            kind: K::AddressOf(Box::new(n::Expr {
                source: val_source,
                ty,
                kind: K::Local(binding),
            })),
        })
    }

    pub(super) fn finish_comparison_protocol_call(
        &mut self,
        site: &Expr<'_>,
        protocol: DeclarationId,
        method: &str,
        left: n::Expr,
        right: n::Expr,
        expected: Option<&Term>,
    ) -> Result<n::Expr, Error> {
        let param_count = self
            .checked_traits
            .by_id(protocol)
            .map_or(0, |t| t.type_parameters.len());
        let interface_arguments = if param_count == 1 {
            let rhs_ty = if is_never(&right.ty) {
                self.protocol_argument_context(&left.ty, protocol, site.span)?
                    .unwrap_or_else(|| left.ty.clone())
            } else {
                right.ty.clone()
            };
            vec![rhs_ty]
        } else {
            vec![]
        };
        let callee = self
            .selected_method(
                left.ty.clone(),
                Some(n::InterfaceInstance {
                    declaration: protocol,
                    arguments: interface_arguments,
                }),
                method,
                site.span,
            )?
            .ok_or_else(|| {
                error(
                    site.span,
                    format!("comparison protocol `{protocol}` has no selected `{method}` method"),
                )
            })?;
        let expects_pointers = matches!(
            callee.ty.callable_signature(),
            Term::Function { parameters, .. }
                if parameters.len() == 2
                    && matches!(parameters[0].ty, Term::Pointer(_))
                    && matches!(parameters[1].ty, Term::Pointer(_))
        );
        let result = if expects_pointers {
            let mut statements = Vec::new();
            let left_arg = self.borrow_operand_for_pointer_arg(left, &mut statements, site.span)?;
            let right_arg =
                self.borrow_operand_for_pointer_arg(right, &mut statements, site.span)?;
            let call = self.finish_call(
                site,
                callee,
                &[],
                vec![left_arg, right_arg],
                Some(&p(P::Bool)),
                None,
            )?;
            if statements.is_empty() {
                call
            } else {
                n::Expr {
                    ty: call.ty.clone(),
                    source: source(site.span),
                    kind: K::Block {
                        statements,
                        result: Some(Box::new(call)),
                    },
                }
            }
        } else {
            self.finish_call(
                site,
                callee,
                &[],
                vec![left, right],
                Some(&p(P::Bool)),
                None,
            )?
        };
        if let Some(expected) = expected {
            self.require_expression(&result.ty, expected, site.span)?;
        }
        Ok(result)
    }

    #[inline(never)]
    pub(super) fn finish_comparison_expression(
        &mut self,
        expr: &Expr<'_>,
        expected: Option<&Term>,
        left: n::Expr,
        right: n::Expr,
    ) -> Result<n::Expr, Error> {
        let (ty, kind) = match &expr.kind {
            H::Binary(op @ (BinOp::Eq | BinOp::Ne), _, _) => {
                let (a, b) = (left, right);
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
                    let protocol = self.selected_comparison_protocol(false, &a.ty, expr.span)?;
                    let method = if *op == BinOp::Eq { "eq" } else { "ne" };
                    return self
                        .finish_comparison_protocol_call(expr, protocol, method, a, b, expected);
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

            H::Binary(op @ (BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge), _, _) => {
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
                    if !matches!(shape, Term::Primitive(primitive) if matches!(primitive, P::Char | P::Float | P::F16 | P::Bf16 | P::F32 | P::F64 | P::Usize | P::Isize | P::Never) || self.integer_layout(primitive).is_some())
                        && !self.proved_ordered_scalar_family(&shape)
                    {
                        let protocol =
                            self.selected_comparison_protocol(true, &left.ty, expr.span)?;
                        let method = match op {
                            BinOp::Lt => "lt",
                            BinOp::Le => "le",
                            BinOp::Gt => "gt",
                            _ => "ge",
                        };
                        return self.finish_comparison_protocol_call(
                            expr, protocol, method, left, right, expected,
                        );
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
            .by_id(protocol)
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
            let binders = implementation
                .parameter_binders()
                .map_err(|e| error(span, e.to_string()))?;
            let signature = Term::Function {
                parameters: vec![Parameter {
                    sort: Sort::Runtime,
                    ty: implementation.subject.clone(),
                }],
                result: Box::new(rhs.clone()),
            }
            .abstract_parameters(&binders)
            .map_err(|e| error(span, e.to_string()))?;
            let scheme = crate::calls::Scheme {
                parameter_sorts: binders.iter().map(|p| p.sort).collect(),
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
