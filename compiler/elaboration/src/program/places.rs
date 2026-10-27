//! Preserve dereferenced storage through field selection. This builds addresses;
//! reading or transferring the contents is a separate operation at the use site.
use super::*;

impl Checker<'_, '_> {
    /// Formation only: nominal Copy and resource legality are checked later.
    pub(super) fn wrapped_read_input(&self, ty: &Term) -> bool {
        // A staged generic body can form a transfer before its representation
        // is known. Required invocation checks the instantiated type against
        // the immutable Copy/Drop world before reading or replacing storage.
        // Quoted runtime code still follows the runtime formation rules.
        let staged = !self.in_code_body
            && self.captures.last().map(|frame| frame.comptime).unwrap_or_else(|| {
                self.in_required_comptime
                    || self.in_staged_input
                    || self.signature_owner.and_then(|owner| self.package.item(owner)).is_some_and(
                        |item| matches!(&item.kind, ItemKind::Fn(sig, _) if sig.modifiers.is_comptime || sig.modifiers.is_normal_form_head),
                    )
            });
        if staged {
            return true;
        }
        self.readable_storage_input(ty, &mut 10_000, 0, true, false)
            .unwrap_or(false)
    }
    /// Form aggregate loads; resource checking still requires Copy and live origins.
    pub(super) fn copy_read_input(&self, ty: &Term) -> bool {
        self.wrapped_read_input(ty)
            || self
                .readable_storage_input(ty, &mut 10_000, 0, true, true)
                .unwrap_or(false)
    }
    fn readable_storage_input(
        &self,
        ty: &Term,
        fuel: &mut usize,
        depth: usize,
        pointer: bool,
        nested_pointers: bool,
    ) -> Result<bool, String> {
        if depth >= 128 {
            return Err("read layout depth exhausted".into());
        }
        crate::verification::memory::bound_type(ty, fuel)?;
        if n::intrinsic_pointer_value(ty) {
            return Ok(pointer || !matches!(ty, Term::Pointer(_)));
        }
        if let Some(fields) = self.bound_tuple_elements(ty).or_else(|| match ty {
            Term::Tuple(fields) => Some(fields.as_slice()),
            _ => None,
        }) {
            for field in fields {
                if !self.readable_storage_input(
                    field,
                    fuel,
                    depth + 1,
                    pointer && nested_pointers,
                    nested_pointers,
                )? {
                    return Ok(false);
                }
            }
            return Ok(true);
        }
        if matches!(ty, Term::Function { .. } | Term::Primitive(P::Str)) {
            // Form a descriptor transfer without granting Copy or proving the
            // lifetime of the separate backing character region. Resource
            // verification must retain those obligations for text views.
            return Ok(true);
        }
        let fields = match ty {
            Term::ErrorResult { errors, result } => {
                // Formation inspects every possible payload. This does not
                // grant Copy: the complete result and its active payload must
                // still pass the resource world's Copy/Drop checks.
                let mut fields = errors.clone();
                if !is_never(result) {
                    fields.push((**result).clone());
                }
                fields
            }
            Term::Record(fields) => fields.iter().map(|(_, ty)| ty.clone()).collect::<Vec<_>>(),
            Term::Nominal {
                declaration,
                arguments,
            } => {
                if let Some(definition) = self.newtypes.get(declaration) {
                    let value = definition
                        .representation_budgeted(arguments, &[], fuel)
                        .map_err(|e| e.to_string())?;
                    return self.readable_storage_input(
                        &value,
                        fuel,
                        depth + 1,
                        pointer,
                        nested_pointers,
                    );
                }
                if let Some(definition) = self.structs.get(declaration) {
                    definition
                        .instantiate_fields(arguments)
                        .map_err(|e| e.to_string())?
                        .into_iter()
                        .map(|(_, ty)| ty)
                        .collect()
                } else if let Some(definition) = self.enums.get(declaration) {
                    definition
                        .instantiate_variants(arguments)
                        .map_err(|e| e.to_string())?
                        .into_iter()
                        .flat_map(|variant| variant.fields)
                        .collect()
                } else {
                    return Ok(false);
                }
            }
            _ => return Ok(false),
        };
        for field in fields {
            if !self.readable_storage_input(
                &field,
                fuel,
                depth + 1,
                pointer && nested_pointers,
                nested_pointers,
            )? {
                return Ok(false);
            }
        }
        Ok(true)
    }
    pub(super) fn intrinsic_pointer_input(&self, ty: &Term) -> bool {
        n::intrinsic_pointer_value(ty)
            || self
                .context
                .structural_view(ty, crate::ptc::Budget::default())
                .is_ok_and(|view| view.is_some_and(|(ty, _)| n::intrinsic_pointer_value(&ty)))
    }

    pub(super) fn dereferenced_place(&mut self, expr: &Expr<'_>) -> Result<Option<n::Expr>, Error> {
        let mut base = expr;
        let mut fields = Vec::new();
        while let H::Projection(inner, field) = &base.kind {
            if fields.len() == 128 {
                return Err(error(expr.span, "dereferenced place depth exhausted"));
            }
            fields.push(*field);
            base = inner;
        }
        let H::Deref(pointer) = &base.kind else {
            return Ok(None);
        };
        let mut pointer = self.expr(pointer, None)?;
        if !matches!(pointer.ty, Term::Pointer(_)) {
            return Err(error(base.span, "dereference requires a pointer"));
        }
        for field in fields.into_iter().rev() {
            let Term::Pointer(subject) = &pointer.ty else {
                unreachable!()
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
                pointer = n::Expr {
                    source: source(expr.span),
                    ty: Term::Pointer(Box::new(fields[index].1.clone())),
                    kind: K::PointerAnonymousField {
                        pointer: Box::new(pointer),
                        name: field.name.to_string(),
                        index,
                    },
                };
                continue;
            }

            let Term::Nominal {
                declaration,
                arguments,
            } = shape
            else {
                return Err(error(
                    field.span,
                    "dereferenced field place requires a struct; spell pointer indirection explicitly",
                ));
            };
            let definition = self
                .structs
                .get(&declaration)
                .ok_or_else(|| error(field.span, "dereferenced field place requires a struct"))?;
            let index = definition
                .fields
                .iter()
                .position(|(name, _)| name == field.name.as_str())
                .ok_or_else(|| error(field.span, format!("unknown struct field {}", field.name)))?;
            let ty = self.instantiate_input_types(
                &definition.fields[index].1,
                &definition.type_parameters,
                &arguments,
                field.span,
            )?;
            pointer = n::Expr {
                source: source(expr.span),
                ty: Term::Pointer(Box::new(ty)),
                kind: K::PointerField {
                    pointer: Box::new(pointer),
                    declaration,
                    index,
                },
            };
        }
        Ok(Some(pointer))
    }
}

impl Checker<'_, '_> {
    /// Reuse ordinary operator selection and writeback. Pointer-producing
    /// targets are prepared once; local/field paths have no address effects.
    pub(super) fn compound_assignment(
        &mut self,
        expression: &Expr<'_>,
        op: &BinOp,
        target: &Expr<'_>,
        value: &Expr<'_>,
        expected: Option<&Term>,
    ) -> Result<n::Expr, Error> {
        let mut storage = target;
        let mut indices = Vec::new();
        while let H::Index(base, index) = &storage.kind {
            indices.push(*index);
            storage = base;
        }
        indices.reverse();
        let take_bits = if let H::TakeBitVec(base) = &storage.kind {
            storage = base;
            true
        } else {
            false
        };
        if take_bits || !indices.is_empty() {
            let pointer = if let Some(pointer) = self.dereferenced_place(storage)? {
                pointer
            } else {
                if let H::ResolvedLocal(_, id) | H::ResolvedFnParam(_, id) = storage.kind {
                    if self.immutable.contains(&local(id)) {
                        return Err(error(
                            target.span,
                            "cannot reassign a let binding; use var for whole-value assignment",
                        ));
                    }
                }
                self.expr(
                    &Expr {
                        kind: H::Ref(storage),
                        ..storage.clone()
                    },
                    None,
                )?
            };
            let Term::Pointer(ty) = &pointer.ty else {
                unreachable!()
            };
            let shape = self.structural_type(ty, target.span)?;
            let width = match shape {
                Term::Primitive(P::BitVec(width)) => width,
                Term::Primitive(primitive) if take_bits => primitive
                    .unrestricted_scalar_width_in(self.target)
                    .map(|width| width as usize)
                    .ok_or_else(|| {
                        error(
                            target.span,
                            "bit writeback requires an unrestricted scalar representation",
                        )
                    })?,
                _ => {
                    return Err(error(
                        target.span,
                        "bit writeback requires a direct bit view or a BitVec place",
                    ));
                }
            };
            return self
                .bit_compound_writeback(expression, op, pointer, width, &indices, value, expected);
        }
        let Some(pointer) = self.dereferenced_place(target)? else {
            // Restrict this rewrite to stable local field paths. Indexing needs
            // its own prepared location protocol, not duplicated source code.
            let mut base = target;
            while let H::Projection(inner, _) = &base.kind {
                base = inner;
            }
            if !matches!(
                base.kind,
                H::ResolvedLocal(..) | H::ResolvedFnParam(..) | H::TacticParameter(_)
            ) {
                return Err(error(
                    target.span,
                    "compound assignment requires a local field or dereferenced place; indexed locations need a prepared writeback protocol",
                ));
            }
            let binary = Expr {
                kind: H::Binary(*op, target, value),
                ..expression.clone()
            };
            let assign = Expr {
                kind: H::Assign(target, &binary),
                ..expression.clone()
            };
            return self.expr(&assign, expected);
        };
        let binding = self.spread_binding(target.span)?;
        let id = hir::HirId::new(hir::OwnerId::INVALID, hir::ItemLocalId::new(binding.index));
        let local_expr = Expr {
            hir_id: id,
            kind: H::ResolvedLocal(hir::Symbol::invalid(), id),
            span: target.span,
        };
        let prepared_target = Expr {
            kind: H::Deref(&local_expr),
            ..target.clone()
        };
        let binary = Expr {
            kind: H::Binary(*op, &prepared_target, value),
            ..expression.clone()
        };
        let assign = Expr {
            kind: H::Assign(&prepared_target, &binary),
            ..expression.clone()
        };
        self.locals.insert(binding, pointer.ty.clone());
        let checked = self.expr(&assign, expected);
        self.locals.remove(&binding);
        let mut checked = checked?;
        let original = source(expression.span);
        let mut generated = || -> Result<n::SourceRange, Error> {
            let index = u32::try_from(self.expansion_sources.len()).map_err(|_| {
                error(
                    expression.span,
                    "compound assignment source space exhausted",
                )
            })?;
            self.expansion_sources.push(n::ExpansionSource {
                original,
                site: original,
            });
            Ok(n::SourceRange::generated(index))
        };
        checked.source = generated()?;
        let prepare = n::Expr {
            ty: p(P::Void),
            source: generated()?,
            kind: K::Let {
                binding,
                value: Box::new(pointer),
            },
        };
        Ok(n::Expr {
            ty: checked.ty.clone(),
            source: original,
            kind: K::Block {
                statements: vec![prepare],
                result: Some(Box::new(checked)),
            },
        })
    }
}
