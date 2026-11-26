//! Indexing, conversion and value views.
use super::*;

impl Checker<'_, '_> {
    #[inline(never)]
    pub(super) fn views_value_expression(
        &mut self,
        expr: &Expr<'_>,
        expected: Option<&Term>,
        _partial: Option<&crate::calls::CallableContext>,
    ) -> Result<n::Expr, Error> {
        let (ty, kind) = match &expr.kind {
            H::Index(value, args) => {
                let [hir::common::Arg::Positional(index)] = args else {
                    if matches!(value.kind, H::TakeBitVec(_)) {
                        return Err(error(
                            expr.span,
                            "multi-argument indexing is not supported on BitVec views",
                        ));
                    }
                    let (temp_stmt, receiver_ptr) =
                        if let Some(ptr) = self.index_base_place_pointer(value, false)? {
                            (None, ptr)
                        } else {
                            let base_val = self.expr(value, None)?;
                            if is_never(&base_val.ty) {
                                if let Some(expected) = expected {
                                    self.require_expression(&base_val.ty, expected, expr.span)?;
                                }
                                return Ok(base_val);
                            }
                            self.spill_receiver_for_address(base_val, value.span)?
                        };
                    let result =
                        self.space_apply_with_args(expr, receiver_ptr, args, None, expected)?;
                    return Ok(self.wrap_temp_stmt(temp_stmt, result));
                };
                if !matches!(value.kind, H::TakeBitVec(_))
                    && (self.runtime_value_context() || self.in_code_body)
                    && let Some(place_ptr) = self.index_base_place_pointer(value, false)?
                {
                    let Term::Pointer(inner) = &place_ptr.ty else {
                        unreachable!()
                    };
                    let shape = self.structural_type(inner, expr.span)?;
                    if self.list_element_type(&shape).is_some() {
                        let value = n::Expr {
                            source: source(value.span),
                            ty: (**inner).clone(),
                            kind: K::ReadPointer(Box::new(place_ptr)),
                        };
                        let element = self.list_element_type(&value.ty).cloned().unwrap();
                        let index = self.expr(index, Some(&p(P::Integer)))?;
                        let ty = if index.ty == p(P::Never) {
                            p(P::Never)
                        } else {
                            element
                        };
                        let result = n::Expr {
                            source: source(expr.span),
                            ty,
                            kind: K::TypeListIndex(Box::new(value), Box::new(index)),
                        };
                        if let Some(expected) = expected {
                            self.require_expression(&result.ty, expected, expr.span)?;
                        }
                        return Ok(result);
                    }
                    if let Term::Primitive(P::BitVec(width)) = shape {
                        let value = n::Expr {
                            source: source(value.span),
                            ty: p(P::BitVec(width)),
                            kind: K::ReadPointer(Box::new(place_ptr)),
                        };
                        if let H::Range {
                            start: Some(start),
                            end: Some(end),
                            inclusive: false,
                        } = &index.kind
                        {
                            return self.bit_window(expr, value, start, end, expected);
                        }
                        if matches!(index.kind, H::Range { .. }) {
                            return Err(error(
                                index.span,
                                "BitVec slicing currently requires an explicit half-open range",
                            ));
                        }
                        let (index_expr, static_index) = self.bit_read_index(index)?;
                        if let Some(bit_index) = static_index
                            && bit_index >= width as usize
                        {
                            return Err(error(expr.span, "BitVec index is out of bounds"));
                        }
                        let kind = K::BitIndex(Box::new(value), Box::new(index_expr));
                        let ty = p(P::BitVec(1));
                        if let Some(expected) = expected {
                            self.require_expression(&ty, expected, expr.span)?;
                        }
                        return Ok(n::Expr {
                            source: source(expr.span),
                            ty,
                            kind,
                        });
                    }
                    let arg_ctx = if let Some(index_proto) = self.protocols.index {
                        self.adapt_receiver_index_context(&place_ptr.ty, index_proto, expr.span)?
                    } else {
                        None
                    };
                    let argument = match self.argument_attempt(index, None, None)? {
                        Some(argument) => argument,
                        None => self.expr(index, arg_ctx.as_ref())?,
                    };
                    return match self.resolve_index_call_on_receiver(
                        expr, place_ptr, argument, false, expected,
                    )? {
                        crate::program::places::IndexResolution::Selected(call) => Ok(call),
                        crate::program::places::IndexResolution::NoCandidate {
                            receiver_ptr,
                            arg_expr,
                        } => {
                            if self.protocols.space_apply.is_some() {
                                self.space_apply_with_args(
                                    expr,
                                    receiver_ptr,
                                    args,
                                    Some(arg_expr),
                                    expected,
                                )
                            } else {
                                let Term::Pointer(inner) = &receiver_ptr.ty else {
                                    unreachable!()
                                };
                                Err(error(
                                    expr.span,
                                    format!(
                                        "no `Index` or `SpaceApply` implementation applies for `{inner}`"
                                    ),
                                ))
                            }
                        }
                    };
                }
                let value = self.expr(value, None)?;
                if let Some(element) = self.list_element_type(&value.ty).cloned() {
                    let index = self.expr(index, Some(&p(P::Integer)))?;
                    let ty = if index.ty == p(P::Never) {
                        p(P::Never)
                    } else {
                        element
                    };
                    let result = n::Expr {
                        source: source(expr.span),
                        ty,
                        kind: K::TypeListIndex(Box::new(value), Box::new(index)),
                    };
                    if let Some(expected) = expected {
                        self.require_expression(&result.ty, expected, expr.span)?;
                    }
                    return Ok(result);
                }
                let Term::Primitive(P::BitVec(width)) =
                    self.structural_type(&value.ty, expr.span)?
                else {
                    if is_never(&value.ty) {
                        if let Some(expected) = expected {
                            self.require_expression(&value.ty, expected, expr.span)?;
                        }
                        return Ok(value);
                    }
                    if !self.runtime_value_context() && !self.in_code_body {
                        let protocol = self.selected_index_protocol(&value.ty, expr.span)?;
                        if Some(protocol) == self.protocols.comptime_index {
                            let argument = match self.argument_attempt(index, None, None)? {
                                Some(argument) => argument,
                                None => {
                                    let context = self.protocol_argument_context(
                                        &value.ty, protocol, expr.span,
                                    )?;
                                    self.expr(index, context.as_ref())?
                                }
                            };
                            return self.finish_binary_protocol_call(
                                expr, protocol, "index", value, argument, expected,
                            );
                        }
                    }
                    let (temp_stmt, receiver_ptr) =
                        self.spill_receiver_for_address(value, expr.span)?;
                    let arg_ctx = if let Some(index_proto) = self.protocols.index {
                        self.adapt_receiver_index_context(&receiver_ptr.ty, index_proto, expr.span)?
                    } else {
                        None
                    };
                    let argument = match self.argument_attempt(index, None, None)? {
                        Some(argument) => argument,
                        None => self.expr(index, arg_ctx.as_ref())?,
                    };
                    let result = match self.resolve_index_call_on_receiver(
                        expr,
                        receiver_ptr,
                        argument,
                        false,
                        expected,
                    )? {
                        crate::program::places::IndexResolution::Selected(call) => call,
                        crate::program::places::IndexResolution::NoCandidate {
                            receiver_ptr,
                            arg_expr,
                        } => {
                            if self.protocols.space_apply.is_some() {
                                self.space_apply_with_args(
                                    expr,
                                    receiver_ptr,
                                    args,
                                    Some(arg_expr),
                                    expected,
                                )?
                            } else {
                                let Term::Pointer(inner) = &receiver_ptr.ty else {
                                    unreachable!()
                                };
                                return Err(error(
                                    expr.span,
                                    format!(
                                        "no `Index` or `SpaceApply` implementation applies for `{inner}`"
                                    ),
                                ));
                            }
                        }
                    };
                    return Ok(self.wrap_temp_stmt(temp_stmt, result));
                };
                let (start, end) = match &index.kind {
                    H::Range {
                        start: Some(start),
                        end: Some(end),
                        inclusive: false,
                    } => {
                        return self.bit_window(expr, value, start, end, expected);
                    }
                    H::Range { .. } => {
                        return Err(error(
                            index.span,
                            "BitVec slicing currently requires an explicit half-open range",
                        ));
                    }
                    _ => {
                        let (checked, position) = self.bit_read_index(index)?;
                        let Some(index) = position else {
                            let ty = if checked.ty == p(P::Never) {
                                p(P::Never)
                            } else {
                                p(P::BitVec(1))
                            };
                            if let Some(expected) = expected {
                                self.require_expression(&ty, expected, expr.span)?;
                            }
                            return Ok(n::Expr {
                                source: source(expr.span),
                                ty,
                                kind: K::BitIndex(Box::new(value), Box::new(checked)),
                            });
                        };
                        (
                            index,
                            index
                                .checked_add(1)
                                .ok_or_else(|| error(expr.span, "BitVec index overflow"))?,
                        )
                    }
                };
                if start > end || end > width {
                    return Err(error(expr.span, "BitVec index or slice is out of bounds"));
                }
                (
                    p(P::BitVec(end - start)),
                    K::BitSlice {
                        value: Box::new(value),
                        start,
                        end,
                    },
                )
            }
            H::DynCast(value, trait_expr) => {
                return self.dyn_cast_expression(expr, value, trait_expr, expected);
            }
            H::Cast(value, target) => {
                let target = self.type_expr(target)?;
                let value = self.expr(value, None)?;
                self.ensure_nominal_type(&target, expr.span)?;
                self.ensure_nominal_type(&value.ty, expr.span)?;
                let conversion = nir::newtypes::conversion_with(
                    &value.ty,
                    &target,
                    &self.call_input_frames(),
                    &mut 100_000,
                    |id, _| Ok(self.newtypes.get(&id)),
                )
                .map_err(|e| error(expr.span, e.to_string()))?;
                if conversion.is_some() {
                    if let Some(expected) = expected {
                        self.require_expression(&target, expected, expr.span)?;
                    }
                    return Ok(n::Expr {
                        ty: target,
                        source: source(expr.span),
                        kind: K::NewtypeConvert(Box::new(value)),
                    });
                }
                if let K::TypeValue(base_ty) = value.kind {
                    self.validate_type_view(&base_ty, &target, expr.span)?;
                    if let Some(expected) = expected {
                        self.require_expression(&p(P::Type), expected, expr.span)?;
                    }
                    return Ok(n::Expr {
                        ty: p(P::Type),
                        source: source(expr.span),
                        kind: K::TypeValue(base_ty),
                    });
                }
                let Term::Primitive(P::BitVec(width)) =
                    self.structural_type(&value.ty, expr.span)?
                else {
                    return Err(error(
                        expr.span,
                        "this conversion requires a protocol not yet implemented",
                    ));
                };
                let shape = self.structural_type(&target, expr.span)?;
                let (target_width, widening) = match shape {
                    Term::Primitive(P::BitVec(width)) => (width, true),
                    Term::Primitive(primitive) => (
                        primitive.unrestricted_scalar_width_in(self.target).ok_or_else(|| error(expr.span,
                            "BitVec conversion requires an unrestricted scalar or BitVec target; other representations need unsafe construction support"))? as usize,
                        self.integer_layout(primitive).is_some_and(|(signed, _)| !signed),
                    ),
                    Term::Nominal { declaration, .. }
                        if self
                            .structs
                            .get(&declaration)
                            .is_some_and(|def| def.packed == n::PackedMode::Bit) =>
                    {
                        let (target_width, restricted) = self.bit_packed_representation_width(
                            &shape,
                            "BitVec conversion",
                            expr.span,
                        )?;
                        if restricted && !self.in_unsafe_context() {
                            return Err(error(
                                expr.span,
                                "BitVec conversion to attrs.packed.bit struct with restricted field representations requires an unsafe block",
                            ));
                        }
                        (target_width, false)
                    }
                    _ => return Err(error(expr.span, "BitVec conversion target is not implemented")),
                };
                if !widening && target_width != width {
                    return Err(error(
                        expr.span,
                        "BitVec representation conversion requires exactly the target width",
                    ));
                }
                if target_width < width {
                    return Err(error(
                        expr.span,
                        "BitVec narrowing requires an explicit slice",
                    ));
                }
                (target, K::BitConvert(Box::new(value)))
            }
            H::MultiDeref(inner) => {
                let prev_allow = std::mem::replace(&mut self.allow_packed_pointer_projection, true);
                let current = self.expr(inner, None);
                self.allow_packed_pointer_projection = prev_allow;
                let mut current = current?;
                let mut depth = 0usize;
                loop {
                    let shape = self.structural_type(&current.ty, expr.span)?;
                    let Term::Pointer(pointee) = shape else {
                        if depth == 0 {
                            return Err(error(expr.span, "multi_deref requires a pointer"));
                        }
                        break;
                    };
                    depth += 1;
                    if depth > 128 {
                        return Err(error(expr.span, "multi_deref depth exhausted"));
                    }
                    if !self.intrinsic_pointer_input(&pointee)
                        && !self.copy_read_input(&pointee, expr.span)
                    {
                        return Err(error(
                            expr.span,
                            "pointer reads of non-intrinsic values require Copy evidence or a memory transfer contract; this read is not yet supported",
                        ));
                    }
                    current = n::Expr {
                        source: source(expr.span),
                        ty: (*pointee).clone(),
                        kind: K::ReadPointer(Box::new(current)),
                    };
                }
                if let Some(expected) = expected {
                    self.require_expression(&current.ty, expected, expr.span)?;
                }
                return Ok(current);
            }
            H::TakeOld(value) => {
                let Some(scope) = self
                    .old_scope
                    .as_mut()
                    .filter(|scope| Some(scope.owner) == self.signature_owner)
                else {
                    return Err(error(
                        expr.span,
                        "old is only available in program postconditions",
                    ));
                };
                let previous = std::mem::replace(&mut scope.active, true);
                let result = self.expr(value, expected);
                if let Some(scope) = self.old_scope.as_mut() {
                    scope.active = previous;
                }
                return result;
            }
            H::TakeTupleEnumerate(input) => {
                self.tuple_enumeration_input(input)?;
                return Err(error(
                    expr.span,
                    "tuple_enumerate structural iteration is not yet implemented; runtime elements cannot be evaluated at comptime",
                ));
            }
            H::TakeTupleIndexed(input) => {
                self.tuple_enumeration_input(input)?;
                return Err(error(
                    expr.span,
                    "tuple_indexed structural iteration requires inline for",
                ));
            }
            H::TakeIntrinsic(_) => {
                return Err(error(
                    expr.span,
                    "intrinsic view requires a direct positional call; target input protocols for other call forms are not implemented",
                ));
            }
            H::TakeAttribute(value) => {
                if self.runtime_value_context() || self.in_code_body {
                    return Err(error(
                        expr.span,
                        "attribute view requires comptime invocation",
                    ));
                }
                let target = self.expr(value, None)?;
                let ty = if is_never(&target.ty) {
                    p(P::Never)
                } else {
                    p(P::Object)
                };
                (ty, K::DefinitionAttributes(Box::new(target)))
            }
            H::TakeTagName(value) => {
                let value = self.expr(value, None)?;
                let source_ty = self.structural_type(&value.ty, expr.span)?;
                self.ensure_nominal_type(&source_ty, expr.span)?;
                if let Term::Nominal { declaration, .. } = source_ty {
                    if !self.enums.contains_key(&declaration) {
                        return Err(error(expr.span, "tag_name requires a Symbol or enum value"));
                    }
                    (p(P::Str), K::EnumName(Box::new(value)))
                } else {
                    if !is_never(&source_ty) && source_ty != p(P::Symbol) {
                        return Err(error(expr.span, "tag_name requires a Symbol or enum value"));
                    }
                    let ty = if is_never(&value.ty) {
                        p(P::Never)
                    } else {
                        p(P::Str)
                    };
                    (ty, K::SymbolName(Box::new(value)))
                }
            }
            H::TakeBitVec(value) => {
                let value = self.expr(value, None)?;
                let source_ty = self.structural_type(&value.ty, expr.span)?;
                self.ensure_nominal_type(&source_ty, expr.span)?;
                let width = match source_ty {
                    Term::Primitive(P::BitVec(width)) => width,
                    Term::Nominal { declaration, .. } => {
                        if let Some(definition) = self.enums.get(&declaration) {
                            definition
                                .encoding
                                .as_ref()
                                .map(|encoding| encoding.width())
                                .ok_or_else(|| {
                                    error(
                                        expr.span,
                                        "enum bit view requires a checked fixed-width encoding",
                                    )
                                })?
                        } else if self
                            .structs
                            .get(&declaration)
                            .is_some_and(|def| def.packed == n::PackedMode::Bit)
                        {
                            self.bit_packed_representation_width(&source_ty, "bit view", expr.span)?
                                .0
                        } else {
                            return Err(error(
                                expr.span,
                                "bit view requires a supported fixed-width representation",
                            ));
                        }
                    }
                    Term::Primitive(primitive) => primitive
                        .readable_scalar_width_in(self.target)
                        .map(|width| width as usize)
                        .ok_or_else(|| {
                            error(
                                expr.span,
                                "bit view requires a supported fixed-width representation",
                            )
                        })?,
                    _ => {
                        return Err(error(
                            expr.span,
                            "bit view requires a supported fixed-width representation",
                        ));
                    }
                };
                (p(P::BitVec(width)), K::TakeBits(Box::new(value)))
            }
            _ => unreachable!("value expression category was checked by the dispatcher"),
        };
        self.finish_value_expression(expr, expected, ty, kind)
    }
}
