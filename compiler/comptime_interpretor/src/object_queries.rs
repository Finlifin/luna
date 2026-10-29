//! Ordered Object reads preserve entry domains instead of erasing them.
use super::*;

pub(super) enum LocalSelector<'a> {
    Value(Option<&'a Value>, SourceRange),
    Literal(&'a Expr),
}

impl Interpreter<'_> {
    pub(super) fn query_local_object(
        &mut self,
        query: nir::program::ObjectQueryKind,
        ty: &Term,
        receiver: &Value,
        receiver_source: SourceRange,
        selector: Option<LocalSelector<'_>>,
        source: SourceRange,
    ) -> Result<Flow, Error> {
        let independent = match &selector {
            None | Some(LocalSelector::Literal(_)) => true,
            Some(LocalSelector::Value(Some(Value::Integer(_) | Value::Symbol(_)), _)) => true,
            _ => false,
        };
        self.tick(Some(receiver_source))?;
        if independent
            && self.cleanup_schedule.is_none()
            && *ty != Term::Primitive(Primitive::Never)
            && matches!(
                receiver,
                Value::Object(_) | Value::Storage(_) | Value::ComptimeReference(_)
            )
        {
            let root;
            let reference = match receiver {
                Value::Storage(storage) => {
                    root = value_reference::ValueReference::root(storage.clone());
                    Some(&root)
                }
                Value::ComptimeReference(reference) => Some(reference),
                _ => None,
            };
            let contents = reference
                .map(|reference| {
                    reference
                        .borrow(&mut self.budget.steps)
                        .map_err(|kind| Error {
                            kind,
                            source: Some(receiver_source),
                        })
                })
                .transpose()?;
            let value = contents
                .as_ref()
                .map_or(receiver, |contents| contents.value());
            self.check_borrowed_representation(value, receiver_source, 0)?;
            let selector = self.read_object_selector(selector)?;
            // Account for packet construction even though this path retains no
            // Object snapshot. No user computation runs under the receiver lock.
            self.tick(Some(source))?;
            let Value::Object(object) = value else {
                return Err(self.failure(ErrorKind::Type, Some(source)));
            };
            return self.query_object(query, ty, object, selector, source);
        }
        // A selector can reference a sibling in the same storage. Snapshot the
        // receiver and release its lock before reading that selector.
        let receiver = self.invocation_slot_value(receiver, Some(receiver_source))?;
        let selector = self.read_object_selector(selector)?;
        self.tick(Some(source))?;
        let mut packet = vec![receiver];
        packet.extend(selector);
        self.complete_object_query(query, ty, Value::Tuple(packet), source)
    }

    fn read_object_selector(
        &mut self,
        selector: Option<LocalSelector<'_>>,
    ) -> Result<Option<Value>, Error> {
        let Some(selector) = selector else {
            return Ok(None);
        };
        match selector {
            LocalSelector::Value(value, source) => {
                self.tick(Some(source))?;
                let value =
                    value.ok_or_else(|| self.failure(ErrorKind::MissingBinding, Some(source)))?;
                self.invocation_slot_value(value, Some(source)).map(Some)
            }
            LocalSelector::Literal(expr) => {
                self.tick(Some(expr.source))?;
                let Flow::Value(value) = self.eval_literal(expr, &mut Environment::new())? else {
                    return Err(self.failure(ErrorKind::InvalidControlFlow, Some(expr.source)));
                };
                Ok(Some(value))
            }
        }
    }

    pub(super) fn eval_object_query(
        &mut self,
        expr: &Expr,
        environment: &mut Environment,
    ) -> Result<Flow, Error> {
        let K::ObjectQuery { subject, query } = &expr.kind else {
            unreachable!()
        };
        self.prepare_object_query(*query, &expr.ty, expr.source)?;
        if let Some(result) = self.eval_borrowed_object_query(expr, subject, environment) {
            return result;
        }
        self.eval_object_query_packet(expr, subject, environment)
    }

    pub(super) fn prepare_object_query(
        &mut self,
        query: nir::program::ObjectQueryKind,
        ty: &Term,
        source: SourceRange,
    ) -> Result<(), Error> {
        use nir::program::ObjectQueryKind as Q;
        if !self.comptime_only {
            return Err(self.failure(ErrorKind::PhaseMismatch, Some(source)));
        }
        let failure = || Error {
            kind: ErrorKind::Type,
            source: Some(source),
        };
        let expected = match query {
            Q::Length => Term::Primitive(Primitive::Integer),
            Q::Key => Term::Optional(Box::new(Term::Primitive(Primitive::Symbol))),
            Q::EntryType => Term::Primitive(Primitive::Type),
            Q::Value | Q::NamedValue => {
                if !matches!(*ty, Term::Optional(_) | Term::Primitive(Primitive::Never)) {
                    return Err(failure());
                }
                ty.clone()
            }
        };
        if *ty != expected && *ty != Term::Primitive(Primitive::Never) {
            return Err(failure());
        }
        Ok(())
    }

    // Leaf selectors cannot mutate an Object while it is observed. Under a
    // receiver lock, local selectors must be owned scalars so reading one
    // cannot borrow the same storage again. Other cases retain snapshots.
    fn eval_borrowed_object_query(
        &mut self,
        expr: &Expr,
        subject: &Expr,
        environment: &Environment,
    ) -> Option<Result<Flow, Error>> {
        use nir::program::ObjectQueryKind as Q;
        if self.cleanup_schedule.is_some() || expr.ty == Term::Primitive(Primitive::Never) {
            return None;
        }
        let K::ObjectQuery { query, .. } = &expr.kind else {
            return None;
        };
        let K::Tuple(fields) = &subject.kind else {
            return None;
        };
        let length = *query == Q::Length;
        if fields.len() != if length { 1 } else { 2 } {
            return None;
        }
        let K::Local(binding) = fields[0].kind else {
            return None;
        };
        let local = environment.get(&binding)?;
        let referenced = matches!(local, Value::Storage(_) | Value::ComptimeReference(_));
        if !matches!(local, Value::Object(_)) && !referenced {
            return None;
        }
        if !length && !matches!(fields[1].kind, K::Local(_) | K::Literal(_)) {
            return None;
        }
        if referenced && !length {
            let independent = match fields[1].kind {
                K::Literal(_) => true,
                K::Local(selector) => matches!(
                    environment.get(&selector),
                    Some(Value::Integer(_) | Value::Symbol(_))
                ),
                _ => false,
            };
            if !independent {
                return None;
            }
        }
        let depth = self.depth;
        let result = (|| {
            // Reproduce tuple and local evaluation boundaries, including depth
            // failures and the complete representation-checking fuel charge.
            self.tick(Some(subject.source))?;
            self.depth += 1;
            self.tick(Some(fields[0].source))?;
            self.depth += 1;
            let root;
            let reference = match local {
                Value::Storage(storage) => {
                    root = value_reference::ValueReference::root(storage.clone());
                    Some(&root)
                }
                Value::ComptimeReference(reference) => Some(reference),
                _ => None,
            };
            let contents = reference
                .map(|reference| {
                    reference
                        .borrow(&mut self.budget.steps)
                        .map_err(|kind| Error {
                            kind,
                            source: Some(fields[0].source),
                        })
                })
                .transpose()?;
            let value = contents.as_ref().map_or(local, |contents| contents.value());
            self.check_borrowed_representation(value, fields[0].source, 0)?;
            self.depth -= 1;
            let selector = if length {
                None
            } else {
                let selector = &fields[1];
                Some(match selector.kind {
                    K::Local(binding) => {
                        self.tick(Some(selector.source))?;
                        self.depth += 1;
                        let value = self.local_value(environment, binding, Some(selector.source));
                        self.depth -= 1;
                        value?
                    }
                    _ => match self.eval(selector, &mut Environment::new())? {
                        Flow::Value(value) => value,
                        flow => return Ok(flow),
                    },
                })
            };
            self.depth = depth;
            let Value::Object(object) = value else {
                return Err(self.failure(ErrorKind::Type, Some(expr.source)));
            };
            self.query_object(*query, &expr.ty, object, selector, expr.source)
        })();
        self.depth = depth;
        Some(result)
    }

    fn eval_object_query_packet(
        &mut self,
        expr: &Expr,
        subject: &Expr,
        environment: &mut Environment,
    ) -> Result<Flow, Error> {
        let packet = match self.eval(subject, environment)? {
            Flow::Value(value) => value,
            flow => return Ok(flow),
        };
        let K::ObjectQuery { query, .. } = &expr.kind else {
            unreachable!()
        };
        self.complete_object_query(*query, &expr.ty, packet, expr.source)
    }

    pub(super) fn complete_object_query(
        &mut self,
        query: nir::program::ObjectQueryKind,
        ty: &Term,
        packet: Value,
        source: SourceRange,
    ) -> Result<Flow, Error> {
        let failure = || Error {
            kind: ErrorKind::Type,
            source: Some(source),
        };
        if *ty == Term::Primitive(Primitive::Never) {
            return Err(failure());
        }
        let Value::Tuple(packet) = packet else {
            return Err(failure());
        };
        let mut packet = packet.into_iter();
        let Some(Value::Object(object)) = packet.next() else {
            return Err(failure());
        };
        let selector = packet.next();
        if packet.next().is_some() {
            return Err(failure());
        }
        self.query_object(query, ty, &object, selector, source)
    }

    fn query_object(
        &mut self,
        query: nir::program::ObjectQueryKind,
        ty: &Term,
        object: &Object<ObjectValue>,
        selector: Option<Value>,
        source: SourceRange,
    ) -> Result<Flow, Error> {
        use nir::program::ObjectQueryKind as Q;
        let failure = || Error {
            kind: ErrorKind::Type,
            source: Some(source),
        };
        if query == Q::Length {
            if selector.is_some() {
                return Err(failure());
            }
            return Ok(Flow::Value(Value::Integer(
                comptime_runtime::Integer::from_u128(object.entries().len() as u128),
            )));
        }
        if query == Q::NamedValue {
            let Some(Value::Symbol(key)) = selector else {
                return Err(failure());
            };
            let Term::Optional(target) = ty else {
                return Err(failure());
            };
            let property =
                self.lookup_object_property(object, &key, target)
                    .map_err(|mut error| {
                        if error.source.is_none() {
                            error.source = Some(source);
                        }
                        error
                    })?;
            let value = match property {
                ObjectProperty::Found(entry) => Some(Box::new(
                    self.checkpoint_representation(&entry.value, source)?,
                )),
                ObjectProperty::Missing | ObjectProperty::TypeMismatch(_) => None,
            };
            return Ok(Flow::Value(Value::Optional {
                ty: ty.clone(),
                value,
            }));
        }
        let Some(Value::Integer(index)) = selector else {
            return Err(failure());
        };
        let index = index
            .to_u128()
            .ok()
            .and_then(|value| usize::try_from(value).ok())
            .filter(|index| *index < object.entries().len())
            .ok_or_else(|| self.failure(ErrorKind::IndexOutOfBounds, Some(source)))?;
        let entry = &object.entries()[index];
        let value = match query {
            Q::Length | Q::NamedValue => unreachable!(),
            Q::Key => {
                if let Some(key) = entry.key() {
                    self.integer_operation(Some(source), |budget| {
                        budget.consume(key.len().saturating_add(1))
                    })?;
                }
                Value::Optional {
                    ty: ty.clone(),
                    value: entry.key().map(|key| Box::new(Value::Symbol(key.into()))),
                }
            }
            Q::EntryType => {
                let work = self
                    .type_description_work(&entry.value().ty)
                    .ok_or_else(failure)?;
                self.integer_operation(Some(source), |budget| budget.consume(work))?;
                Value::Type(entry.value().ty.clone())
            }
            Q::Value => {
                let Term::Optional(target) = ty else {
                    return Err(failure());
                };
                let work = self
                    .type_description_work(target)
                    .and_then(|left| {
                        self.type_description_work(&entry.value().ty)
                            .and_then(|right| left.checked_add(right))
                    })
                    .ok_or_else(failure)?;
                self.integer_operation(Some(source), |budget| budget.consume(work))?;
                let value = if entry.value().ty == **target {
                    if !self.has_type(&entry.value().value, target) {
                        return Err(failure());
                    }
                    Some(Box::new(
                        self.checkpoint_representation(&entry.value().value, source)?,
                    ))
                } else {
                    None
                };
                Value::Optional {
                    ty: ty.clone(),
                    value,
                }
            }
        };
        Ok(Flow::Value(value))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nir::program::ObjectQueryKind as Q;

    fn expression(query: Q, ty: Term) -> Expr {
        let source = SourceRange { start: 1, end: 2 };
        let object = Expr {
            source,
            ty: Term::Primitive(Primitive::Object),
            kind: K::Local(LocalId { owner: 1, index: 0 }),
        };
        let index = Expr {
            source,
            ty: Term::Primitive(Primitive::Integer),
            kind: K::Literal(nir::program::Literal::Integer("0".into())),
        };
        Expr {
            source,
            ty,
            kind: K::ObjectQuery {
                query,
                subject: Box::new(Expr {
                    source,
                    ty: Term::Tuple(vec![object.ty.clone(), index.ty.clone()]),
                    kind: K::Tuple(vec![object, index]),
                }),
            },
        }
    }

    fn observation(query: Q) -> Expr {
        let ty = match query {
            Q::Length => Term::Primitive(Primitive::Integer),
            Q::Key => Term::Optional(Box::new(Term::Primitive(Primitive::Symbol))),
            Q::EntryType => Term::Primitive(Primitive::Type),
            _ => Term::Optional(Box::new(Term::Primitive(Primitive::Bool))),
        };
        let mut expr = expression(query, ty);
        let K::ObjectQuery { subject, .. } = &mut expr.kind else {
            unreachable!()
        };
        let K::Tuple(fields) = &mut subject.kind else {
            unreachable!()
        };
        if query == Q::Length {
            fields.truncate(1);
        } else if query == Q::NamedValue {
            fields[1].ty = Term::Primitive(Primitive::Symbol);
            fields[1].kind = K::Literal(nir::program::Literal::Symbol("flag".into()));
        }
        subject.ty = Term::Tuple(fields.iter().map(|field| field.ty.clone()).collect());
        expr
    }

    fn object() -> Value {
        Value::Object(Object::from_entries(vec![
            ObjectEntry::Property {
                key: "flag".into(),
                value: ObjectValue {
                    ty: Term::Primitive(Primitive::Bool),
                    value: Value::Bool(true),
                },
            },
            ObjectEntry::Value(ObjectValue {
                ty: Term::Optional(Box::new(Term::Primitive(Primitive::Bool))),
                value: Value::Optional {
                    ty: Term::Optional(Box::new(Term::Primitive(Primitive::Bool))),
                    value: Some(Box::new(Value::Bool(false))),
                },
            }),
        ]))
    }

    fn value(result: Result<Flow, Error>) -> Result<Value, Error> {
        result.map(|flow| match flow {
            Flow::Value(value) => value,
            _ => panic!("unexpected flow"),
        })
    }

    #[test]
    fn borrowed_queries_preserve_fuel_depth_errors_and_recovery() {
        let package = Package::default();
        for mode in 0..3 {
            for query in [Q::Length, Q::Key, Q::EntryType, Q::Value, Q::NamedValue] {
                let expr = observation(query);
                let K::ObjectQuery { subject, .. } = &expr.kind else {
                    unreachable!()
                };
                for depth in [1, 2, 3, 4, 8, 128] {
                    for steps in 0..180 {
                        let mut fast =
                            Interpreter::for_comptime(&package, Budget::default()).unwrap();
                        let mut slow =
                            Interpreter::for_comptime(&package, Budget::default()).unwrap();
                        fast.budget = Budget { steps, depth };
                        slow.budget = fast.budget;
                        let mut env = Environment::new();
                        let receiver = match mode {
                            0 => object(),
                            1 => Value::Storage(Storage::new(object())),
                            _ => Value::ComptimeReference(value_reference::ValueReference::root(
                                Storage::new(object()),
                            )),
                        };
                        env.insert(LocalId { owner: 1, index: 0 }, receiver);
                        let result = fast
                            .eval_borrowed_object_query(&expr, subject, &env)
                            .unwrap();
                        let expected = slow.eval_object_query_packet(&expr, subject, &mut env);
                        assert_eq!(
                            value(result),
                            value(expected),
                            "{query:?} depth={depth} steps={steps}"
                        );
                        assert_eq!(fast.budget.steps, slow.budget.steps);
                        assert_eq!(fast.depth, 0);
                        fast.budget = Budget {
                            steps: 1000,
                            depth: 128,
                        };
                        slow.budget = fast.budget;
                        assert_eq!(
                            value(fast.eval_object_query(&expr, &mut env)),
                            value(slow.eval_object_query_packet(&expr, subject, &mut env))
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn borrowed_length_handles_storage_and_value_references() {
        let package = Package::default();
        let expr = observation(Q::Length);
        let K::ObjectQuery { subject, .. } = &expr.kind else {
            unreachable!()
        };
        let storage = Storage::new(object());
        for local in [
            Value::Storage(storage.clone()),
            Value::ComptimeReference(value_reference::ValueReference::root(storage.clone())),
        ] {
            for steps in 0..90 {
                let budget = Budget { steps, depth: 128 };
                let mut fast = Interpreter::for_comptime(&package, Budget::default()).unwrap();
                let mut slow = Interpreter::for_comptime(&package, Budget::default()).unwrap();
                fast.budget = budget;
                slow.budget = budget;
                let mut env = Environment::new();
                env.insert(LocalId { owner: 1, index: 0 }, local.clone());
                let result = fast
                    .eval_borrowed_object_query(&expr, subject, &env)
                    .unwrap();
                assert_eq!(
                    value(result),
                    value(slow.eval_object_query_packet(&expr, subject, &mut env))
                );
                assert_eq!(fast.budget.steps, slow.budget.steps);
                assert_eq!(fast.depth, 0);
            }
        }
        let _locked = storage.0.lock().unwrap();
        let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
        let mut env = Environment::new();
        env.insert(
            LocalId { owner: 1, index: 0 },
            Value::Storage(storage.clone()),
        );
        assert_eq!(
            vm.eval(&expr, &mut env).unwrap_err().kind,
            ErrorKind::InvalidValueReference
        );
    }

    #[test]
    fn borrowed_local_selectors_preserve_bounds_missing_bindings_and_domains() {
        let package = Package::default();
        for query in [Q::Key, Q::EntryType, Q::Value, Q::NamedValue] {
            let mut expr = observation(query);
            let K::ObjectQuery { subject, .. } = &mut expr.kind else {
                unreachable!()
            };
            let K::Tuple(fields) = &mut subject.kind else {
                unreachable!()
            };
            fields[1].kind = K::Local(LocalId { owner: 1, index: 1 });
            let selectors = vec![
                None,
                Some(Value::Integer(comptime_runtime::Integer::from_u128(0))),
                Some(Value::Integer(comptime_runtime::Integer::from_u128(1))),
                Some(Value::Integer(comptime_runtime::Integer::from_u128(99))),
                Some(Value::Symbol("flag".into())),
                Some(Value::Symbol("missing".into())),
            ];
            let K::ObjectQuery { subject, .. } = &expr.kind else {
                unreachable!()
            };
            for selector in selectors {
                let mut env = Environment::new();
                env.insert(LocalId { owner: 1, index: 0 }, object());
                if let Some(selector) = selector {
                    env.insert(LocalId { owner: 1, index: 1 }, selector);
                }
                let mut fast = Interpreter::for_comptime(&package, Budget::default()).unwrap();
                let mut slow = Interpreter::for_comptime(&package, Budget::default()).unwrap();
                let result = fast
                    .eval_borrowed_object_query(&expr, subject, &env)
                    .unwrap();
                assert_eq!(
                    value(result),
                    value(slow.eval_object_query_packet(&expr, subject, &mut env))
                );
                assert_eq!(fast.budget.steps, slow.budget.steps);
            }
        }
    }

    #[test]
    fn computed_selectors_retain_snapshots_and_reference_payloads_can_borrow() {
        let package = Package::default();
        let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
        let mut env = Environment::new();
        env.insert(LocalId { owner: 1, index: 0 }, object());
        let mut expr = observation(Q::Value);
        let K::ObjectQuery { subject, .. } = &mut expr.kind else {
            unreachable!()
        };
        let K::Tuple(fields) = &mut subject.kind else {
            unreachable!()
        };
        fields[1] = Expr {
            source: fields[1].source,
            ty: fields[1].ty.clone(),
            kind: K::Block {
                statements: vec![],
                result: Some(Box::new(fields[1].clone())),
            },
        };
        let K::ObjectQuery { subject, .. } = &expr.kind else {
            unreachable!()
        };
        assert!(
            vm.eval_borrowed_object_query(&expr, subject, &env)
                .is_none()
        );
        let expected = value(vm.eval_object_query(&expr, &mut env)).unwrap();
        let expr = observation(Q::Value);
        let K::ObjectQuery { subject, .. } = &expr.kind else {
            unreachable!()
        };
        env.insert(
            LocalId { owner: 1, index: 0 },
            Value::Storage(Storage::new(object())),
        );
        assert_eq!(
            value(
                vm.eval_borrowed_object_query(&expr, subject, &env)
                    .expect("borrowed receiver")
            )
            .unwrap(),
            expected
        );
    }

    #[test]
    fn sibling_reference_selector_is_read_outside_the_object_lock() {
        let package = Package::default();
        let storage = Storage::new(Value::AnonymousRecord(vec![
            ("object".into(), object()),
            (
                "index".into(),
                Value::Integer(comptime_runtime::Integer::from_u128(0)),
            ),
        ]));
        let mut env = Environment::new();
        for index in 0..2 {
            let reference = value_reference::ValueReference::projected(
                storage.clone(),
                &[(nir::program::FieldOwner::Anonymous, index)],
                &mut 1000,
            )
            .unwrap();
            env.insert(
                LocalId {
                    owner: 1,
                    index: index as u32,
                },
                Value::ComptimeReference(reference),
            );
        }
        let mut expr = observation(Q::Value);
        let K::ObjectQuery { subject, .. } = &mut expr.kind else {
            unreachable!()
        };
        let K::Tuple(fields) = &mut subject.kind else {
            unreachable!()
        };
        fields[1].kind = K::Local(LocalId { owner: 1, index: 1 });
        let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
        assert!(
            vm.eval_borrowed_object_query(
                &expr,
                match &expr.kind {
                    K::ObjectQuery { subject, .. } => subject,
                    _ => unreachable!(),
                },
                &env
            )
            .is_none()
        );
        assert_eq!(
            value(vm.eval_object_query(&expr, &mut env)).unwrap(),
            Value::Optional {
                ty: expr.ty.clone(),
                value: Some(Box::new(Value::Bool(true))),
            }
        );
    }

    #[test]
    fn malformed_entry_cannot_be_extracted_by_matching_only_its_tag() {
        let package = Package::default();
        let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
        let mut env = Environment::new();
        env.insert(
            LocalId { owner: 1, index: 0 },
            Value::Object(Object::from_entries(vec![
                comptime_runtime::object::Entry::Value(ObjectValue {
                    ty: Term::Primitive(Primitive::Bool),
                    value: Value::Char('x'),
                }),
            ])),
        );
        let expr = expression(
            Q::Value,
            Term::Optional(Box::new(Term::Primitive(Primitive::Bool))),
        );
        assert_eq!(vm.eval(&expr, &mut env).unwrap_err().kind, ErrorKind::Type);
    }

    #[test]
    fn invalid_result_and_runtime_phase_fail_before_reading_the_packet() {
        let package = Package::default();
        let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
        let expr = expression(Q::Value, Term::Primitive(Primitive::Bool));
        assert_eq!(
            vm.eval(&expr, &mut Environment::new()).unwrap_err().kind,
            ErrorKind::Type
        );
        vm.comptime_only = false;
        assert_eq!(
            vm.eval(&expr, &mut Environment::new()).unwrap_err().kind,
            ErrorKind::PhaseMismatch
        );
    }

    #[test]
    fn named_lookup_rejects_invalid_unselected_payloads() {
        let package = Package::default();
        let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
        let mut env = Environment::new();
        env.insert(
            LocalId { owner: 1, index: 0 },
            Value::Object(Object::from_entries(vec![
                ObjectEntry::Property {
                    key: "hidden".into(),
                    value: ObjectValue {
                        ty: Term::Primitive(Primitive::Bool),
                        value: Value::Char('x'),
                    },
                },
                ObjectEntry::Property {
                    key: "flag".into(),
                    value: ObjectValue {
                        ty: Term::Primitive(Primitive::Bool),
                        value: Value::Bool(true),
                    },
                },
            ])),
        );
        let mut expr = expression(
            Q::NamedValue,
            Term::Optional(Box::new(Term::Primitive(Primitive::Bool))),
        );
        let K::ObjectQuery { subject, .. } = &mut expr.kind else {
            unreachable!()
        };
        let K::Tuple(fields) = &mut subject.kind else {
            unreachable!()
        };
        fields[1].ty = Term::Primitive(Primitive::Symbol);
        fields[1].kind = K::Literal(nir::program::Literal::Symbol("flag".into()));
        subject.ty = Term::Tuple(fields.iter().map(|field| field.ty.clone()).collect());
        let failure = vm.eval(&expr, &mut env).unwrap_err();
        assert_eq!(failure.kind, ErrorKind::Type);
        assert!(failure.source.is_some());
    }
}
