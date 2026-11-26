//! Operation-local state for program facts. Recursive children re-enter the
//! common checker, preserving budgets, failure locations and evidence hooks.
//! Separate frames keep unrelated operation temporaries off the active path.
use super::*;

impl Walker<'_> {
    fn known_associated_value(
        &mut self,
        interface: &nir::program::InterfaceInstance,
        member: nir::DeclarationId,
        subject: &Term,
    ) -> Result<Option<nir::completed_values::CompletedValue>, Reason> {
        let Some(package) = self.package else {
            return Ok(None);
        };
        let mut budget = crate::ptc::Budget {
            steps: self.limits.arithmetic.steps,
            ..Default::default()
        };
        let result = (|| {
            let world = match &mut self.associated_world {
                Some(world) => world,
                slot => slot.insert(
                    self.declaration_source
                        .unwrap_or_else(|| {
                            type_resolution::dispatch::DeclarationSource::Borrowed(package)
                        })
                        .world(&mut budget, None)?,
                ),
            };
            world
                .capabilities()
                .known_associated_value(interface, member, subject, &mut budget)
        })();
        self.limits.arithmetic.steps = budget.steps;
        result.map_err(|failure| match failure {
            nir::NominalAdmissionError::Budget => {
                Reason::Unknown(comptime_runtime::Error::BudgetExhausted.into())
            }
            _ => Reason::InvalidSignature,
        })
    }

    #[inline(never)]
    pub(super) fn operation_expression(
        &mut self,
        expr: &Expr,
        state: &mut State,
        depth: usize,
    ) -> Result<Value, Reason> {
        let mut result = Value::default();
        match &expr.kind {
            K::Intrinsic { inputs, .. } => {
                result = self.intrinsic_value(expr, inputs, state, depth)?;
            }
            K::BitWindow { value, start, end } => {
                self.bit_window(expr, value, start, end, state, depth)?;
            }
            K::BitIndex(value, index) => self.bit_index(expr, value, index, state, depth)?,
            // IEEE arithmetic supplies no exact integer/polynomial evidence.
            // Still visit operands in source order: their calls, writes and
            // machine arithmetic remain verification obligations.
            K::FloatNegate(value)
            | K::BitSlice { value, .. }
            | K::TakeBits(value)
            | K::TextQuery { subject: value, .. }
            | K::IntegerRepresentation(value)
            | K::TryIntegerConvert(value)
            | K::ComptimeIntegerConvert(value)
            | K::SymbolName(value)
            | K::EnumName(value) => {
                self.child(value, state, depth)?;
                result = self.unknown_value(&expr.ty)?;
            }
            K::BitConvert(value) => {
                self.child(value, state, depth)?;
                result = self.unknown_value(&expr.ty)?;
                // Unsigned reconstruction zero-extends the source bits. Its
                // upper bound follows from their width, not the wider target
                // domain. Do not transfer this rule to signed reinterpretation
                // or to nominal types with independent representation duties.
                if let (Term::Primitive(nir::Primitive::BitVec(width)), Term::Primitive(target)) =
                    (&value.ty, &expr.ty)
                {
                    if self
                        .integer_layout(*target)
                        .is_some_and(|(signed, bits)| !signed && *width <= bits as usize)
                    {
                        result.range = Some(if *width == 0 {
                            Interval::new(Integer::zero(), Integer::zero(), self.limits)?
                        } else {
                            Interval::machine(*width as u16, false, self.limits)?
                        });
                    }
                }
            }
            K::FloatBinary { left, right, .. }
            | K::BitEqual(left, right)
            | K::BitConcat(left, right) => {
                self.child(left, state, depth)?;
                self.child(right, state, depth)?;
                result = self.unknown_value(&expr.ty)?;
            }
            K::TypeListIndex(values, index) if matches!(expr.ty, Term::Primitive(primitive) if primitive != nir::Primitive::Object) =>
            {
                // Primitive elements carry no mutable storage aliases. Keep
                // their domain bounds, without assuming a particular element
                // value or transferring facts from another list observation.
                // Operands execute in order; checked indexing remains in the VM.
                // Never transports no element: returning from either operand
                // prevents the later operand and indexing from executing.
                // Objects and structured domains need their own alias transfer.
                self.child(values, state, depth)?;
                self.child(index, state, depth)?;
                result = self.unknown_value(&expr.ty)?;
            }
            K::ComptimeFunctionInstance { inputs, .. } => {
                for input in inputs.expressions() {
                    self.child(input, state, depth)?;
                }
                result = self.unknown_value(&expr.ty)?;
            }
            K::NewtypeRepresentation(subject)
            | K::DefinitionAttributes(subject)
            | K::StructFieldTypes(subject)
            | K::StructFieldNames(subject)
            | K::RecordFieldTypes(subject)
            | K::RecordFieldNames(subject)
            | K::ReflectCallable(subject)
            | K::TypeId(subject)
            | K::TupleLen(subject)
            | K::TupleElem(subject)
            | K::CompileError(subject)
            | K::EmitDiagnostic(subject)
            | K::BoundedStackBytes(subject)
            | K::BuildStruct(subject)
            | K::BuildNewtype(subject)
            | K::BuildEnum(subject)
            | K::ObjectQuery { subject, .. }
            | K::ObjectEdit { subject, .. }
            | K::EnumStructure { subject, .. }
            | K::ImplicitQuery {
                operand: subject, ..
            } => {
                self.child(subject, state, depth)?;
                result = self.unknown_value(&expr.ty)?;
            }
            _ => return Err(Reason::Unsupported),
        }
        Ok(result)
    }

    #[inline(never)]
    pub(super) fn pointer_read_expression(
        &mut self,
        expr: &Expr,
        state: &mut State,
        depth: usize,
    ) -> Result<Value, Reason> {
        let mut result = Value::default();
        match &expr.kind {
            K::AddressOf(_) => {
                result.pointer_target =
                    direct_pointer_place(expr, &mut self.limits.arithmetic.steps)
                        .map_err(|_| {
                            Reason::Unknown(comptime_runtime::Error::BudgetExhausted.into())
                        })?
                        .map(|target| (state.frame, vec![target]));
            }
            K::ConstAddress { .. } | K::StaticAddress(_) => {}
            K::PointerField {
                pointer,
                declaration,
                index,
            } => {
                let input = self.child(pointer, state, depth)?;
                let origins = self.pointer_value_origins(&input, state)?;
                if let Some((frame, targets)) = input.pointer_target {
                    let mut projected = Vec::new();
                    for mut target in targets {
                        self.limits.step(1)?;
                        target
                            .fields
                            .push((nir::program::FieldOwner::Nominal(*declaration), *index));
                        insert_pointer_target(
                            &mut projected,
                            target,
                            &mut self.limits.arithmetic.steps,
                        )?;
                    }
                    result.pointer_target = Some((frame, projected));
                } else if let Some(mut targets) = origins {
                    for target in &mut targets {
                        self.limits.step(1)?;
                        target
                            .project(&[(nir::program::FieldOwner::Nominal(*declaration), *index)]);
                    }
                    result.pointer_origins = Some((state.frame, targets));
                }
            }
            K::PointerAnonymousField { pointer, index, .. } => {
                let input = self.child(pointer, state, depth)?;
                let origins = self.pointer_value_origins(&input, state)?;
                if let Some((frame, targets)) = input.pointer_target {
                    let mut projected = Vec::new();
                    for mut target in targets {
                        self.limits.step(1)?;
                        target
                            .fields
                            .push((nir::program::FieldOwner::Anonymous, *index));
                        insert_pointer_target(
                            &mut projected,
                            target,
                            &mut self.limits.arithmetic.steps,
                        )?;
                    }
                    result.pointer_target = Some((frame, projected));
                } else if let Some(mut targets) = origins {
                    for target in &mut targets {
                        self.limits.step(1)?;
                        target.project(&[(nir::program::FieldOwner::Anonymous, *index)]);
                    }
                    result.pointer_origins = Some((state.frame, targets));
                }
            }
            K::ReadPointer(pointer) => {
                let pointer_value = self.child(pointer, state, depth)?;
                let origins = self.pointer_value_origins(&pointer_value, state)?;
                let targets = pointer_value
                    .pointer_target
                    .filter(|(frame, _)| *frame == state.frame)
                    .map(|(_, targets)| targets);
                result = if let Some(targets) = targets {
                    let mut joined: Option<Value> = None;
                    for target in targets {
                        let current = self.read_pointer_place(state, &target)?;
                        joined = Some(match joined {
                            Some(previous) => previous.join(&current, self.limits, 0)?,
                            None => current,
                        });
                    }
                    match joined {
                        Some(value) => value,
                        None => self.unknown_value(&expr.ty)?,
                    }
                } else if let Some(addresses) = origins {
                    let mut joined: Option<Value> = None;
                    for address in addresses {
                        let current = self.read_pointer_origin(address, &expr.ty, state)?;
                        joined = Some(match joined {
                            Some(previous) => previous.join(&current, self.limits, 0)?,
                            None => current,
                        });
                    }
                    match joined {
                        Some(value) => value,
                        None => self.unknown_value(&expr.ty)?,
                    }
                } else {
                    self.observe_external_read(pointer, &expr.ty, state)?
                };
            }
            _ => return Err(Reason::Unsupported),
        }
        Ok(result)
    }

    #[inline(never)]
    pub(super) fn write_expression(
        &mut self,
        expr: &Expr,
        state: &mut State,
        depth: usize,
    ) -> Result<Value, Reason> {
        let result = Value::default();
        match &expr.kind {
            K::BitWritePointer {
                pointer,
                selections,
                value,
            } => {
                let Term::Pointer(storage) = &pointer.ty else {
                    return Err(Reason::Unsupported);
                };
                let targets = self
                    .child(pointer, state, depth)?
                    .pointer_target
                    .filter(|(frame, _)| *frame == state.frame)
                    .map(|(_, targets)| targets);
                self.bit_selections(expr, storage, selections, state, depth)?;
                self.child(value, state, depth)?;
                if state.facts.is_reachable() {
                    if let Some(targets) = targets {
                        for target in targets {
                            let unknown = self.unknown_value(storage)?;
                            if target.is_root() {
                                self.bind_related_value(state, target.root, storage, unknown)?;
                            } else {
                                self.write_field(state, &target, unknown)?;
                            }
                        }
                    } else {
                        state.havoc(self.limits)?;
                    }
                }
            }
            K::WritePointer { pointer, value } => {
                let targets = self
                    .child(pointer, state, depth)?
                    .pointer_target
                    .filter(|(frame, _)| *frame == state.frame)
                    .map(|(_, targets)| targets);
                let address = self
                    .aliased_read_path(pointer, state)?
                    .and_then(|path| state.facts.value(path.root).map(|version| (path, version)));
                let mut replacement = self.child(value, state, depth)?;
                if state.facts.is_reachable() {
                    if let Some(targets) = targets {
                        state.forget_reads(self.limits)?;
                        let multiple = targets.len() > 1;
                        let mut updates = Vec::new();
                        for target in targets {
                            let changed = if multiple {
                                // Each candidate may remain untouched. Merge
                                // old/new values, never assign the RHS to all.
                                self.read_pointer_place(state, &target)?.join(
                                    &replacement,
                                    self.limits,
                                    0,
                                )?
                            } else {
                                std::mem::take(&mut replacement)
                            };
                            updates.push((target, changed));
                        }
                        for (target, changed) in updates {
                            if target.is_root() {
                                self.bind_related_value(state, target.root, &value.ty, changed)?;
                            } else {
                                self.write_field(state, &target, changed)?;
                            }
                        }
                    } else {
                        // The address was evaluated before the RHS. Only attach
                        // the new observation to its source path if that binding
                        // still names the captured address after RHS effects.
                        let view = self.scalar_type_view(&value.ty)?;
                        let observed = if let Some((path, version)) = &address {
                            if state.facts.value(path.root) == Some(*version)
                                && matches!(view, Term::Primitive(p) if p == nir::Primitive::Bool || self.integer_layout(p).is_some())
                            {
                                let transfer = self.prepare_copy(&replacement, state)?;
                                self.limits.step(path.fields.len().saturating_add(1))?;
                                Some((path.clone(), transfer))
                            } else {
                                None
                            }
                        } else {
                            None
                        };
                        self.havoc_after_store(pointer, &view, address.as_ref(), state)?;
                        if let Some((path, transfer)) = observed {
                            let root = self.result_root(&view)?.ok_or(Reason::Unsupported)?;
                            bind_value_place(state, root, &view, replacement, self.limits)?;
                            if let Some(transfer) = transfer {
                                let copies = self.install_copy_at(state, root, &[], transfer)?;
                                self.finish_copies(state, copies)?;
                            }
                            state.reads.insert(path, (view, root));
                        }
                    }
                }
            }
            K::Assign {
                place: target,
                value,
            } => {
                let value_result = self.child(value, state, depth)?;
                if state.facts.is_reachable() {
                    if target.is_root() {
                        self.bind_related_value(state, target.root, &value.ty, value_result)?;
                    } else {
                        self.write_field(state, target, value_result)?;
                    }
                }
            }
            K::BitAssign {
                place: target,
                storage_type,
                value,
                ..
            }
            | K::BitUpdate {
                place: target,
                storage_type,
                value,
                ..
            } => {
                if let K::BitUpdate { selections, .. } = &expr.kind {
                    self.bit_selections(expr, storage_type, selections, state, depth)?;
                }
                self.child(value, state, depth)?;
                if state.facts.is_reachable() {
                    // The slice write preserves the storage type, but not its
                    // previous numeric range or relations to other places.
                    let unknown = self.unknown_value(storage_type)?;
                    if target.is_root() {
                        bind_value(state, target.root, storage_type, unknown, self.limits)?;
                    } else {
                        self.write_field(state, target, unknown)?;
                    }
                }
            }
            _ => return Err(Reason::Unsupported),
        }
        Ok(result)
    }

    #[inline(never)]
    pub(super) fn transparent_expression(
        &mut self,
        expr: &Expr,
        state: &mut State,
        depth: usize,
    ) -> Result<Value, Reason> {
        let mut result = Value::default();
        match &expr.kind {
            K::Closure { .. } => {
                result = self.closure_expression(expr, state, depth)?;
            }
            K::UnsafeScope(value) => {
                result = self.child(value, state, depth)?;
            }
            K::NewtypeConvert(value) => {
                result = self.child(value, state, depth)?;
                result.nominal_type = matches!(
                    expr.ty,
                    Term::Nominal { .. } | Term::Tuple(_) | Term::Record(_)
                )
                .then(|| std::sync::Arc::new(expr.ty.clone()));
            }
            K::Discard(value) => {
                self.child(value, state, depth)?;
            }
            _ => return Err(Reason::Unsupported),
        }
        Ok(result)
    }

    #[inline(never)]
    pub(super) fn formation_expression(
        &mut self,
        expr: &Expr,
        state: &mut State,
        depth: usize,
    ) -> Result<Value, Reason> {
        let result = Value::default();
        match &expr.kind {
            K::PlannedFunctionInstance { .. }
            | K::ApplyNormalForm { .. }
            | K::NormalFormType { .. }
            | K::NominalType { .. }
            | K::WithTypeInputs { .. }
            | K::CheckTypeAlias { .. } => {
                let public = if let K::ApplyNormalForm {
                    callee, input_ids, ..
                } = &expr.kind
                {
                    self.child(callee, state, depth)?;
                    if !state.facts.is_reachable() {
                        return Ok(result);
                    }
                    Some(
                        nir::normal_forms::open_inputs(
                            &callee.ty,
                            input_ids,
                            &mut self.limits.arithmetic.steps,
                        )
                        .map_err(declaration_failure)?,
                    )
                } else {
                    None
                };
                let (arguments, schedule) = expr.kind.declaration_inputs().unwrap();
                self.limits.step(arguments.len())?;
                let schedule = schedule
                    .check(arguments.len(), |n| self.limits.step(n))?
                    .ok_or(Reason::Unsupported)?;
                let metadata = match &expr.kind {
                    K::NormalFormType { head, .. } => self.package.and_then(|package| {
                        package
                            .normal_forms
                            .iter()
                            .find(|item| item.declaration == *head)
                    }),
                    _ => None,
                };
                for step in schedule.steps() {
                    if !state.facts.is_reachable() {
                        break;
                    }
                    match step {
                        nir::program::InputStep::Prepare(expr) => {
                            self.child(expr, state, depth)?;
                        }
                        nir::program::InputStep::Supply(slot) => {
                            let domain = public
                                .as_ref()
                                .and_then(|inputs| inputs.get(slot))
                                .map(|(_, _, ty)| ty)
                                .or_else(|| {
                                    metadata
                                        .and_then(|head| head.parameters.get(slot))
                                        .map(|p| &p.ty)
                                });
                            let (value, ty) =
                                self.normal_input_value(&arguments[slot], domain, state, depth)?;
                            if let Some(binding) = schedule.binding(slot)
                                && state.facts.is_reachable()
                            {
                                self.bind_related_value(state, binding, &ty, value)?;
                            }
                        }
                    }
                }
                if let K::CheckTypeAlias {
                    body: Some(body), ..
                }
                | K::WithTypeInputs { body, .. } = &expr.kind
                    && state.facts.is_reachable()
                {
                    self.child(body, state, depth)?;
                }
                if matches!(expr.kind, K::PlannedFunctionInstance { .. }) {
                    return self.unknown_value(&expr.ty);
                }
            }
            K::TypeList(arguments)
            | K::FunctionType {
                parameters: arguments,
                ..
            } => {
                // Formation predicates belong to the type checker/invocation,
                // but computing their operands is ordinary program evaluation.
                // Preserve operand order, effects and early exits here without
                // importing type propositions into the program facts.
                for argument in arguments {
                    self.child(argument, state, depth)?;
                }
            }
            K::OptionalType(value) | K::PointerType(value) | K::BitVecType(value) => {
                self.child(value, state, depth)?;
            }
            K::FunctionResultType {
                signature,
                result: output,
            } => {
                self.child(signature, state, depth)?;
                self.child(output, state, depth)?;
            }
            K::ClosureType {
                environment,
                signature,
            } => {
                for field in environment {
                    self.child(field, state, depth)?;
                }
                self.child(signature, state, depth)?;
            }
            K::TraitObjectType { arguments, .. } => {
                for arg in arguments {
                    self.child(arg, state, depth)?;
                }
            }
            K::DynCast(cast) => {
                self.child(&cast.value, state, depth)?;
                for arg in &cast.env_arguments {
                    self.child(arg, state, depth)?;
                }
                return self.unknown_value(&expr.ty);
            }
            K::DynCall(call) => {
                self.child(&call.receiver, state, depth)?;
                for arg in &call.arguments {
                    self.child(&arg.value, state, depth)?;
                }
                if state.facts.is_reachable() {
                    state.havoc(self.limits)?;
                }
                return self.unknown_value(&expr.ty);
            }
            K::TypeEqual(left, right) => {
                self.child(left, state, depth)?;
                self.child(right, state, depth)?;
                // This layer tracks the evaluation, not type-theory equality.
                // Its Bool result remains unknown without program evidence.
            }
            K::TypeKind { operand, .. } | K::TypeNotSized { operand } => {
                self.child(operand, state, depth)?;
            }
            K::TypeLayout { operand, .. } => {
                self.child(operand, state, depth)?;
                return self.unknown_value(&expr.ty);
            }
            K::TypeHasField { subject, field }
            | K::TypeFieldType { subject, field }
            | K::StructFieldType { subject, field }
            | K::FieldValue {
                value: subject,
                field,
            }
            | K::TupleAppend {
                tuple: subject,
                element: field,
            } => {
                self.child(subject, state, depth)?;
                self.child(field, state, depth)?;
                return self.unknown_value(&expr.ty);
            }
            K::NamedField { value: operand, .. }
            | K::ConstructStruct { fields: operand }
            | K::EnumVariantIndex(operand)
            | K::StructFieldSpans(operand)
            | K::RecordFieldSpans(operand)
            | K::DeclarationSpan(operand)
            | K::EnumVariantSpans(operand)
            | K::EnumVariantFieldSpans(operand)
            | K::ComptimeEnumVariantIndex(operand)
            | K::ComptimeEnumVariantFieldValues(operand) => {
                self.child(operand, state, depth)?;
                return self.unknown_value(&expr.ty);
            }
            K::ConstructEnum { variant, fields } => {
                self.child(variant, state, depth)?;
                self.child(fields, state, depth)?;
                return self.unknown_value(&expr.ty);
            }
            K::FieldPtr { pointer, field } => {
                self.child(pointer, state, depth)?;
                self.child(field, state, depth)?;
                return self.unknown_value(&expr.ty);
            }
            K::EnumVariantFieldPtr {
                pointer,
                variant,
                field,
            }
            | K::ComptimeFieldEntry {
                value: pointer,
                field: variant,
                ty: field,
            } => {
                self.child(pointer, state, depth)?;
                self.child(variant, state, depth)?;
                self.child(field, state, depth)?;
                return self.unknown_value(&expr.ty);
            }
            _ => return Err(Reason::Unsupported),
        }
        Ok(result)
    }

    #[inline(never)]
    pub(super) fn aggregate_expression(
        &mut self,
        expr: &Expr,
        state: &mut State,
        depth: usize,
    ) -> Result<Value, Reason> {
        let mut result = Value::default();
        match &expr.kind {
            K::ErrorConvert { conversion, value } => {
                result = self.error_conversion(expr, *conversion, value, state, depth)?;
            }
            K::OptionalAbsent | K::OptionalPresent(_) => {
                let Term::Optional(payload_type) = &expr.ty else {
                    return Err(Reason::InvalidSignature);
                };
                let mut fields = Vec::new();
                if let K::OptionalPresent(payload) = &expr.kind {
                    if payload.ty != **payload_type {
                        return Err(Reason::InvalidSignature);
                    }
                    let mut field = self.child(payload, state, depth)?;
                    if matches!(payload.ty, Term::Pointer(_)) && field.pointer_target.is_none() {
                        field.pointer_origins = self
                            .pointer_value_origins(&field, state)?
                            .map(|targets| (state.frame, targets));
                    }
                    field.evidence = self.prepare_copy(&field, state)?;
                    fields.push(field.snapshot(self.limits, 0)?);
                }
                result.fields = Some(fields);
                result.nominal_type = Some(std::sync::Arc::new(expr.ty.clone()));
                result.aggregate_kind = Some(AggregateKind::Optional(matches!(
                    expr.kind,
                    K::OptionalPresent(_)
                )));
                result = self.materialize_aggregate(result, &expr.ty, state)?;
            }
            K::Object { fields, .. } => {
                // Metadata retains no scalar proof here. Its producers still
                // run in order and must satisfy their own program contracts.
                for field in fields {
                    self.child(field, state, depth)?;
                }
            }
            K::Tuple(values)
            | K::Array(values)
            | K::AnonymousRecord { fields: values }
            | K::Record { fields: values, .. }
            | K::Variant { fields: values, .. } => {
                let mut fields = Vec::new();
                for value in values {
                    let mut evaluated = self.child(value, state, depth)?;
                    if matches!(value.ty, Term::Pointer(_)) && evaluated.pointer_target.is_none() {
                        evaluated.pointer_origins = self
                            .pointer_value_origins(&evaluated, state)?
                            .map(|targets| (state.frame, targets));
                    }
                    evaluated.evidence = self.prepare_copy(&evaluated, state)?;
                    fields.push(evaluated.snapshot(self.limits, 0)?);
                }
                result.fields = Some(fields);
                result.nominal_type = if matches!(expr.kind, K::Tuple(_) | K::Array(_))
                    || matches!(expr.ty, Term::Nominal { .. } | Term::Record(_))
                {
                    Some(std::sync::Arc::new(expr.ty.clone()))
                } else {
                    None
                };
                result.aggregate_kind = Some(match expr.kind {
                    K::Tuple(..) | K::Array(..) => AggregateKind::Tuple,
                    K::AnonymousRecord { .. } => AggregateKind::AnonymousRecord,
                    K::Record { declaration, .. } => AggregateKind::Record(declaration),
                    K::Variant {
                        declaration, index, ..
                    } => AggregateKind::Variant(declaration, index),
                    _ => unreachable!(),
                });
                result = self.materialize_aggregate(result, &expr.ty, state)?;
            }
            K::AnonymousField { value, index, .. } => {
                let base = self.child(value, state, depth)?;
                result = self.read_anonymous_field(base, &value.ty, *index, state)?;
            }
            K::AssociatedValue {
                interface,
                member,
                subject,
            } => {
                result = self.unknown_value(&expr.ty)?;
                let view = self.scalar_type_view(&expr.ty)?;
                self.limits.step(self.associated_scalars.len())?;
                let existing = self.associated_scalars.iter().find(
                    |((stored_interface, stored_member, stored_subject), _, _)| {
                        stored_interface == interface
                            && stored_member == member
                            && stored_subject == subject
                    },
                );
                let root = if let Some((_, domain, root)) = existing {
                    if domain != &view {
                        return Err(Reason::InvalidSignature);
                    }
                    Some(*root)
                } else if let Some(root) = self.result_root(&view)? {
                    for ty in std::iter::once(subject)
                        .chain(&interface.arguments)
                        .chain(std::iter::once(&view))
                    {
                        nir::type_computations::charge_term(ty, &mut self.limits.arithmetic.steps)
                            .map_err(|_| {
                                Reason::Unknown(comptime_runtime::Error::BudgetExhausted.into())
                            })?;
                    }
                    self.associated_scalars.push((
                        (interface.clone(), *member, subject.clone()),
                        view.clone(),
                        root,
                    ));
                    Some(root)
                } else {
                    None
                };
                if let Some(root) = root {
                    // Associated constants retain identity across observations
                    // and call frames, without assuming an initializer result.
                    if state.facts.value(root).is_none() {
                        // The closed declaration world can supply a checked literal.
                        // An open receiver or an executable initializer contributes
                        // only the stable identity of this scalar observation.
                        let known = self.known_associated_value(interface, *member, subject)?;
                        if let Some(known) = known {
                            let value = known.expression();
                            if value.ty != view {
                                return Err(Reason::InvalidSignature);
                            }
                            if matches!(value.kind, K::Literal(_)) {
                                result = self.child(value, state, depth)?;
                            }
                        }
                        bind_value_place(state, root, &view, result, self.limits)?;
                    }
                    // This is an immutable declaration value, not caller
                    // storage that an opaque call may overwrite.
                    state.frozen.insert(root, view.clone());
                    result = self.local_value_at(state, root)?;
                }
            }
            K::Field {
                value,
                declaration,
                index,
            } => {
                let base = self.child(value, state, depth)?;
                result = self.read_record_field(base, &value.ty, *declaration, *index, state)?;
            }
            _ => return Err(Reason::Unsupported),
        }
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nir::Primitive;

    fn nested_write(negate: bool) -> Function {
        let boolean = Term::Primitive(Primitive::Bool);
        let void = Term::Primitive(Primitive::Void);
        let local = LocalId { owner: 1, index: 0 };
        let expr = |kind, ty| Expr {
            source: SourceRange { start: 10, end: 20 },
            ty,
            kind,
        };
        let pointer = || {
            expr(
                K::AddressOf(Box::new(expr(K::Local(local), boolean.clone()))),
                Term::Pointer(Box::new(boolean.clone())),
            )
        };
        let mut value = expr(
            K::Block {
                statements: vec![expr(
                    K::WritePointer {
                        pointer: Box::new(pointer()),
                        value: Box::new(expr(K::Literal(Literal::Bool(true)), boolean.clone())),
                    },
                    void.clone(),
                )],
                result: Some(Box::new(expr(
                    K::ReadPointer(Box::new(pointer())),
                    boolean.clone(),
                ))),
            },
            boolean.clone(),
        );
        // Generated wrappers must preserve the inner write and its observation
        // without reserving every other operation's scratch state per level.
        for _ in 0..40 {
            value = expr(K::UnsafeScope(Box::new(value)), boolean.clone());
        }
        if negate {
            value = expr(K::Not(Box::new(value)), boolean.clone());
        }
        let mut predicate = expr(K::EnsurePredicate(Box::new(value)), void.clone());
        predicate.source = SourceRange { start: 30, end: 40 };
        Function {
            interface: nir::program::FunctionDeclaration {
                source: nir::program::SourceRange { start: 0, end: 0 },
                declaration: nir::DeclarationId {
                    package: 0,
                    local: 1,
                },
                name: "nested_write".into(),
                phase: nir::program::FunctionPhase::Runtime,
                inline: false,
                pure: false,
                is_unsafe: false,
                type_parameters: vec![],
                parameters: vec![],
                env_parameters: vec![],
                defaults: vec![],
                requirements: vec![],
                capabilities: vec![],
                preconditions: vec![],
                postconditions: vec![],
                entry_snapshots: vec![],
                result_binding: None,
                result_type_binder: None,
                result_guarantees: vec![],
                signature: Term::Function {
                    parameters: vec![],
                    result: Box::new(void.clone()),
                },
            },
            body: expr(
                K::Block {
                    statements: vec![
                        expr(
                            K::Let {
                                binding: local,
                                value: Box::new(expr(K::Literal(Literal::Bool(false)), boolean)),
                            },
                            void.clone(),
                        ),
                        predicate,
                    ],
                    result: None,
                },
                void,
            ),
            runtime_deliveries: Default::default(),
        }
    }

    #[test]
    fn nested_operations_preserve_writes_proofs_and_budget_recovery() {
        // Match the usual test-thread stack explicitly, independent of a
        // developer's RUST_MIN_STACK setting. This is not an enlarged stack.
        std::thread::Builder::new()
            .stack_size(2 * 1024 * 1024)
            .spawn(|| {
                let function = nested_write(false);
                let mut short = Limits::default();
                short.arithmetic.steps = 20;
                let failure = analyze(&function, &mut short).err().unwrap();
                assert_eq!(
                    failure.reason,
                    Reason::Unknown(comptime_runtime::Error::BudgetExhausted.into())
                );
                let report = analyze(&function, &mut Limits::default()).unwrap();
                assert!(report.exit.is_reachable());
                let failure = analyze(&nested_write(true), &mut Limits::default())
                    .err()
                    .unwrap();
                assert_eq!(failure.reason, Reason::MissingPostconditionProof);
                assert_eq!(failure.source, SourceRange { start: 30, end: 40 });
                analyze(&function, &mut Limits::default()).unwrap();
            })
            .unwrap()
            .join()
            .unwrap();
    }
}
