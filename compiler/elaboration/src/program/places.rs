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
        if let Some((element, len)) = self.array_element_and_len(ty) {
            if len == 0 {
                return Ok(true);
            }
            return self.readable_storage_input(
                &element,
                fuel,
                depth + 1,
                pointer && nested_pointers,
                nested_pointers,
            );
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
        } else if allow_projected_base
            && let H::Index(index_base, args) = &base.kind
            && let Some(index_ptr) = self.index_place_pointer(base, index_base, args, mutable)?
        {
            index_ptr
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
        if is_never(&pointer.ty) {
            return Ok(Some(pointer));
        }
        for field in fields.into_iter().rev() {
            let mut visited = Vec::new();
            let mut depth = 0usize;
            let shape = loop {
                let Term::Pointer(subject) = &pointer.ty else {
                    unreachable!()
                };
                let resolved_subject = if subject
                    .structural_terms()
                    .any(|term| matches!(term, Term::AssociatedTypeCall(_)))
                {
                    Some(self.resolve_ptc_associated_calls(subject, field.span)?)
                } else {
                    None
                };
                let candidate_subject = resolved_subject.as_ref().unwrap_or(subject);
                let candidate_shape = self.structural_type(candidate_subject, field.span)?;
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
                let field_sym = Term::Symbol(field.name.as_str().into());
                let has_field_pred = n::TypePredicate::HasField {
                    subject: shape.clone(),
                    field: field_sym.clone(),
                };
                if self
                    .establish_type_requirement(&has_field_pred, field.span)
                    .is_ok()
                {
                    let field_ty = Term::field_type(shape, field_sym);
                    pointer = n::Expr {
                        source: source(expr.span),
                        ty: Term::Pointer(Box::new(field_ty)),
                        kind: K::FieldPtr {
                            pointer: Box::new(pointer),
                            field: Box::new(n::Expr {
                                source: source(field.span),
                                ty: p(P::Symbol),
                                kind: K::Literal(n::Literal::Symbol(field.name.to_string())),
                            }),
                        },
                    };
                    continue;
                }
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

    /// Build a storage pointer `&target` (`*T`) for a local variable or local field path,
    /// or return the pointer directly when the local/field already holds `*U`.
    pub(super) fn local_storage_pointer(
        &mut self,
        expr: &Expr<'_>,
        mutable: bool,
    ) -> Result<Option<n::Expr>, Error> {
        let mut base = expr;
        let mut fields = Vec::new();
        while let H::Projection(inner, field) = &base.kind {
            if fields.len() == 128 {
                return Err(error(expr.span, "storage place depth exhausted"));
            }
            fields.push(*field);
            base = inner;
        }
        if !matches!(
            base.kind,
            H::ResolvedLocal(..) | H::ResolvedFnParam(..) | H::TacticParameter(_) | H::SelfValue
        ) {
            return Ok(None);
        }
        if fields.is_empty()
            && let H::ResolvedLocal(_, hir_id) = base.kind
            && !self.locals.contains_key(&local(hir_id))
        {
            return Ok(None);
        }
        let val = self.expr(base, None)?;
        let root_ty = val.ty.clone();
        let root_shape = self.structural_type(&root_ty, base.span)?;
        let K::Local(root_local) = val.kind else {
            return Ok(None);
        };
        if mutable
            && self.immutable.contains(&root_local)
            && !(fields.is_empty() && matches!(root_shape, Term::Pointer(_)))
        {
            // Check whether an intermediate field is a pointer / DerefMut, or reject immutable binding.
            if fields.is_empty() {
                return Err(error(
                    base.span,
                    "cannot mutably borrow an immutable `let` binding; use `var`",
                ));
            }
        }
        let mut pointer = if fields.is_empty() && matches!(root_shape, Term::Pointer(_)) {
            val
        } else {
            if self.comptime_pointer_forbidden() {
                return Ok(None);
            }
            let addr = n::Expr {
                ty: Term::Pointer(Box::new(root_ty)),
                source: source(base.span),
                kind: K::AddressOf(Box::new(val)),
            };
            if let Some(message) = n::pointer_operation_error_in(&addr, self.target) {
                return Err(error(base.span, message));
            }
            addr
        };
        for field in fields.into_iter().rev() {
            let mut visited = Vec::new();
            let mut depth = 0usize;
            let shape = loop {
                let Term::Pointer(subject) = &pointer.ty else {
                    unreachable!()
                };
                let resolved_subject = if subject
                    .structural_terms()
                    .any(|term| matches!(term, Term::AssociatedTypeCall(_)))
                {
                    Some(self.resolve_ptc_associated_calls(subject, field.span)?)
                } else {
                    None
                };
                let candidate_subject = resolved_subject.as_ref().unwrap_or(subject);
                let candidate_shape = self.structural_type(candidate_subject, field.span)?;
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
            if let Term::Record(entries) = &shape {
                let index = entries
                    .iter()
                    .position(|(name, _)| name == field.name.as_str())
                    .ok_or_else(|| {
                        error(field.span, format!("unknown record field {}", field.name))
                    })?;
                pointer = n::Expr {
                    source: source(expr.span),
                    ty: Term::Pointer(Box::new(entries[index].1.clone())),
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
                let field_sym = Term::Symbol(field.name.as_str().into());
                let has_field_pred = n::TypePredicate::HasField {
                    subject: shape.clone(),
                    field: field_sym.clone(),
                };
                if self
                    .establish_type_requirement(&has_field_pred, field.span)
                    .is_ok()
                {
                    let field_ty = Term::field_type(shape, field_sym);
                    pointer = n::Expr {
                        source: source(expr.span),
                        ty: Term::Pointer(Box::new(field_ty)),
                        kind: K::FieldPtr {
                            pointer: Box::new(pointer),
                            field: Box::new(n::Expr {
                                source: source(field.span),
                                ty: p(P::Symbol),
                                kind: K::Literal(n::Literal::Symbol(field.name.to_string())),
                            }),
                        },
                    };
                    continue;
                }
                return Err(error(
                    field.span,
                    "projected storage place requires a struct or record",
                ));
            };
            let definition = self
                .structs
                .get(&declaration)
                .ok_or_else(|| error(field.span, "projected storage place requires a struct"))?;
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

    pub(super) fn index_base_place_pointer(
        &mut self,
        base: &Expr<'_>,
        mutable: bool,
    ) -> Result<Option<n::Expr>, Error> {
        if matches!(base.kind, H::TakeBitVec(_)) {
            return Ok(None);
        }
        if let Some(pointer) = self.dereferenced_place_with(base, mutable, true)? {
            return Ok(Some(pointer));
        }
        self.local_storage_pointer(base, mutable)
    }

    pub(super) fn adapt_receiver_index_context(
        &mut self,
        receiver_ptr_ty: &Term,
        index_proto: nir::DeclarationId,
        span: Span,
    ) -> Result<Option<Term>, Error> {
        let mut ty = receiver_ptr_ty.clone();
        let mut visited = Vec::new();
        let mut depth = 0usize;
        loop {
            let Term::Pointer(subject) = ty else {
                return Ok(None);
            };
            if let Some(ctx) = self.protocol_argument_context(&subject, index_proto, span)? {
                return Ok(Some(ctx));
            }
            let shape = self.structural_type(&subject, span)?;
            if matches!(shape, Term::Pointer(_)) {
                let next = (*subject).clone();
                self.check_deref_cycle(&mut visited, &next, depth, span)?;
                depth += 1;
                ty = next;
                continue;
            }
            if let Some(step) = self.query_deref_step(&subject, false, span)? {
                let next = step.target.clone();
                self.check_deref_cycle(&mut visited, &next, depth, span)?;
                depth += 1;
                ty = Term::Pointer(Box::new(next));
                continue;
            }
            return Ok(None);
        }
    }

    pub(super) fn index_place_pointer(
        &mut self,
        expr: &Expr<'_>,
        base: &Expr<'_>,
        args: &[hir::common::Arg<'_>],
        mutable: bool,
    ) -> Result<Option<n::Expr>, Error> {
        let [hir::common::Arg::Positional(index_hir)] = args else {
            return Ok(None);
        };
        if matches!(base.kind, H::TakeBitVec(_)) {
            return Ok(None);
        }
        let Some(receiver_ptr) = self.index_base_place_pointer(base, mutable)? else {
            return Ok(None);
        };
        let Some(index_id) = self.protocols.index else {
            return Ok(None);
        };
        let Term::Pointer(inner) = &receiver_ptr.ty else {
            return Ok(None);
        };
        let shape = self.structural_type(inner, base.span)?;
        if matches!(shape, Term::Primitive(P::BitVec(_)))
            || self.list_element_type(&shape).is_some()
        {
            return Ok(None);
        }
        let arg_ctx = self.adapt_receiver_index_context(&receiver_ptr.ty, index_id, expr.span)?;
        let arg_expr = match self.argument_attempt(index_hir, None, None)? {
            Some(argument) => argument,
            None => self.expr(index_hir, arg_ctx.as_ref())?,
        };
        match self.resolve_index_call_on_receiver(expr, receiver_ptr, arg_expr, mutable, None)? {
            IndexResolution::Selected(call) => Ok(Some(call)),
            IndexResolution::NoCandidate { .. } => Ok(None),
        }
    }

    pub(super) fn spill_receiver_for_address(
        &mut self,
        value: n::Expr,
        span: Span,
    ) -> Result<(Option<n::Expr>, n::Expr), Error> {
        if let K::ReadPointer(pointer) = value.kind {
            return Ok((None, *pointer));
        }
        let ptr_ty = Term::Pointer(Box::new(value.ty.clone()));
        let addr = n::Expr {
            ty: ptr_ty.clone(),
            source: source(span),
            kind: K::AddressOf(Box::new(value.clone())),
        };
        if n::pointer_operation_error_in(&addr, self.target).is_none() {
            return Ok((None, addr));
        }
        let binding = self.spread_binding(span)?;
        let val_ty = value.ty.clone();
        let let_stmt = n::Expr {
            source: value.source,
            ty: if is_never(&val_ty) {
                p(P::Never)
            } else {
                p(P::Void)
            },
            kind: K::Let {
                binding,
                value: Box::new(value),
            },
        };
        let spilled_addr = n::Expr {
            ty: ptr_ty,
            source: source(span),
            kind: K::AddressOf(Box::new(n::Expr {
                source: source(span),
                ty: val_ty,
                kind: K::Local(binding),
            })),
        };
        Ok((Some(let_stmt), spilled_addr))
    }

    pub(super) fn wrap_temp_stmt(&self, temp_stmt: Option<n::Expr>, body: n::Expr) -> n::Expr {
        let Some(stmt) = temp_stmt else {
            return body;
        };
        n::Expr {
            ty: body.ty.clone(),
            source: body.source,
            kind: K::Block {
                statements: vec![stmt],
                result: Some(Box::new(body)),
            },
        }
    }

    pub(super) fn resolve_index_call_on_receiver(
        &mut self,
        expr: &Expr<'_>,
        mut receiver_ptr: n::Expr,
        arg_expr: n::Expr,
        mutable: bool,
        expected: Option<&Term>,
    ) -> Result<IndexResolution, Error> {
        if is_never(&arg_expr.ty) {
            if let Some(expected) = expected {
                self.require_expression(&p(P::Never), expected, expr.span)?;
            }
            let unused = self.spread_binding(expr.span)?;
            let keep_receiver = n::Expr {
                source: receiver_ptr.source,
                ty: p(P::Void),
                kind: K::Let {
                    binding: unused,
                    value: Box::new(receiver_ptr),
                },
            };
            return Ok(IndexResolution::Selected(n::Expr {
                source: source(expr.span),
                ty: p(P::Never),
                kind: K::Block {
                    statements: vec![keep_receiver],
                    result: Some(Box::new(arg_expr)),
                },
            }));
        }
        let Some(index_id) = self.protocols.index else {
            return Ok(IndexResolution::NoCandidate {
                receiver_ptr,
                arg_expr,
            });
        };
        let saved_ptr = receiver_ptr.clone();
        let index_iface = n::InterfaceInstance {
            declaration: index_id,
            arguments: vec![arg_expr.ty.clone()],
        };
        let mut visited = Vec::new();
        let mut depth = 0usize;
        loop {
            let Term::Pointer(subject_ref) = &receiver_ptr.ty else {
                unreachable!("index receiver must be a pointer");
            };
            let subject = (**subject_ref).clone();
            if self.query_single_trait_candidate(&subject, &index_iface, "Index", expr.span)? {
                let (selected_iface, method_name) = if mutable {
                    let Some(index_mut_id) = self.protocols.index_mut else {
                        return Err(error(
                            expr.span,
                            format!(
                                "`{subject}` implements `Index`, but mutable indexing requires a bound `IndexMut` protocol"
                            ),
                        ));
                    };
                    let index_mut_iface = n::InterfaceInstance {
                        declaration: index_mut_id,
                        arguments: vec![arg_expr.ty.clone()],
                    };
                    if !self.query_single_trait_candidate(
                        &subject,
                        &index_mut_iface,
                        "IndexMut",
                        expr.span,
                    )? {
                        return Err(error(
                            expr.span,
                            format!(
                                "`{subject}` implements `Index` but not `IndexMut`; cannot mutably index"
                            ),
                        ));
                    }
                    (index_mut_iface, "index_mut")
                } else {
                    (index_iface, "index")
                };
                let callee = self
                    .selected_method(
                        subject.clone(),
                        Some(selected_iface),
                        method_name,
                        expr.span,
                    )?
                    .ok_or_else(|| {
                        error(
                            expr.span,
                            format!("`{method_name}` method on `{subject}` did not resolve"),
                        )
                    })?;
                let call = self.finish_call(
                    expr,
                    callee,
                    &[],
                    vec![receiver_ptr, arg_expr],
                    expected,
                    None,
                )?;
                return Ok(IndexResolution::Selected(call));
            }
            let shape = self.structural_type(&subject, expr.span)?;
            if matches!(shape, Term::Pointer(_)) {
                self.check_deref_cycle(&mut visited, &subject, depth, expr.span)?;
                depth += 1;
                receiver_ptr = n::Expr {
                    source: source(expr.span),
                    ty: subject,
                    kind: K::ReadPointer(Box::new(receiver_ptr)),
                };
                continue;
            }
            if let Some(step) = self.query_deref_step(&subject, mutable, expr.span)? {
                let next = step.target.clone();
                self.check_deref_cycle(&mut visited, &next, depth, expr.span)?;
                depth += 1;
                receiver_ptr = self.emit_deref_call(expr, receiver_ptr, &step)?;
                continue;
            }
            return Ok(IndexResolution::NoCandidate {
                receiver_ptr: saved_ptr,
                arg_expr,
            });
        }
    }

    pub(super) fn space_apply_with_args(
        &mut self,
        expr: &Expr<'_>,
        mut receiver_ptr: n::Expr,
        args: &[hir::common::Arg<'_>],
        pre_evaluated_first: Option<n::Expr>,
        expected: Option<&Term>,
    ) -> Result<n::Expr, Error> {
        let Some(space_apply_id) = self.protocols.space_apply else {
            return Err(error(
                expr.span,
                "multi-argument or view indexing requires a bound SpaceApply protocol",
            ));
        };
        let space_apply_iface = n::InterfaceInstance {
            declaration: space_apply_id,
            arguments: vec![],
        };
        let mut visited = Vec::new();
        let mut depth = 0usize;
        let subject = loop {
            let Term::Pointer(subject_ref) = &receiver_ptr.ty else {
                unreachable!("space_apply receiver must be a pointer");
            };
            let candidate = (**subject_ref).clone();
            if self.query_single_trait_candidate(
                &candidate,
                &space_apply_iface,
                "SpaceApply",
                expr.span,
            )? {
                break candidate;
            }
            let shape = self.structural_type(&candidate, expr.span)?;
            if matches!(shape, Term::Pointer(_)) {
                self.check_deref_cycle(&mut visited, &candidate, depth, expr.span)?;
                depth += 1;
                receiver_ptr = n::Expr {
                    source: source(expr.span),
                    ty: candidate,
                    kind: K::ReadPointer(Box::new(receiver_ptr)),
                };
                continue;
            }
            if let Some(step) = self.query_deref_step(&candidate, false, expr.span)? {
                let next = step.target.clone();
                self.check_deref_cycle(&mut visited, &next, depth, expr.span)?;
                depth += 1;
                receiver_ptr = self.emit_deref_call(expr, receiver_ptr, &step)?;
                continue;
            }
            return Err(error(
                expr.span,
                format!("type `{candidate}` does not implement `Index` or `SpaceApply`"),
            ));
        };
        let collect_to_member = self
            .checked_traits
            .by_id(space_apply_id)
            .and_then(|contract| {
                contract
                    .associated
                    .iter()
                    .find(|member| member.name == "CollectTo")
            })
            .ok_or_else(|| {
                error(
                    expr.span,
                    format!("`{subject}` SpaceApply implementation lacks CollectTo"),
                )
            })?
            .declaration;
        let collect_to_assoc = Term::AssociatedType {
            origin: Box::new(nir::AssociatedTypeOrigin {
                interface: space_apply_id,
                member: collect_to_member,
            }),
            arguments: vec![],
            subject: Box::new(subject.clone()),
        };
        let assumptions = self.capability_assumptions();
        let collect_to_resolved = crate::capabilities::World {
            index: &self.implementation_index,
            implementations: &self.checked_implementations,
            traits: &self.checked_traits,
            assumptions: &assumptions,
            context: &self.context,
        }
        .associated_type_view_metered(&collect_to_assoc, &mut crate::ptc::Budget::default())
        .map(|view| view.value)
        .unwrap_or(collect_to_assoc);
        let collect_to_ty = self
            .structural_type(&collect_to_resolved, expr.span)
            .unwrap_or(collect_to_resolved);
        let extended = args
            .iter()
            .any(|arg| matches!(arg, hir::common::Arg::Named(..)));
        let mut statements = Vec::new();
        let packet = if let Some(first) = pre_evaluated_first {
            let ty = if is_never(&first.ty) {
                p(P::Never)
            } else {
                self.tuple_value_type(vec![first.ty.clone()], expr.span)?
            };
            n::Expr {
                ty,
                source: source(expr.span),
                kind: K::Tuple(vec![first]),
            }
        } else if extended {
            let mut names = Vec::new();
            let mut named_values = Vec::new();
            for arg in args {
                match arg {
                    hir::common::Arg::Named(ident, value) => {
                        let checked = self.expr(value, None)?;
                        names.push((ident.name.to_string(), checked.ty.clone()));
                        named_values.push(checked);
                    }
                    _ => {
                        return Err(error(
                            expr.span,
                            "named indexing arguments require all entries to be named or positional-only",
                        ));
                    }
                }
            }
            let order: Vec<usize> = (0..named_values.len()).collect();
            self.finish_property_packet(expr.span, names, named_values, &order)?
        } else {
            let mut values = Vec::new();
            for arg in args {
                match arg {
                    hir::common::Arg::Positional(value) => values.push(self.expr(value, None)?),
                    hir::common::Arg::Expand(value) => {
                        let spread = self.expr(value, None)?;
                        let shape = self.structural_type(&spread.ty, expr.span)?;
                        if let Term::Tuple(elems) = shape {
                            if elems.is_empty() {
                                if is_never(&spread.ty) {
                                    values.push(spread);
                                }
                            } else {
                                let mut pattern_fields = Vec::with_capacity(elems.len());
                                let mut element_locals = Vec::with_capacity(elems.len());
                                for elem_ty in elems {
                                    let elem_binding = self.spread_binding(expr.span)?;
                                    self.locals.insert(elem_binding, elem_ty.clone());
                                    pattern_fields.push(n::Pattern::Binding {
                                        binding: elem_binding,
                                        inner: Box::new(n::Pattern::Wild),
                                    });
                                    element_locals.push((elem_binding, elem_ty));
                                }
                                statements.push(n::Expr {
                                    ty: p(P::Void),
                                    source: source(expr.span),
                                    kind: K::LetPattern {
                                        pattern: n::Pattern::Tuple(pattern_fields),
                                        value: Box::new(spread),
                                        guard: None,
                                        otherwise: None,
                                    },
                                });
                                for (elem_binding, elem_ty) in element_locals {
                                    values.push(n::Expr {
                                        ty: elem_ty,
                                        source: source(expr.span),
                                        kind: K::Local(elem_binding),
                                    });
                                }
                            }
                        } else {
                            return Err(error(
                                expr.span,
                                "spread indexing arguments require a Tuple",
                            ));
                        }
                    }
                    _ => {
                        return Err(error(
                            expr.span,
                            "unsupported argument form inside `[...]` indexing",
                        ));
                    }
                }
            }
            let ty = if values.iter().any(|v| is_never(&v.ty)) {
                p(P::Never)
            } else if values.is_empty() {
                p(P::Void)
            } else {
                self.tuple_value_type(values.iter().map(|v| v.ty.clone()).collect(), expr.span)?
            };
            n::Expr {
                ty,
                source: source(expr.span),
                kind: if values.is_empty() {
                    K::Unit
                } else {
                    K::Tuple(values)
                },
            }
        };
        if is_never(&packet.ty) {
            if let Some(expected) = expected {
                self.require_expression(&p(P::Never), expected, expr.span)?;
            }
            let unused = self.spread_binding(expr.span)?;
            let mut all_stmts = vec![n::Expr {
                source: receiver_ptr.source,
                ty: p(P::Void),
                kind: K::Let {
                    binding: unused,
                    value: Box::new(receiver_ptr),
                },
            }];
            all_stmts.extend(statements);
            return Ok(n::Expr {
                source: source(expr.span),
                ty: p(P::Never),
                kind: K::Block {
                    statements: all_stmts,
                    result: Some(Box::new(packet)),
                },
            });
        }
        let collected = self.lower_variadic_collector_packet(
            &mut statements,
            packet,
            &collect_to_ty,
            extended,
            false,
            expr.span,
        )?;
        let callee = self
            .selected_method(
                subject.clone(),
                Some(space_apply_iface),
                "space_apply",
                expr.span,
            )?
            .ok_or_else(|| {
                error(
                    expr.span,
                    format!("`space_apply` method on `{subject}` did not resolve"),
                )
            })?;
        let call = self.finish_call(
            expr,
            callee,
            &[],
            vec![receiver_ptr, collected],
            expected,
            None,
        )?;
        if statements.is_empty() {
            Ok(call)
        } else {
            let diverges = statements.iter().any(|s| is_never(&s.ty));
            Ok(n::Expr {
                ty: if diverges {
                    p(P::Never)
                } else {
                    call.ty.clone()
                },
                source: source(expr.span),
                kind: K::CallPreparation {
                    statements,
                    result: Some(Box::new(call)),
                },
            })
        }
    }
}

pub(super) enum IndexResolution {
    Selected(n::Expr),
    NoCandidate {
        receiver_ptr: n::Expr,
        arg_expr: n::Expr,
    },
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
        let deref_target = if matches!(target.kind, H::TakeBitVec(_)) {
            None
        } else {
            self.dereferenced_place_for_write(target)?
        };
        let mut cached_bit_pointer = None;
        if deref_target.is_none() {
            let mut storage = target;
            let mut indices = Vec::new();
            while let H::Index(base, args) = &storage.kind {
                let [hir::common::Arg::Positional(index)] = args else {
                    return Err(error(
                        target.span,
                        "compound assignment requires a writable place",
                    ));
                };
                indices.push(*index);
                storage = base;
                if matches!(storage.kind, H::TakeBitVec(_)) {
                    break;
                }
                if let Some(ptr) = self.dereferenced_place_for_write(storage)? {
                    cached_bit_pointer = Some(ptr);
                    break;
                }
            }
            indices.reverse();
            let take_bits = if let H::TakeBitVec(base) = &storage.kind {
                storage = base;
                true
            } else {
                false
            };
            if take_bits || !indices.is_empty() {
                let (operand, place) = if let Some(pointer) = cached_bit_pointer {
                    (pointer, None)
                } else if let Some(pointer) = self.dereferenced_place_for_write(storage)? {
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
        }
        let Some(pointer) = deref_target else {
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
                    "compound assignment requires a writable place",
                ));
            }
            let target_read = self.expr(target, None)?;
            let target_ty = target_read.ty.clone();
            let shape = self.structural_type(&target_ty, target.span)?;
            if !is_never(&target_ty)
                && !self.native_arithmetic_type(op, &shape)
                && let Some(compound_op) = crate::arithmetic::CompoundOperator::from_bin_op(*op)
            {
                let mut fields_empty = true;
                if let H::Projection(..) = &target.kind {
                    fields_empty = false;
                }
                if fields_empty
                    && let K::Local(root) = target_read.kind
                    && self.immutable.contains(&root)
                {
                    return Err(error(
                        target.span,
                        "cannot reassign a let binding; use var for compound assignment",
                    ));
                }
                if self.comptime_pointer_forbidden() {
                    return self.compound_protocol_or_fallback_comptime(
                        expression,
                        compound_op,
                        target,
                        target_read,
                        value,
                        expected,
                    );
                }
                if let Some(pointer) = self.local_storage_pointer(target, true)? {
                    return self.compound_protocol_or_fallback_pointer(
                        expression,
                        op,
                        compound_op,
                        target,
                        pointer,
                        value,
                        expected,
                    );
                }
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
        if is_never(&pointer.ty) {
            if let Some(expected) = expected {
                self.require_expression(&pointer.ty, expected, expression.span)?;
            }
            return Ok(pointer);
        }
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
        if field_idents.is_empty()
            && let Term::Pointer(pointee) = &pointer.ty
            && let Some(compound_op) = crate::arithmetic::CompoundOperator::from_bin_op(*op)
        {
            let shape = self.structural_type(pointee, target.span)?;
            if !is_never(pointee) && !self.native_arithmetic_type(op, &shape) {
                return self.compound_protocol_or_fallback_pointer(
                    expression,
                    op,
                    compound_op,
                    target,
                    pointer,
                    value,
                    expected,
                );
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

    fn compound_protocol_or_fallback_pointer(
        &mut self,
        expression: &Expr<'_>,
        _op: &BinOp,
        compound_op: crate::arithmetic::CompoundOperator,
        target: &Expr<'_>,
        pointer: n::Expr,
        value: &Expr<'_>,
        expected: Option<&Term>,
    ) -> Result<n::Expr, Error> {
        let Term::Pointer(pointee_box) = &pointer.ty else {
            unreachable!("compound assignment pointer target requires a pointer");
        };
        let pointee_ty = (**pointee_box).clone();
        let rhs_ctx =
            if let Some(assign_proto) = self.protocols.compound_arithmetic[compound_op as usize] {
                if let Some(ctx) =
                    self.protocol_argument_context(&pointee_ty, assign_proto, expression.span)?
                {
                    Some(ctx)
                } else if let Some(bin_proto) =
                    self.protocols.arithmetic[compound_op.binary_operator() as usize]
                {
                    self.protocol_argument_context(&pointee_ty, bin_proto, expression.span)?
                } else {
                    None
                }
            } else if let Some(bin_proto) =
                self.protocols.arithmetic[compound_op.binary_operator() as usize]
            {
                self.protocol_argument_context(&pointee_ty, bin_proto, expression.span)?
            } else {
                None
            };
        let rhs = self.expr(value, rhs_ctx.as_ref())?;
        if is_never(&rhs.ty) {
            if let Some(expected) = expected {
                self.require_expression(&p(P::Never), expected, expression.span)?;
            }
            let unused = self.spread_binding(target.span)?;
            let keep_target = n::Expr {
                source: pointer.source,
                ty: p(P::Void),
                kind: K::Let {
                    binding: unused,
                    value: Box::new(pointer),
                },
            };
            return Ok(n::Expr {
                source: source(expression.span),
                ty: p(P::Never),
                kind: K::Block {
                    statements: vec![keep_target],
                    result: Some(Box::new(rhs)),
                },
            });
        }
        if let Some(assign_proto) = self.protocols.compound_arithmetic[compound_op as usize] {
            let assign_iface = n::InterfaceInstance {
                declaration: assign_proto,
                arguments: vec![rhs.ty.clone()],
            };
            if self.query_single_trait_candidate(
                &pointee_ty,
                &assign_iface,
                compound_op.method(),
                expression.span,
            )? {
                let callee = self
                    .selected_method(
                        pointee_ty.clone(),
                        Some(assign_iface),
                        compound_op.method(),
                        expression.span,
                    )?
                    .ok_or_else(|| {
                        error(
                            expression.span,
                            format!(
                                "`{}` method on `{pointee_ty}` did not resolve",
                                compound_op.method()
                            ),
                        )
                    })?;
                return self.finish_call(
                    expression,
                    callee,
                    &[],
                    vec![pointer, rhs],
                    expected,
                    None,
                );
            }
        }
        // Fallback ONLY when compound-assign has NoApplicableCandidate:
        // requires Copy + matching binary operator (Output == Self).
        if matches!(
            self.type_copy_status(&pointee_ty, target.span)?,
            super::receiver_adaptation::CopyStatus::NotCopy
        ) {
            return Err(error(
                expression.span,
                format!(
                    "`{pointee_ty}` does not implement `{}` or `Copy` for compound assignment",
                    compound_op.method()
                ),
            ));
        }
        let bin_op = compound_op.binary_operator();
        let Some(bin_proto) = self.protocols.arithmetic[bin_op as usize] else {
            return Err(error(
                expression.span,
                format!(
                    "`{pointee_ty}` does not implement `{}` and no binary fallback protocol is bound",
                    compound_op.method()
                ),
            ));
        };
        let bin_iface = n::InterfaceInstance {
            declaration: bin_proto,
            arguments: vec![rhs.ty.clone()],
        };
        if !self.query_single_trait_candidate(
            &pointee_ty,
            &bin_iface,
            bin_op.method(),
            expression.span,
        )? {
            return Err(error(
                expression.span,
                format!(
                    "no `{}` or `{}` implementation applies for `{pointee_ty}`",
                    compound_op.method(),
                    bin_op.method()
                ),
            ));
        }
        let binding = self.spread_binding(target.span)?;
        let ptr_local = n::Expr {
            ty: pointer.ty.clone(),
            source: source(target.span),
            kind: K::Local(binding),
        };
        let prepare = n::Expr {
            ty: p(P::Void),
            source: source(target.span),
            kind: K::Let {
                binding,
                value: Box::new(pointer),
            },
        };
        let old = n::Expr {
            ty: pointee_ty.clone(),
            source: source(target.span),
            kind: K::ReadPointer(Box::new(ptr_local.clone())),
        };
        let computed = self.finish_binary_protocol_call(
            expression,
            bin_proto,
            bin_op.method(),
            old,
            rhs,
            Some(&pointee_ty),
        )?;
        let result_ty = if is_never(&computed.ty) {
            p(P::Never)
        } else {
            p(P::Void)
        };
        if let Some(expected) = expected {
            self.require_expression(&result_ty, expected, expression.span)?;
        }
        let write = n::Expr {
            ty: result_ty.clone(),
            source: source(expression.span),
            kind: K::WritePointer {
                pointer: Box::new(ptr_local),
                value: Box::new(computed),
            },
        };
        Ok(n::Expr {
            ty: result_ty,
            source: source(expression.span),
            kind: K::Block {
                statements: vec![prepare],
                result: Some(Box::new(write)),
            },
        })
    }

    fn compound_protocol_or_fallback_comptime(
        &mut self,
        expression: &Expr<'_>,
        compound_op: crate::arithmetic::CompoundOperator,
        target: &Expr<'_>,
        target_read: n::Expr,
        value: &Expr<'_>,
        expected: Option<&Term>,
    ) -> Result<n::Expr, Error> {
        let target_ty = target_read.ty.clone();
        let rhs_ctx = if let Some(assign_proto) =
            self.protocols.comptime_compound_arithmetic[compound_op as usize]
        {
            if let Some(ctx) =
                self.protocol_argument_context(&target_ty, assign_proto, expression.span)?
            {
                Some(ctx)
            } else if let Some(bin_proto) =
                self.protocols.arithmetic[compound_op.binary_operator() as usize]
            {
                self.protocol_argument_context(&target_ty, bin_proto, expression.span)?
            } else {
                None
            }
        } else if let Some(bin_proto) =
            self.protocols.arithmetic[compound_op.binary_operator() as usize]
        {
            self.protocol_argument_context(&target_ty, bin_proto, expression.span)?
        } else {
            None
        };
        let rhs = self.expr(value, rhs_ctx.as_ref())?;
        if is_never(&rhs.ty) {
            if let Some(expected) = expected {
                self.require_expression(&p(P::Never), expected, expression.span)?;
            }
            let unused = self.spread_binding(target.span)?;
            let keep_target = n::Expr {
                source: target_read.source,
                ty: p(P::Void),
                kind: K::Let {
                    binding: unused,
                    value: Box::new(target_read),
                },
            };
            return Ok(n::Expr {
                source: source(expression.span),
                ty: p(P::Never),
                kind: K::Block {
                    statements: vec![keep_target],
                    result: Some(Box::new(rhs)),
                },
            });
        }
        if let Some(assign_proto) =
            self.protocols.comptime_compound_arithmetic[compound_op as usize]
        {
            let assign_iface = n::InterfaceInstance {
                declaration: assign_proto,
                arguments: vec![rhs.ty.clone()],
            };
            if self.query_single_trait_candidate(
                &target_ty,
                &assign_iface,
                compound_op.method(),
                expression.span,
            )? {
                let callee = self
                    .selected_method(
                        target_ty.clone(),
                        Some(assign_iface),
                        compound_op.method(),
                        expression.span,
                    )?
                    .ok_or_else(|| {
                        error(
                            expression.span,
                            format!(
                                "`{}` method on `{target_ty}` did not resolve",
                                compound_op.method()
                            ),
                        )
                    })?;
                return self.finish_call(
                    expression,
                    callee,
                    &[],
                    vec![target_read, rhs],
                    expected,
                    None,
                );
            }
        }
        // Fallback ONLY on NoApplicableCandidate, requiring Copy + binary operator (Output == Self).
        if matches!(
            self.type_copy_status(&target_ty, target.span)?,
            super::receiver_adaptation::CopyStatus::NotCopy
        ) {
            return Err(error(
                expression.span,
                format!(
                    "`{target_ty}` does not implement comptime `{}` or `Copy` for compound assignment",
                    compound_op.method()
                ),
            ));
        }
        let bin_op = compound_op.binary_operator();
        let Some(bin_proto) = self.protocols.arithmetic[bin_op as usize] else {
            return Err(error(
                expression.span,
                format!(
                    "`{target_ty}` does not implement comptime `{}` and no binary fallback protocol is bound",
                    compound_op.method()
                ),
            ));
        };
        let bin_iface = n::InterfaceInstance {
            declaration: bin_proto,
            arguments: vec![rhs.ty.clone()],
        };
        if !self.query_single_trait_candidate(
            &target_ty,
            &bin_iface,
            bin_op.method(),
            expression.span,
        )? {
            return Err(error(
                expression.span,
                format!(
                    "no `{}` or `{}` implementation applies for `{target_ty}`",
                    compound_op.method(),
                    bin_op.method()
                ),
            ));
        }
        let binary = Expr {
            kind: H::Binary(
                match compound_op {
                    crate::arithmetic::CompoundOperator::AddAssign => BinOp::Add,
                    crate::arithmetic::CompoundOperator::SubAssign => BinOp::Sub,
                    crate::arithmetic::CompoundOperator::MulAssign => BinOp::Mul,
                    crate::arithmetic::CompoundOperator::DivAssign => BinOp::Div,
                    crate::arithmetic::CompoundOperator::RemAssign => BinOp::Rem,
                },
                target,
                value,
            ),
            ..expression.clone()
        };
        let assign = Expr {
            kind: H::Assign(target, &binary),
            ..expression.clone()
        };
        self.expr(&assign, expected)
    }
}
