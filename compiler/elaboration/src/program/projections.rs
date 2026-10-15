//! Field and associated-member checking lives outside the recursive expression
//! dispatcher so its temporaries do not inflate every signature dependency frame.
use super::*;

impl Checker<'_, '_> {
    #[inline(never)]
    pub(super) fn projection_expression(
        &mut self,
        expr: &Expr<'_>,
        value: &Expr<'_>,
        field: &hir::common::Ident,
        expected: Option<&Term>,
        partial: Option<&crate::calls::CallableContext>,
    ) -> Result<n::Expr, Error> {
        if let Some(value) = self.associated_projection(value, field.name.as_str(), expr.span)? {
            if let Some(expected) = expected {
                self.require(&value.ty, expected, expr.span)?;
            }
            return Ok(value);
        }
        let (ty, kind) = {
            if let Some(pointer) = self.dereferenced_place(expr)? {
                let Term::Pointer(ty) = &pointer.ty else {
                    unreachable!()
                };
                if !self.intrinsic_pointer_input(ty) {
                    return Err(error(
                        expr.span,
                        "non-Copy pointer reads require memory transfer contracts",
                    ));
                }
                if let Some(expected) = expected {
                    self.require(ty, expected, expr.span)?;
                }
                return Ok(n::Expr {
                    ty: (**ty).clone(),
                    source: source(expr.span),
                    kind: K::ReadPointer(Box::new(pointer)),
                });
            }

            if matches!(field.name.as_str(), "NaN" | "Infinity") && self.is_type_head(value) {
                let ty = self.type_expr(value)?;
                if let Some(constant) =
                    self.float_associated_constant(&ty, field.name.as_str(), expr.span)?
                {
                    if let Some(expected) = expected {
                        self.require(&constant.ty, expected, expr.span)?;
                    }
                    return Ok(constant);
                }
            }
            if let Some(method) = self.static_method(value, field.name.as_str(), expr.span)? {
                if self.incomplete_method_instance(&method) {
                    return self.contextual_method_instance(expr, method, expected, partial);
                }
                return self.callable_value_view(method, expected, expr.span);
            }
            if let Some((ty, index)) = self.variant_target(expr, expected)? {
                let Term::Nominal { declaration, .. } = &ty else {
                    unreachable!()
                };
                let name = self.enums[declaration].variants[index].name.clone();
                let variant = self.enum_variant(&ty, &name, expr.span)?;
                if let Some(expected) = expected {
                    self.require(&variant.ty, expected, expr.span)?;
                }
                return Ok(variant);
            }
            if self.is_type_head(value) {
                let ty = self.type_expr(value)?;
                if let Term::Nominal { declaration, .. } = &ty {
                    if self.enums.contains_key(declaration) {
                        let variant = self.enum_variant(&ty, field.name.as_str(), expr.span)?;
                        if let Some(expected) = expected {
                            self.require(&variant.ty, expected, expr.span)?;
                        }
                        return Ok(variant);
                    }
                }
            }
            let value = self.expr(value, None)?;
            // A resolved builtin view or Type constant is already a pure
            // TypeValue. Do not erase a dynamic Type-valued computation.
            if let K::TypeValue(ty) = &value.kind {
                if let Some(constant) =
                    self.float_associated_constant(ty, field.name.as_str(), expr.span)?
                {
                    if let Some(expected) = expected {
                        self.require(&constant.ty, expected, expr.span)?;
                    }
                    return Ok(constant);
                }
            }

            let pointer_projection = matches!(value.ty, Term::Pointer(_));
            let subject = match &value.ty {
                Term::Pointer(inner) => inner.as_ref(),
                ty => ty,
            };
            let shape = self.structural_type(subject, field.span)?;
            self.ensure_nominal_type(&shape, field.span)?;
            if let Term::Record(fields) = &shape {
                let index = fields
                    .iter()
                    .position(|(name, _)| name == field.name.as_str())
                    .ok_or_else(|| {
                        error(field.span, format!("unknown record field {}", field.name))
                    })?;
                let ty = fields[index].1.clone();
                let mut projected = if pointer_projection {
                    n::Expr {
                        ty: Term::Pointer(Box::new(ty.clone())),
                        source: source(expr.span),
                        kind: K::PointerAnonymousField {
                            pointer: Box::new(value),
                            name: field.name.to_string(),
                            index,
                        },
                    }
                } else {
                    n::Expr {
                        ty: ty.clone(),
                        source: source(expr.span),
                        kind: K::AnonymousField {
                            value: Box::new(value),
                            name: field.name.to_string(),
                            index,
                        },
                    }
                };
                if pointer_projection {
                    projected = self.contextual_field_read(projected, expected, expr.span)?;
                }
                if let Some(expected) = expected {
                    self.require(&projected.ty, expected, expr.span)?;
                }
                return Ok(projected);
            }
            let Term::Nominal {
                declaration,
                arguments,
            } = &shape
            else {
                return Err(error(
                    expr.span,
                    "field projection requires a supported struct",
                ));
            };
            let def = self
                .structs
                .get(declaration)
                .ok_or_else(|| error(expr.span, "missing struct descriptor"))?;
            let index = def
                .fields
                .iter()
                .position(|(name, _)| name == field.name.as_str())
                .ok_or_else(|| error(field.span, format!("unknown struct field {}", field.name)))?;
            let ty = self.instantiate_input_types(
                &def.fields[index].1,
                &def.type_parameters,
                arguments,
                expr.span,
            )?;
            let ty = self.callable_input_view(&ty, expr.span)?;
            let declaration = *declaration;
            if pointer_projection {
                let projection = K::PointerField {
                    pointer: Box::new(value),
                    declaration,
                    index,
                };
                let pointer = n::Expr {
                    source: source(expr.span),
                    ty: Term::Pointer(Box::new(ty)),
                    kind: projection,
                };
                let projected = self.contextual_field_read(pointer, expected, expr.span)?;
                (projected.ty, projected.kind)
            } else {
                (
                    ty,
                    K::Field {
                        value: Box::new(value),
                        declaration,
                        index,
                    },
                )
            }
        };
        if let Some(expected) = expected {
            self.require(&ty, expected, expr.span)?;
        }
        Ok(n::Expr {
            ty,
            source: source(expr.span),
            kind,
        })
    }

    /// A field pointer keeps its identity without a target. With a concrete
    /// target, only built-in Copy reads may remove pointer layers.
    fn contextual_field_read(
        &self,
        mut value: n::Expr,
        expected: Option<&Term>,
        span: rustc_span::Span,
    ) -> Result<n::Expr, Error> {
        let Some(expected) = expected else {
            return Ok(value);
        };
        for _ in 0..128 {
            if self.require(&value.ty, expected, span).is_ok() {
                return Ok(value);
            }
            let Term::Pointer(pointee) = &value.ty else {
                break;
            };
            if !self.intrinsic_pointer_input(pointee) {
                break;
            }
            value = n::Expr {
                source: value.source,
                ty: (**pointee).clone(),
                kind: K::ReadPointer(Box::new(value)),
            };
        }
        self.require(&value.ty, expected, span)?;
        Ok(value)
    }
}
