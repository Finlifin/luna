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
                        |item| matches!(&item.kind, ItemKind::Fn(sig, _) | ItemKind::Case(sig, _) if sig.modifiers.is_comptime || sig.modifiers.is_normal_form_head),
                    )
            });
        if staged {
            return true;
        }
        self.readable_storage_input(ty, &mut 10_000, 0, true, false)
            .unwrap_or(false)
    }

    /// Form aggregate loads; resource checking still requires Copy and live origins.
    pub(super) fn copy_read_input(&self, ty: &Term, span: Span) -> bool {
        self.wrapped_read_input(ty)
            || self
                .readable_storage_input(ty, &mut 10_000, 0, true, true)
                .unwrap_or(false)
            || self.protocols.copy.is_some_and(|interface| {
                // Generic Copy evidence can authorize formation before the
                // payload layout is known. Instantiated resource verification
                // still checks the complete layout, Drop and live origins.
                self.establish_type_requirement(
                    &n::TypePredicate::Trait(n::TraitRequirement {
                        subject: ty.clone(),
                        interface,
                        interface_arguments: Vec::new(),
                    }),
                    span,
                )
                .is_ok()
            })
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
                        .instantiate_stored_fields(arguments, &[], fuel)
                        .map_err(|e| e.to_string())?
                } else if let Some(definition) = self.enums.get(declaration) {
                    definition
                        .instantiate_stored_fields(arguments, &[], fuel)
                        .map_err(|e| e.to_string())?
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
            // Formation can retain a rigid scalar constrained to several
            // primitive domains; instantiated loads still check resources.
            || self.proved_primitive_family(ty, &[
                P::Void, P::Bool, P::Char, P::Symbol, P::Integer, P::Float,
                P::F16, P::Bf16, P::F32, P::F64,
                P::U8, P::U16, P::U32, P::U64, P::U128, P::Usize,
                P::I8, P::I16, P::I32, P::I64, P::I128, P::Isize,
            ])
            || self
                .context
                .structural_view(ty, crate::ptc::Budget::default())
                .is_ok_and(|view| view.is_some_and(|(ty, _)| n::intrinsic_pointer_value(&ty)))
    }

    pub(super) fn static_owner_of(&mut self, expr: &Expr<'_>) -> Result<Option<OwnerId>, Error> {
        match &expr.kind {
            H::ResolvedDef { def_id, .. } => {
                let Some(owner) = self.package.definition_owner(*def_id) else {
                    return Ok(None);
                };
                if self
                    .package
                    .item(owner)
                    .is_some_and(|item| matches!(item.kind, ItemKind::Static(..)))
                {
                    Ok(Some(owner))
                } else {
                    Ok(None)
                }
            }
            H::Projection(..) => self.projected_static_owner(expr),
            _ => Ok(None),
        }
    }

    pub(super) fn static_pointer(&mut self, owner: OwnerId, span: Span) -> Result<n::Expr, Error> {
        if !self.runtime_value_context() {
            return Err(error(span, "cannot access runtime static at comptime"));
        }
        let static_def = self.ensure_static(owner, span)?;
        Ok(n::Expr {
            source: source(span),
            ty: Term::Pointer(Box::new(static_def.ty)),
            kind: K::StaticAddress(static_def.declaration),
        })
    }

    pub(super) fn dereferenced_place(&mut self, expr: &Expr<'_>) -> Result<Option<n::Expr>, Error> {
        self.dereferenced_place_with(expr, false, false)
    }

    pub(super) fn dereferenced_place_for_ref(
        &mut self,
        expr: &Expr<'_>,
    ) -> Result<Option<n::Expr>, Error> {
        self.dereferenced_place_with(expr, false, true)
    }

    pub(super) fn dereferenced_place_for_write(
        &mut self,
        expr: &Expr<'_>,
    ) -> Result<Option<n::Expr>, Error> {
        self.dereferenced_place_with(expr, true, true)
    }

    pub(super) fn dereferenced_place_with(
        &mut self,
        expr: &Expr<'_>,
        mutable: bool,
        allow_projected_base: bool,
    ) -> Result<Option<n::Expr>, Error> {
        let mut base = expr;
        let mut fields = Vec::new();
        let mut static_owner = None;
        loop {
            if let Some(owner) = self.static_owner_of(base)? {
                static_owner = Some(owner);
                break;
            }
            let H::Projection(inner, field) = &base.kind else {
                break;
            };
            if fields.len() == 128 {
                return Err(error(expr.span, "dereferenced place depth exhausted"));
            }
            fields.push(*field);
            base = inner;
        }
        let mut pointer = if let Some(owner) = static_owner {
            self.static_pointer(owner, base.span)?
        } else if matches!(base.kind, H::Deref(_) | H::MultiDeref(_)) {
            let (inner, multi) = match &base.kind {
                H::Deref(pointer) => (*pointer, false),
                H::MultiDeref(pointer) => (*pointer, true),
                _ => unreachable!(),
            };
            self.require_runtime_pointer_context(base.span, "dereference")?;
            let prev_allow = std::mem::replace(&mut self.allow_packed_pointer_projection, true);
            let inner_place = self.dereferenced_place_with(inner, mutable, false);
            self.allow_packed_pointer_projection = prev_allow;
            let mut pointer = if let Some(place_addr) = inner_place? {
                let Term::Pointer(pointee) = &place_addr.ty else {
                    unreachable!()
                };
                let inner_shape = self.structural_type(pointee, base.span)?;
                if matches!(inner_shape, Term::Pointer(_)) {
                    if let K::AddressOf(place) = place_addr.kind {
                        *place
                    } else {
                        n::Expr {
                            source: source(base.span),
                            ty: (**pointee).clone(),
                            kind: K::ReadPointer(Box::new(place_addr)),
                        }
                    }
                } else if !multi
                    && let Some(step) = self.query_deref_step(pointee, mutable, base.span)?
                {
                    self.emit_deref_call(base, place_addr, &step)?
                } else {
                    return Err(error(
                        base.span,
                        if multi {
                            "multi_deref requires a pointer"
                        } else {
                            "dereference requires a pointer or `Deref` implementation"
                        },
                    ));
                }
            } else {
                let prev_allow = std::mem::replace(&mut self.allow_packed_pointer_projection, true);
                let operand = self.expr(inner, None);
                self.allow_packed_pointer_projection = prev_allow;
                let operand = operand?;
                let shape = self.structural_type(&operand.ty, base.span)?;
                if matches!(shape, Term::Pointer(_)) {
                    operand
                } else if !multi
                    && let Some(step) = self.query_deref_step(&operand.ty, mutable, base.span)?
                {
                    let addr = self.ensure_address_of_value(operand, inner.span)?;
                    self.emit_deref_call(base, addr, &step)?
                } else {
                    return Err(error(
                        base.span,
                        if multi {
                            "multi_deref requires a pointer"
                        } else {
                            "dereference requires a pointer or `Deref` implementation"
                        },
                    ));
                }
            };
            let Term::Pointer(mut pointee) = self.structural_type(&pointer.ty, base.span)? else {
                return Err(error(base.span, "dereference did not yield a pointer"));
            };
            if multi {
                let mut depth = 0usize;
                loop {
                    let inner_shape = self.structural_type(&pointee, base.span)?;
                    let Term::Pointer(next_pointee) = inner_shape else {
                        break;
                    };
                    depth += 1;
                    if depth >= 128 {
                        return Err(error(base.span, "multi_deref depth exhausted"));
                    }
                    pointer = n::Expr {
                        source: source(base.span),
                        ty: (*pointee).clone(),
                        kind: K::ReadPointer(Box::new(pointer)),
                    };
                    pointee = next_pointee;
                }
            }
            pointer
        } else if allow_projected_base && !fields.is_empty() {
            // If a projected base is itself a pointer (for writes) or implements
            // Deref/DerefMut (and does not own the projected field directly), resolve
            // its address without evaluating it twice so Layer 2 can project through it.
            let val = self.expr(base, None)?;
            let shape = self.structural_type(&val.ty, base.span)?;
            let first_field = fields.last().unwrap().name.as_str();
            let has_direct_field = match &shape {
                Term::Record(entries) => entries.iter().any(|(name, _)| name == first_field),
                Term::Nominal { declaration, .. } => self
                    .structs
                    .get(declaration)
                    .is_some_and(|def| def.fields.iter().any(|(name, _)| name == first_field)),
                _ => false,
            };
            if mutable && matches!(shape, Term::Pointer(_)) {
                val
            } else if !has_direct_field
                && !matches!(shape, Term::Pointer(_))
                && self
                    .query_deref_step(&val.ty, mutable, base.span)?
                    .is_some()
            {
                self.ensure_address_of_value(val, base.span)?
            } else {
                return Ok(None);
            }
        } else {
            return Ok(None);
        };
        for field in fields.into_iter().rev() {
            let mut visited = Vec::new();
            let mut depth = 0usize;
            let shape = loop {
                let Term::Pointer(subject) = &pointer.ty else {
                    unreachable!()
                };
                let candidate_shape = self.structural_type(subject, field.span)?;
                self.ensure_nominal_type(&candidate_shape, field.span)?;
                let has_field = match &candidate_shape {
                    Term::Record(entries) => {
                        entries.iter().any(|(name, _)| name == field.name.as_str())
                    }
                    Term::Nominal { declaration, .. } => {
                        self.structs.get(declaration).is_some_and(|def| {
                            def.fields
                                .iter()
                                .any(|(name, _)| name == field.name.as_str())
                        })
                    }
                    _ => false,
                };
                if has_field {
                    break candidate_shape;
                }
                if matches!(&candidate_shape, Term::Pointer(_)) {
                    let pointee = (**subject).clone();
                    self.check_deref_cycle(&mut visited, &pointee, depth, field.span)?;
                    depth += 1;
                    pointer = n::Expr {
                        source: source(field.span),
                        ty: pointee,
                        kind: K::ReadPointer(Box::new(pointer)),
                    };
                    continue;
                }
                let subject_owned = (**subject).clone();
                if let Some(step) = self.query_deref_step(&subject_owned, mutable, field.span)? {
                    let target = step.target.clone();
                    self.check_deref_cycle(&mut visited, &target, depth, field.span)?;
                    depth += 1;
                    pointer = self.emit_deref_call(expr, pointer, &step)?;
                    continue;
                }
                break candidate_shape;
            };
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
            let ty = self.instantiate_declaration_type(
                &definition.fields[index].1,
                &definition.type_parameters,
                &definition.parameter_types,
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
            let (operand, place) = if let Some(pointer) =
                self.dereferenced_place_for_write(storage)?
            {
                (pointer, None)
            } else {
                let read = self.expr(storage, None)?;
                let mut base = &read;
                let mut fields = Vec::new();
                loop {
                    match &base.kind {
                        K::Field {
                            value,
                            declaration,
                            index,
                        } => {
                            fields.push((n::FieldOwner::Nominal(*declaration), *index));
                            base = value;
                        }
                        K::AnonymousField { value, index, .. } => {
                            fields.push((n::FieldOwner::Anonymous, *index));
                            base = value;
                        }
                        _ => break,
                    }
                }
                let K::Local(root) = base.kind else {
                    return Err(error(
                        target.span,
                        "bit compound assignment requires a stable local field or dereferenced place",
                    ));
                };
                fields.reverse();
                if fields.is_empty() && self.immutable.contains(&root) {
                    return Err(error(
                        target.span,
                        "cannot reassign a let binding; use var for whole-value assignment",
                    ));
                }
                (read, Some(n::Place { root, fields }))
            };
            let ty = if place.is_some() {
                &operand.ty
            } else {
                let Term::Pointer(ty) = &operand.ty else {
                    unreachable!()
                };
                ty
            };
            let shape = self.structural_type(ty, target.span)?;
            let width = self.bit_writeback_storage_width(&shape, take_bits, target.span)?;
            return self.bit_compound_writeback(
                expression, op, operand, place, width, &indices, value, expected,
            );
        }
        let Some(pointer) = self.dereferenced_place_for_write(target)? else {
            // Restrict this rewrite to stable local field paths. Indexing needs
            // its own prepared location protocol, not duplicated source code.
            let mut base = target;
            while let H::Projection(inner, _) = &base.kind {
                base = inner;
            }
            if !matches!(
                base.kind,
                H::ResolvedLocal(..)
                    | H::ResolvedFnParam(..)
                    | H::TacticParameter(_)
                    | H::SelfValue
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
        let mut base_pointer = &pointer;
        let mut field_idents = Vec::new();
        while self.pointer_targets_packed_field(base_pointer) {
            match &base_pointer.kind {
                K::PointerField {
                    pointer: inner,
                    declaration,
                    index,
                } => {
                    let name = self
                        .structs
                        .get(declaration)
                        .and_then(|def| def.fields.get(*index))
                        .map(|(name, _)| name.as_str())
                        .ok_or_else(|| {
                            error(
                                target.span,
                                "compound assignment missing packed struct field",
                            )
                        })?;
                    field_idents.push(hir::common::Ident::new(
                        hir::Symbol::intern(name),
                        target.span,
                    ));
                    base_pointer = inner;
                }
                K::PointerAnonymousField {
                    pointer: inner,
                    name,
                    ..
                } => {
                    field_idents.push(hir::common::Ident::new(
                        hir::Symbol::intern(name),
                        target.span,
                    ));
                    base_pointer = inner;
                }
                _ => break,
            }
        }
        let binding = self.spread_binding(target.span)?;
        let id = hir::HirId::new(hir::OwnerId::INVALID, hir::ItemLocalId::new(binding.index));
        let local_expr = Expr {
            hir_id: id,
            kind: H::ResolvedLocal(hir::Symbol::invalid(), id),
            span: target.span,
        };
        let prepared_base = Expr {
            kind: H::Deref(&local_expr),
            ..target.clone()
        };
        let (prepare_value, checked) = if !field_idents.is_empty() {
            field_idents.reverse();
            let base_value = base_pointer.clone();
            self.locals.insert(binding, base_value.ty.clone());
            let checked = self.check_projected_compound_assign(
                &prepared_base,
                &field_idents,
                op,
                expression,
                value,
                expected,
            );
            self.locals.remove(&binding);
            (base_value, checked)
        } else {
            let binary = Expr {
                kind: H::Binary(*op, &prepared_base, value),
                ..expression.clone()
            };
            let assign = Expr {
                kind: H::Assign(&prepared_base, &binary),
                ..expression.clone()
            };
            self.locals.insert(binding, pointer.ty.clone());
            let checked = self.expr(&assign, expected);
            self.locals.remove(&binding);
            (pointer, checked)
        };
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
                value: Box::new(prepare_value),
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

    fn check_projected_compound_assign<'hir>(
        &mut self,
        current: &Expr<'hir>,
        remaining: &[hir::common::Ident],
        op: &BinOp,
        expression: &Expr<'hir>,
        value: &Expr<'hir>,
        expected: Option<&Term>,
    ) -> Result<n::Expr, Error> {
        if let Some((first, rest)) = remaining.split_first() {
            let projected = Expr {
                kind: H::Projection(current, *first),
                span: expression.span,
                ..current.clone()
            };
            return self.check_projected_compound_assign(
                &projected, rest, op, expression, value, expected,
            );
        }
        let binary = Expr {
            kind: H::Binary(*op, current, value),
            ..expression.clone()
        };
        let assign = Expr {
            kind: H::Assign(current, &binary),
            ..expression.clone()
        };
        self.expr(&assign, expected)
    }
}
