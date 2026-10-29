//! Comptime container transfers validate data and permission before mutation.
use super::*;

impl Interpreter<'_> {
    pub(super) fn validate_immutable_storage(
        &mut self,
        value: &Value,
        ty: &Term,
        source: SourceRange,
    ) -> Result<(), Error> {
        self.validate_immutable_input(value, ty)
            .map_err(|mut error| {
                if error.source.is_none() {
                    error.source = Some(source);
                }
                error
            })?;
        self.immutable_storage_domain(ty, source, 0)?;
        self.immutable_storage_contents(value, source, 0)
    }

    fn immutable_storage_domain(
        &mut self,
        ty: &Term,
        source: SourceRange,
        depth: usize,
    ) -> Result<(), Error> {
        self.tick(Some(source))?;
        if depth >= 128 {
            return Err(self.failure(ErrorKind::DepthExhausted, Some(source)));
        }
        match ty {
            // Object entries are checked by their retained domains below.
            // Type descriptions are immutable comptime data, not runtime Copy.
            Term::Primitive(Primitive::Object | Primitive::Type | Primitive::Never) => Ok(()),
            Term::Optional(inner) => self.immutable_storage_domain(inner, source, depth + 1),
            Term::Tuple(fields) => {
                for field in fields {
                    self.immutable_storage_domain(field, source, depth + 1)?;
                }
                Ok(())
            }
            Term::ErrorResult { errors, result } => {
                for field in errors.iter().chain(std::iter::once(result.as_ref())) {
                    self.immutable_storage_domain(field, source, depth + 1)?;
                }
                Ok(())
            }
            _ => {
                if let Some(element) = self.list_element(ty).cloned() {
                    return self.immutable_storage_domain(&element, source, depth + 1);
                }
                if self.is_type_list_input(ty) || self.permits_pointer_copy(ty) {
                    Ok(())
                } else {
                    Err(self.failure(ErrorKind::Unsupported, Some(source)))
                }
            }
        }
    }

    fn immutable_storage_contents(
        &mut self,
        value: &Value,
        source: SourceRange,
        depth: usize,
    ) -> Result<(), Error> {
        self.tick(Some(source))?;
        if depth >= 128 {
            return Err(self.failure(ErrorKind::DepthExhausted, Some(source)));
        }
        match value {
            Value::Object(object) => {
                for entry in object.entries() {
                    let entry = entry.value();
                    self.immutable_storage_domain(&entry.ty, source, depth + 1)?;
                    self.immutable_storage_contents(&entry.value, source, depth + 1)?;
                }
            }
            Value::List { values, .. }
            | Value::Tuple(values)
            | Value::Record { fields: values, .. }
            | Value::Variant { fields: values, .. } => {
                for value in values {
                    self.immutable_storage_contents(value, source, depth + 1)?;
                }
            }
            Value::AnonymousRecord(fields) => {
                for (_, value) in fields {
                    self.immutable_storage_contents(value, source, depth + 1)?;
                }
            }
            Value::Optional {
                value: Some(value), ..
            }
            | Value::ErrorResult { value, .. }
            | Value::Newtype { value, .. } => {
                self.immutable_storage_contents(value, source, depth + 1)?;
            }
            _ => {}
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn object(value: bool) -> Value {
        Value::Object(Object::from_entries(vec![ObjectEntry::Property {
            key: "flag".into(),
            value: ObjectValue {
                ty: Term::Primitive(Primitive::Bool),
                value: Value::Bool(value),
            },
        }]))
    }

    fn write() -> Expr {
        let source = SourceRange { start: 3, end: 9 };
        let ty = Term::Primitive(Primitive::Object);
        Expr {
            source,
            ty: Term::Primitive(Primitive::Void),
            kind: K::WritePointer {
                pointer: Box::new(Expr {
                    source,
                    ty: Term::Pointer(Box::new(ty.clone())),
                    kind: K::Local(LocalId { owner: 1, index: 0 }),
                }),
                value: Box::new(Expr {
                    source,
                    ty,
                    kind: K::Local(LocalId { owner: 1, index: 1 }),
                }),
            },
        }
    }

    fn environment(storage: &Storage, value: Value) -> Environment {
        let mut env = Environment::new();
        env.insert(
            LocalId { owner: 1, index: 0 },
            Value::Pointer(storage.pointer(Term::Primitive(Primitive::Object), vec![])),
        );
        env.insert(LocalId { owner: 1, index: 1 }, value);
        env
    }

    #[test]
    fn invalid_old_or_new_object_cannot_mutate_storage() {
        let invalid = Value::Object(Object::from_entries(vec![ObjectEntry::Value(
            ObjectValue {
                ty: Term::Primitive(Primitive::Bool),
                value: Value::Char('x'),
            },
        )]));
        let target = Storage::new(Value::Bool(false));
        let active = Value::Object(Object::from_entries(vec![ObjectEntry::Value(
            ObjectValue {
                ty: Term::Pointer(Box::new(Term::Primitive(Primitive::Bool))),
                value: Value::Pointer(target.pointer(Term::Primitive(Primitive::Bool), vec![])),
            },
        )]));
        let package = Package::default();
        for (old, new) in [
            (object(false), invalid.clone()),
            (invalid, object(true)),
            (object(false), active.clone()),
            (active, object(true)),
        ] {
            let storage = Storage::new(old.clone());
            let mut env = environment(&storage, new);
            let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
            let error = vm.eval(&write(), &mut env).unwrap_err();
            assert_eq!(error.kind, ErrorKind::Type);
            assert!(error.source.is_some());
            assert_eq!(storage.read().unwrap(), old);
        }
    }

    #[test]
    fn exhausted_object_transfer_keeps_old_value_and_can_retry() {
        let package = Package::default();
        let mut failures = 0;
        let mut successes = 0;
        for steps in 1..100 {
            let storage = Storage::new(object(false));
            let mut env = environment(&storage, object(true));
            let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
            vm.budget.steps = steps;
            match vm.eval(&write(), &mut env) {
                Ok(Flow::Value(Value::Unit)) => {
                    successes += 1;
                    assert_eq!(storage.read().unwrap(), object(true));
                }
                Err(error) => {
                    failures += 1;
                    assert_eq!(error.kind, ErrorKind::StepsExhausted);
                    assert_eq!(storage.read().unwrap(), object(false));
                    vm.budget.steps = 1000;
                    vm.eval(&write(), &mut env).unwrap();
                    assert_eq!(storage.read().unwrap(), object(true));
                }
                other => panic!("unexpected transfer: {other:?}"),
            }
        }
        assert!(failures > 0 && successes > 0);
    }

    #[test]
    fn object_transfer_remains_comptime_only() {
        let package = Package::default();
        let storage = Storage::new(object(false));
        let mut env = environment(&storage, object(true));
        let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
        assert!(vm.eval(&write(), &mut env).is_err());
        assert_eq!(storage.read().unwrap(), object(false));
    }

    #[test]
    fn fused_list_append_checks_budget_before_committing_and_can_retry() {
        use nir::normal_forms::{Head, Kind, Parameter as Input, Role};
        let head = DeclarationId {
            package: 1,
            local: 1,
        };
        let ty = Term::NormalForm {
            head,
            sort: nir::Sort::Type,
            arguments: vec![Term::Primitive(Primitive::Bool)],
        };
        let package = Package {
            normal_forms: vec![Head {
                role: Role::TypeList,
                declaration: head,
                requirements: vec![],
                parameters: vec![Input {
                    declaration: DeclarationId {
                        package: 1,
                        local: 2,
                    },
                    binding: LocalId { owner: 1, index: 0 },
                    kind: Kind::Type,
                    ty: Term::Primitive(Primitive::Type),
                }],
            }],
            ..Default::default()
        };
        let source = SourceRange { start: 1, end: 2 };
        let pointer = Expr {
            source,
            ty: Term::Pointer(Box::new(ty.clone())),
            kind: K::Local(LocalId { owner: 1, index: 0 }),
        };
        let expr = Expr {
            source,
            ty: Term::Primitive(Primitive::Void),
            kind: K::WritePointer {
                pointer: Box::new(pointer.clone()),
                value: Box::new(Expr {
                    source,
                    ty: ty.clone(),
                    kind: K::ListConcat(
                        Box::new(Expr {
                            source,
                            ty: ty.clone(),
                            kind: K::ReadPointer(Box::new(pointer)),
                        }),
                        Box::new(Expr {
                            source,
                            ty: ty.clone(),
                            kind: K::TypeList(vec![Expr {
                                source,
                                ty: Term::Primitive(Primitive::Bool),
                                kind: K::Literal(Literal::Bool(true)),
                            }]),
                        }),
                    ),
                }),
            },
        };
        let original = Value::List {
            ty: ty.clone(),
            values: vec![Value::Bool(false)],
        };
        let expected = Value::List {
            ty: ty.clone(),
            values: vec![Value::Bool(false), Value::Bool(true)],
        };
        let K::WritePointer { value, .. } = &expr.kind else {
            unreachable!()
        };
        let mut reference_value = value.clone();
        let K::ListConcat(left, _) = &mut reference_value.kind else {
            unreachable!()
        };
        left.kind = K::Local(LocalId { owner: 1, index: 0 });
        let reference_expr = Expr {
            source,
            ty: Term::Primitive(Primitive::Void),
            kind: K::Assign {
                place: nir::program::Place {
                    root: LocalId { owner: 1, index: 0 },
                    fields: vec![],
                },
                value: reference_value,
            },
        };
        for mode in 0..5 {
            let wrap = |value| {
                if mode == 2 {
                    Value::AnonymousRecord(vec![
                        ("items".into(), value),
                        ("sibling".into(), Value::Bool(true)),
                    ])
                } else {
                    value
                }
            };
            let mut reference_expr = reference_expr.clone();
            if mode >= 3 {
                let K::Assign { value, .. } = &mut reference_expr.kind else {
                    unreachable!()
                };
                let K::ListConcat(_, right) = &mut value.kind else {
                    unreachable!()
                };
                right.kind = K::Local(LocalId {
                    owner: 1,
                    index: if mode == 4 { 0 } else { 1 },
                });
            }
            let original = wrap(original.clone());
            let expected = wrap(if mode == 4 {
                Value::List {
                    ty: ty.clone(),
                    values: vec![Value::Bool(false); 2],
                }
            } else {
                expected.clone()
            });
            let mut failures = 0;
            let mut successes = 0;
            for steps in 1..200 {
                let storage = Storage::new(original.clone());
                let mut env = Environment::new();
                let receiver = if mode == 0 {
                    Value::Pointer(storage.pointer(ty.clone(), vec![]))
                } else {
                    let reference = if mode == 2 {
                        value_reference::ValueReference::projected(
                            storage.clone(),
                            &[(nir::program::FieldOwner::Anonymous, 0)],
                            &mut 1000,
                        )
                        .unwrap()
                    } else {
                        value_reference::ValueReference::root(storage.clone())
                    };
                    Value::ComptimeReference(reference)
                };
                env.insert(LocalId { owner: 1, index: 0 }, receiver);
                env.insert(
                    LocalId { owner: 1, index: 1 },
                    Value::List {
                        ty: ty.clone(),
                        values: vec![Value::Bool(true)],
                    },
                );
                let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
                vm.budget.steps = steps;
                let mut append = |vm: &mut Interpreter<'_>| {
                    if mode == 0 {
                        vm.eval_list_append(&expr, &mut env)
                            .expect("pointer append")
                    } else {
                        vm.eval_reference_list_append(&reference_expr, &mut env)
                            .expect("reference append")
                    }
                };
                match append(&mut vm) {
                    Ok(Flow::Value(Value::Unit)) => {
                        successes += 1;
                        assert_eq!(storage.read().unwrap(), expected);
                    }
                    Err(error) => {
                        failures += 1;
                        assert_eq!(error.kind, ErrorKind::StepsExhausted);
                        assert_eq!(storage.read().unwrap(), original);
                        vm.budget.steps = 1000;
                        append(&mut vm).unwrap();
                        assert_eq!(storage.read().unwrap(), expected);
                    }
                    other => panic!("unexpected append: {other:?}"),
                }
            }
            assert!(failures > 0 && successes > 0);
        }
    }
}
