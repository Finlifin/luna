//! Immutable list operations retain identity, evaluation order and VM limits.
use super::*;

impl Interpreter<'_> {
    /// Fuse a self-concatenating list write when the suffix cannot execute code.
    /// Keep general expressions on the ordinary snapshot path: a callback may
    /// replace the target between the left read and the eventual write.
    pub(super) fn eval_list_append(
        &mut self,
        expr: &Expr,
        environment: &mut Environment,
    ) -> Option<Result<Flow, Error>> {
        if !self.comptime_only || self.cleanup_schedule.is_some() {
            return None;
        }
        let K::WritePointer { pointer, value } = &expr.kind else {
            return None;
        };
        let K::Local(destination) = pointer.kind else {
            return None;
        };
        let K::ListConcat(left, right) = &value.kind else {
            return None;
        };
        let K::ReadPointer(input) = &left.kind else {
            return None;
        };
        if !matches!(input.kind, K::Local(binding) if binding == destination)
            || input.ty != pointer.ty
            || left.ty != value.ty
            || right.ty != value.ty
            || expr.ty != Term::Primitive(Primitive::Void)
            || (self.list_element(&value.ty).is_none() && !self.is_type_list_input(&value.ty))
        {
            return None;
        }
        let simple = match &right.kind {
            K::TypeList(fields) => fields.iter().all(|field| {
                matches!(
                    field.kind,
                    K::Local(_) | K::TypeValue(_) | K::ComptimeValue(_) | K::Literal(_)
                )
            }),
            K::ComptimeValue(_) => true,
            _ => false,
        };
        if !simple {
            return None;
        }
        Some((|| {
            let pointer = match self.eval(pointer, environment)? {
                Flow::Value(Value::Pointer(pointer)) => pointer,
                Flow::Value(_) => return Err(self.failure(ErrorKind::Type, Some(expr.source))),
                flow => return Ok(flow),
            };
            if pointer.pointee != value.ty {
                return Err(self.failure(ErrorKind::Type, Some(expr.source)));
            }
            self.tick(Some(value.source))?;
            self.tick(Some(left.source))?;
            self.tick(Some(input.source))?;
            pointer
                .inspect(|old| self.validate_immutable_storage(old, &value.ty, expr.source))
                .map_err(|kind| self.failure(kind, Some(expr.source)))??;
            let suffix = match self.eval(right, environment)? {
                Flow::Value(value) => value,
                flow => return Ok(flow),
            };
            self.validate_immutable_storage(&suffix, &value.ty, expr.source)?;
            pointer
                .append_list_checked(suffix, |combined| {
                    let work = combined
                        .scoped_copy_work_with_evidence(
                            &self.abstract_type_parameters,
                            &self.abstract_value_parameters,
                        )
                        .ok_or(ErrorKind::RepresentationSizeLimit)?;
                    self.integer_operation(Some(expr.source), |budget| budget.consume(work))
                        .map_err(|error| error.kind)?;
                    self.validate_immutable_input(combined, &value.ty)
                        .map_err(|error| error.kind)
                })
                .map_err(|kind| self.failure(kind, Some(expr.source)))?;
            Ok(Flow::Value(Value::Unit))
        })())
    }

    /// A method receiver may refer to a projected caller field. Only fuse a
    /// local receiver read and a suffix that cannot execute user code, so no
    /// callback can invalidate the snapshot semantics of concatenation.
    pub(super) fn eval_reference_list_append(
        &mut self,
        expr: &Expr,
        environment: &mut Environment,
    ) -> Option<Result<Flow, Error>> {
        if !self.comptime_only || self.cleanup_schedule.is_some() {
            return None;
        }
        let K::Assign { place, value } = &expr.kind else {
            return None;
        };
        let K::ListConcat(left, right) = &value.kind else {
            return None;
        };
        if !place.fields.is_empty()
            || !matches!(left.kind, K::Local(binding) if binding == place.root)
            || left.ty != value.ty
            || right.ty != value.ty
            || expr.ty != Term::Primitive(Primitive::Void)
            || (self.list_element(&value.ty).is_none() && !self.is_type_list_input(&value.ty))
        {
            return None;
        }
        let simple = match &right.kind {
            K::TypeList(fields) => fields.iter().all(|field| {
                matches!(
                    field.kind,
                    K::Local(_) | K::TypeValue(_) | K::ComptimeValue(_) | K::Literal(_)
                )
            }),
            K::ComptimeValue(_) | K::Local(_) => true,
            _ => false,
        };
        if !simple {
            return None;
        }
        let Value::ComptimeReference(reference) = environment.get(&place.root)? else {
            return None;
        };
        let reference = reference.clone();
        Some((|| {
            self.tick(Some(value.source))?;
            self.tick(Some(left.source))?;
            {
                let contents = reference
                    .borrow(&mut self.budget.steps)
                    .map_err(|kind| self.failure(kind, Some(left.source)))?;
                self.validate_immutable_input(contents.value(), &value.ty)?;
            }
            let suffix = match self.eval(right, environment)? {
                Flow::Value(value) => value,
                flow => return Ok(flow),
            };
            self.validate_immutable_input(&suffix, &value.ty)?;
            // Charge path traversal before borrowing self inside validation.
            // The suffix contains no calls, so the location cannot change.
            let mut path_fuel = self.budget.steps;
            let mut checked = false;
            let result =
                reference.append_list_checked(suffix, &mut path_fuel, |combined, remaining| {
                    checked = true;
                    self.budget.steps = remaining;
                    let work = combined
                        .scoped_copy_work_with_evidence(
                            &self.abstract_type_parameters,
                            &self.abstract_value_parameters,
                        )
                        .ok_or(ErrorKind::RepresentationSizeLimit)?;
                    self.integer_operation(Some(expr.source), |budget| budget.consume(work))
                        .map_err(|error| error.kind)?;
                    self.validate_immutable_input(combined, &value.ty)
                        .map_err(|error| error.kind)
                });
            if !checked {
                self.budget.steps = path_fuel;
            }
            result.map_err(|kind| self.failure(kind, Some(expr.source)))?;
            Ok(Flow::Value(Value::Unit))
        })())
    }

    /// Reuse an owned local list when the suffix cannot execute user code.
    /// Aliased storage keeps its separate reference path; callbacks require the
    /// ordinary left snapshot because they can replace the destination.
    pub(super) fn eval_owned_list_append(
        &mut self,
        expr: &Expr,
        environment: &mut Environment,
    ) -> Option<Result<Flow, Error>> {
        if !self.comptime_only || self.cleanup_schedule.is_some() {
            return None;
        }
        let K::Assign { place, value } = &expr.kind else {
            return None;
        };
        let K::ListConcat(left, right) = &value.kind else {
            return None;
        };
        if !place.fields.is_empty()
            || !matches!(left.kind, K::Local(binding) if binding == place.root)
            || left.ty != value.ty
            || right.ty != value.ty
            || expr.ty != Term::Primitive(Primitive::Void)
            || (self.list_element(&value.ty).is_none() && !self.is_type_list_input(&value.ty))
        {
            return None;
        }
        let simple = match &right.kind {
            K::TypeList(fields) => fields.iter().all(|field| {
                matches!(
                    field.kind,
                    K::Local(_) | K::TypeValue(_) | K::ComptimeValue(_) | K::Literal(_)
                )
            }),
            K::ComptimeValue(_) | K::Local(_) => true,
            _ => false,
        };
        if !simple || !matches!(environment.get(&place.root)?, Value::List { .. }) {
            return None;
        }
        let depth = self.depth;
        let result = (|| {
            // Match ordinary RHS/local evaluation and copy-validation charges,
            // but do not allocate a snapshot that assignment would discard.
            self.tick(Some(value.source))?;
            self.depth += 1;
            self.tick(Some(left.source))?;
            self.depth += 1;
            let original = &environment[&place.root];
            self.check_borrowed_representation(original, left.source, 0)?;
            self.depth -= 1;
            self.validate_immutable_input(original, &left.ty)?;
            let length = match original {
                Value::List { values, .. } => values.len(),
                _ => unreachable!(),
            };
            let suffix = match self.eval(right, environment)? {
                Flow::Value(value) => value,
                flow => return Ok(flow),
            };
            self.validate_immutable_input(&suffix, &right.ty)?;
            let suffix_length = match &suffix {
                Value::List { values, .. } => values.len(),
                _ => return Err(self.failure(ErrorKind::Type, Some(value.source))),
            };
            let count = length
                .checked_add(suffix_length)
                .filter(|count| *count <= 100_000)
                .ok_or_else(|| {
                    self.failure(ErrorKind::RepresentationSizeLimit, Some(value.source))
                })?;
            self.integer_operation(Some(value.source), |budget| {
                budget.consume(count.saturating_add(1))
            })?;
            memory::append_list_checked(
                environment.get_mut(&place.root).unwrap(),
                suffix,
                |combined| {
                    let work = combined
                        .scoped_copy_work_with_evidence(
                            &self.abstract_type_parameters,
                            &self.abstract_value_parameters,
                        )
                        .ok_or(ErrorKind::RepresentationSizeLimit)?;
                    self.integer_operation(Some(value.source), |budget| budget.consume(work))
                        .map_err(|error| error.kind)
                },
            )
            .map_err(|kind| self.failure(kind, Some(value.source)))?;
            Ok(Flow::Value(Value::Unit))
        })();
        self.depth = depth;
        Some(result)
    }

    pub(super) fn eval_list_operation(
        &mut self,
        expr: &Expr,
        environment: &mut Environment,
    ) -> Result<Flow, Error> {
        let (left, right) = match &expr.kind {
            K::ListLength(left) => (left, None),
            K::ListConcat(left, right) => (left, Some(right)),
            _ => unreachable!(),
        };
        self.prepare_list_operation(&expr.ty, &left.ty, right.map(|v| &v.ty), expr.source)?;
        if right.is_none()
            && self.comptime_only
            && self.cleanup_schedule.is_none()
            && let K::Local(binding) = left.kind
        {
            // Inputs and constructed lists have already passed admission.
            // A length observation cannot mutate this immutable representation
            // and does not deliver or copy any element. Keep the local read's
            // tick and storage-lock diagnostics, but avoid its full snapshot.
            self.tick(Some(left.source))?;
            let local = environment
                .get(&binding)
                .ok_or_else(|| self.failure(ErrorKind::MissingBinding, Some(left.source)))?;
            return self.local_list_length(local, &left.ty, left.source, expr.source);
        }
        let value = match self.eval(left, environment)? {
            Flow::Value(value) => value,
            flow => return Ok(flow),
        };
        self.validate_immutable_input(&value, &left.ty)?;
        let length = match &value {
            Value::List { values, .. } => values.len(),
            _ => return Err(self.failure(ErrorKind::Type, Some(expr.source))),
        };
        let Some(right) = right else {
            return self.list_length_value(length, expr.source);
        };
        let other = match self.eval(right, environment)? {
            Flow::Value(value) => value,
            flow => return Ok(flow),
        };
        self.validate_immutable_input(&other, &right.ty)?;
        let other_length = match &other {
            Value::List { values, .. } => values.len(),
            _ => return Err(self.failure(ErrorKind::Type, Some(expr.source))),
        };
        self.complete_list_concat(value, other, length, other_length, expr.source)
    }

    pub(super) fn prepare_list_operation(
        &self,
        ty: &Term,
        left: &Term,
        right: Option<&Term>,
        source: SourceRange,
    ) -> Result<(), Error> {
        let never = Term::Primitive(Primitive::Never);
        let diverges = *left == never || right.is_some_and(|right| *right == never);
        let expected = if diverges {
            &never
        } else if right.is_some() {
            left
        } else {
            &Term::Primitive(Primitive::Integer)
        };
        if *ty != *expected
            || (*left != never
                && self.list_element(left).is_none()
                && !self.is_type_list_input(left))
            || right.is_some_and(|right| *right != never && *left != never && *right != *left)
        {
            return Err(self.failure(ErrorKind::Type, Some(source)));
        }
        Ok(())
    }

    pub(super) fn local_list_length(
        &mut self,
        local: &Value,
        ty: &Term,
        value_source: SourceRange,
        source: SourceRange,
    ) -> Result<Flow, Error> {
        let storage = if let Value::Storage(storage) = local {
            Some(
                storage
                    .0
                    .try_lock()
                    .map_err(|_| self.failure(ErrorKind::InvalidPointer, Some(value_source)))?,
            )
        } else {
            None
        };
        let referenced = if let Value::ComptimeReference(reference) = local {
            Some(
                reference
                    .borrow(&mut self.budget.steps)
                    .map_err(|kind| self.failure(kind, Some(value_source)))?,
            )
        } else {
            None
        };
        let value = referenced
            .as_ref()
            .map(|value| value.value())
            .unwrap_or_else(|| storage.as_deref().unwrap_or(local));
        // Comparing the nominal List<T> identity can itself inspect a
        // large T. Meter that description without walking list elements.
        let work = self
            .type_description_work(ty)
            .ok_or_else(|| self.failure(ErrorKind::Type, Some(value_source)))?;
        self.integer_operation(Some(value_source), |budget| budget.consume(work))?;
        let length = match value {
            Value::List { ty: stored, values }
                if stored == ty && self.list_element(stored).is_some() =>
            {
                values.len()
            }
            _ => return Err(self.failure(ErrorKind::Type, Some(source))),
        };
        return self.list_length_value(length, source);
    }

    pub(super) fn list_operand_length(
        &mut self,
        value: &Value,
        ty: &Term,
        source: SourceRange,
    ) -> Result<usize, Error> {
        self.validate_immutable_input(value, ty)?;
        self.list_value_length(value, source)
    }

    pub(super) fn list_value_length(
        &self,
        value: &Value,
        source: SourceRange,
    ) -> Result<usize, Error> {
        match value {
            Value::List { values, .. } => Ok(values.len()),
            _ => Err(self.failure(ErrorKind::Type, Some(source))),
        }
    }

    pub(super) fn complete_list_concat(
        &mut self,
        value: Value,
        other: Value,
        length: usize,
        other_length: usize,
        source: SourceRange,
    ) -> Result<Flow, Error> {
        let count = length
            .checked_add(other_length)
            .filter(|count| *count <= 100_000)
            .ok_or_else(|| self.failure(ErrorKind::RepresentationSizeLimit, Some(source)))?;
        self.integer_operation(Some(source), |budget| {
            budget.consume(count.saturating_add(1))
        })?;
        let value = match (value, other) {
            (Value::List { ty, mut values }, Value::List { values: right, .. }) => {
                values.extend(right);
                Value::List { ty, values }
            }
            _ => return Err(self.failure(ErrorKind::Type, Some(source))),
        };
        // Both inputs were admitted. A combined immutable description can
        // still exceed the shared structural work limit even below the element
        // limit; report that as a resource limit rather than a malformed type.
        let work = value
            .scoped_copy_work_with_evidence(
                &self.abstract_type_parameters,
                &self.abstract_value_parameters,
            )
            .ok_or_else(|| self.failure(ErrorKind::RepresentationSizeLimit, Some(source)))?;
        self.integer_operation(Some(source), |budget| budget.consume(work))?;
        Ok(Flow::Value(value))
    }

    pub(super) fn list_length_value(
        &mut self,
        length: usize,
        source: SourceRange,
    ) -> Result<Flow, Error> {
        let count = self.integer_operation(Some(source), |budget| {
            budget.consume(1)?;
            Ok(comptime_runtime::Integer::from_u128(length as u128))
        })?;
        Ok(Flow::Value(Value::Integer(count)))
    }
}
