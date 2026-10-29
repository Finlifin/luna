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
            H::Index(value, index) => {
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
                    let protocol = self.protocols.index.ok_or_else(|| error(expr.span,
                        "indexing requires a bound Index protocol, BitVec or comptime List<Type>"))?;
                    let argument = match self.argument_attempt(index, None, None)? {
                        Some(argument) => argument,
                        None => {
                            let context =
                                self.protocol_argument_context(&value.ty, protocol, expr.span)?;
                            self.expr(index, context.as_ref())?
                        }
                    };
                    return self.finish_binary_protocol_call(
                        expr, protocol, "index", value, argument, expected,
                    );
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
            H::Cast(value, target) => {
                let target = self.type_expr(target)?;
                let value = self.expr(value, None)?;
                self.ensure_nominal_type(&target, expr.span)?;
                self.ensure_nominal_type(&value.ty, expr.span)?;
                let conversion = nir::newtypes::conversion_from(
                    self.newtypes.values(),
                    &value.ty,
                    &target,
                    &self.call_input_frames(),
                    &mut 100_000,
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
                self.old_scope.as_mut().unwrap().active = previous;
                return result;
            }
            H::TakeTupleEnumerate(input) => {
                self.tuple_enumeration_input(input)?;
                return Err(error(
                    expr.span,
                    "tuple_enumerate structural iteration is not yet implemented; runtime elements cannot be evaluated at comptime",
                ));
            }
            H::TakeIntrinsic(_) => {
                return Err(error(
                    expr.span,
                    "intrinsic view requires a direct positional call; target input protocols for other call forms are not implemented",
                ));
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
                    Term::Nominal { declaration, .. } => self
                        .enums
                        .get(&declaration)
                        .and_then(|definition| definition.encoding.as_ref())
                        .map(|encoding| encoding.width())
                        .ok_or_else(|| {
                            error(
                                expr.span,
                                "enum bit view requires a checked fixed-width encoding",
                            )
                        })?,
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
