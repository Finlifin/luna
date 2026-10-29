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
            K::TypeList(fields) => fields
                .iter()
                .all(|field| matches!(field.kind, K::Local(_) | K::TypeValue(_) | K::Literal(_))),
            K::TypeListValue(_) => true,
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
        let never = Term::Primitive(Primitive::Never);
        let diverges = left.ty == never || right.is_some_and(|right| right.ty == never);
        let expected = if diverges {
            &never
        } else if right.is_some() {
            &left.ty
        } else {
            &Term::Primitive(Primitive::Integer)
        };
        if expr.ty != *expected
            || (left.ty != never
                && self.list_element(&left.ty).is_none()
                && !self.is_type_list_input(&left.ty))
            || right
                .is_some_and(|right| right.ty != never && left.ty != never && right.ty != left.ty)
        {
            return Err(self.failure(ErrorKind::Type, Some(expr.source)));
        }
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
            let storage = if let Value::Storage(storage) = local {
                Some(
                    storage
                        .0
                        .try_lock()
                        .map_err(|_| self.failure(ErrorKind::InvalidPointer, Some(left.source)))?,
                )
            } else {
                None
            };
            let referenced = if let Value::ComptimeReference(reference) = local {
                Some(
                    reference
                        .borrow(&mut self.budget.steps)
                        .map_err(|kind| self.failure(kind, Some(left.source)))?,
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
                .type_description_work(&left.ty)
                .ok_or_else(|| self.failure(ErrorKind::Type, Some(left.source)))?;
            self.integer_operation(Some(left.source), |budget| budget.consume(work))?;
            let length = match value {
                Value::TypeList(values) if self.is_type_list_input(&left.ty) => values.len(),
                Value::List { ty, values }
                    if *ty == left.ty
                        && self.list_element(ty).is_some_and(|element| {
                            *element != Term::Primitive(Primitive::Type)
                        }) =>
                {
                    values.len()
                }
                _ => return Err(self.failure(ErrorKind::Type, Some(expr.source))),
            };
            return self.list_length_value(length, expr.source);
        }
        let mut value = match self.eval(left, environment)? {
            Flow::Value(value) => value,
            flow => return Ok(flow),
        };
        self.validate_immutable_input(&value, &left.ty)?;
        let length = match &value {
            Value::TypeList(values) => values.len(),
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
            Value::TypeList(values) => values.len(),
            Value::List { values, .. } => values.len(),
            _ => return Err(self.failure(ErrorKind::Type, Some(expr.source))),
        };
        let count = length
            .checked_add(other_length)
            .filter(|count| *count <= 100_000)
            .ok_or_else(|| self.failure(ErrorKind::RepresentationSizeLimit, Some(expr.source)))?;
        self.integer_operation(Some(expr.source), |budget| {
            budget.consume(count.saturating_add(1))
        })?;
        match (&mut value, other) {
            (Value::TypeList(left), Value::TypeList(right)) => left.extend(right),
            (Value::List { values: left, .. }, Value::List { values: right, .. }) => {
                left.extend(right)
            }
            _ => return Err(self.failure(ErrorKind::Type, Some(expr.source))),
        }
        // Both inputs were admitted. A combined immutable description can
        // still exceed the shared structural work limit even below the element
        // limit; report that as a resource limit rather than a malformed type.
        let work = value
            .scoped_copy_work_with_evidence(
                &self.abstract_type_parameters,
                &self.abstract_value_parameters,
            )
            .ok_or_else(|| self.failure(ErrorKind::RepresentationSizeLimit, Some(expr.source)))?;
        self.integer_operation(Some(expr.source), |budget| budget.consume(work))?;
        Ok(Flow::Value(value))
    }

    fn list_length_value(&mut self, length: usize, source: SourceRange) -> Result<Flow, Error> {
        let count = self.integer_operation(Some(source), |budget| {
            budget.consume(1)?;
            Ok(comptime_runtime::Integer::from_u128(length as u128))
        })?;
        Ok(Flow::Value(Value::Integer(count)))
    }
}
