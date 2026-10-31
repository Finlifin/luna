//! Value references, dereferencing and assignment.
use super::*;

impl Checker<'_, '_> {
    #[inline(never)]
    pub(super) fn places_value_expression(
        &mut self,
        expr: &Expr<'_>,
        expected: Option<&Term>,
        _partial: Option<&crate::calls::CallableContext>,
    ) -> Result<n::Expr, Error> {
        let (ty, kind) = match &expr.kind {
            H::Ref(inner) => {
                if self.checking_code_generator {
                    self.forget_generator_schema_evidence(expr.span)?;
                }
                if let Some(pointer) = self.dereferenced_place(inner)? {
                    if let Some(expected) = expected {
                        self.require(&pointer.ty, expected, expr.span)?;
                    }
                    return Ok(pointer);
                }
                let place = self.expr(inner, None)?;
                let result = n::Expr {
                    ty: Term::Pointer(Box::new(place.ty.clone())),
                    source: source(expr.span),
                    kind: K::AddressOf(Box::new(place)),
                };
                if let Some(message) = n::pointer_operation_error_in(&result, self.target) {
                    return Err(error(expr.span, message));
                }
                if let Some(expected) = expected {
                    self.require(&result.ty, expected, expr.span)?;
                }
                return Ok(result);
            }
            H::Deref(inner) => {
                let pointer = self.expr(inner, None)?;
                let Term::Pointer(ty) = &pointer.ty else {
                    return Err(error(expr.span, "dereference requires a pointer"));
                };
                if !self.intrinsic_pointer_input(ty) && !self.copy_read_input(ty) {
                    return Err(error(
                        expr.span,
                        "pointer reads of non-intrinsic values require Copy evidence or a memory transfer contract; this read is not yet supported",
                    ));
                }
                ((**ty).clone(), K::ReadPointer(Box::new(pointer)))
            }
            H::AssignOp(op, target, value) => {
                return self.compound_assignment(expr, op, target, value, expected);
            }
            H::Assign(target, value) => {
                if let Some(mut pointer) = self.dereferenced_place(target)? {
                    let Term::Pointer(ty) = &pointer.ty else {
                        return Err(error(target.span, "pointer assignment requires a pointer"));
                    };
                    if !self.intrinsic_pointer_input(ty) && !self.copy_read_input(ty) {
                        return Err(error(
                            target.span,
                            "non-Copy pointer writes require memory transfer contracts",
                        ));
                    }
                    let value = self.expr(value, Some(ty))?;
                    // Expected-type checking has established equality with the
                    // stored type. Express that identity view on the pointer so
                    // the operation keeps matching operand annotations.
                    let ty = if is_never(&value.ty) {
                        p(P::Never)
                    } else {
                        pointer.ty = Term::Pointer(Box::new(value.ty.clone()));
                        p(P::Void)
                    };
                    if let Some(expected) = expected {
                        self.require_expression(&ty, expected, expr.span)?;
                    }
                    return Ok(n::Expr {
                        ty,
                        source: source(expr.span),
                        kind: K::WritePointer {
                            pointer: Box::new(pointer),
                            value: Box::new(value),
                        },
                    });
                }

                let mut storage = *target;
                let mut selections = Vec::new();
                while let H::Index(base, index) = &storage.kind {
                    selections.push(*index);
                    storage = base;
                }
                selections.reverse();
                let take_bits = if let H::TakeBitVec(base) = &storage.kind {
                    storage = base;
                    true
                } else {
                    false
                };
                let bit_assignment = take_bits || !selections.is_empty();
                if bit_assignment {
                    if let Some(mut pointer) = self.dereferenced_place(storage)? {
                        let Term::Pointer(ty) = &pointer.ty else {
                            unreachable!()
                        };
                        let storage_type = self.structural_type(ty, target.span)?;
                        let width = match &storage_type {
                            Term::Primitive(P::BitVec(width))=>*width,
                            Term::Primitive(primitive) if take_bits => primitive.unrestricted_scalar_width_in(self.target).map(|width|width as usize)
                                .ok_or_else(||error(target.span,"bit writeback requires an unrestricted scalar representation"))?,
                            _=>return Err(error(target.span,"bit writeback requires a direct bit view or a BitVec place")),
                        };
                        pointer.ty = Term::Pointer(Box::new(storage_type));
                        return self.bit_pointer_writeback(
                            expr,
                            pointer,
                            width,
                            &selections,
                            value,
                            expected,
                        );
                    }
                }
                let mut base = storage;
                let mut fields = Vec::new();
                while let H::Projection(inner, name) = &base.kind {
                    fields.push(*name);
                    base = inner;
                }
                fields.reverse();
                let binding = match base.kind {
                    H::ResolvedLocal(_, id) | H::ResolvedFnParam(_, id) => local(id),
                    H::SelfValue => {
                        let Some(ItemKind::Fn(sig, _) | ItemKind::FnDeclaration(sig)) =
                            self.package.item(base.hir_id.owner).map(|item| &item.kind)
                        else {
                            return Err(error(base.span, "self requires a method receiver"));
                        };
                        let param = sig
                            .params
                            .first()
                            .filter(|p| matches!(p.ty.kind, TyParamKind::Itself { .. }))
                            .ok_or_else(|| error(base.span, "self requires a method receiver"))?;
                        local(param.hir_id)
                    }
                    H::TacticParameter(index) => local(self.tactic_parameter(base, index)?),
                    _ => {
                        return Err(error(
                            target.span,
                            "assignment place elaboration is not implemented for this target",
                        ));
                    }
                };
                if fields.is_empty() && self.immutable.contains(&binding) {
                    return Err(error(
                        target.span,
                        "cannot reassign a let binding; use var for whole-value assignment",
                    ));
                }
                if self
                    .captures
                    .iter()
                    .any(|frame| frame.outer.contains(&binding))
                {
                    // Assignment-only use still captures the old slot: replacing
                    // it must retire the value held by this closure environment.
                    let ty = self.locals[&binding].clone();
                    for frame in &mut self.captures {
                        if frame.outer.contains(&binding)
                            && !frame
                                .values
                                .iter()
                                .any(|capture| capture.binding == binding)
                        {
                            frame.values.push(n::Capture {
                                binding,
                                ty: ty.clone(),
                                value: None,
                            });
                        }
                    }
                }
                let mut ty =
                    self.locals.get(&binding).cloned().ok_or_else(|| {
                        error(target.span, "assignment target is outside its scope")
                    })?;
                let mut path = Vec::new();
                for field in fields {
                    let shape = self.structural_type(&ty, field.span)?;
                    self.ensure_nominal_type(&shape, field.span)?;
                    if let Term::Record(fields) = &shape {
                        let index = fields
                            .iter()
                            .position(|(name, _)| name == field.name.as_str())
                            .ok_or_else(|| {
                                error(field.span, format!("unknown record field {}", field.name))
                            })?;
                        path.push((n::FieldOwner::Anonymous, index));
                        ty = fields[index].1.clone();
                        continue;
                    }
                    let Term::Nominal {
                        declaration,
                        arguments,
                    } = &shape
                    else {
                        return Err(error(
                            field.span,
                            "field assignment requires a struct place",
                        ));
                    };
                    let def = self
                        .structs
                        .get(declaration)
                        .ok_or_else(|| error(field.span, "missing struct descriptor"))?;
                    let index = def
                        .fields
                        .iter()
                        .position(|(name, _)| name == field.name.as_str())
                        .ok_or_else(|| {
                            error(field.span, format!("unknown struct field {}", field.name))
                        })?;
                    path.push((n::FieldOwner::Nominal(*declaration), index));
                    ty = self.instantiate_declaration_type(
                        &def.fields[index].1,
                        &def.type_parameters,
                        &def.parameter_types,
                        arguments,
                        field.span,
                    )?;
                }
                if bit_assignment {
                    let storage_type = self.structural_type(&ty, target.span)?;
                    let width = match storage_type {
                        Term::Primitive(P::BitVec(width)) => width,
                        Term::Primitive(primitive) if take_bits => primitive.unrestricted_scalar_width_in(self.target).map(|width| width as usize)
                            .ok_or_else(|| error(target.span,"bit writeback requires a supported scalar or BitVec representation with no invalid bit patterns"))?,
                        _ => return Err(error(target.span,"bit writeback requires a direct bit view or a BitVec place")),
                    };
                    return self.bit_writeback(
                        expr,
                        n::Place {
                            root: binding,
                            fields: path,
                        },
                        storage_type,
                        width,
                        &selections,
                        value,
                        expected,
                    );
                }
                let value = self.expr(value, Some(&ty))?;
                let result = if is_never(&value.ty) {
                    p(P::Never)
                } else {
                    p(P::Void)
                };
                (
                    result,
                    K::Assign {
                        place: n::Place {
                            root: binding,
                            fields: path,
                        },
                        value: Box::new(value),
                    },
                )
            }
            _ => unreachable!("value expression category was checked by the dispatcher"),
        };
        self.finish_value_expression(expr, expected, ty, kind)
    }
}
