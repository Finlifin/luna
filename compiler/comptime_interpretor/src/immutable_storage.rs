//! Comptime container transfers validate data and permission before mutation.
use super::*;

impl Interpreter<'_> {
    /// Validate both sides before replacing immutable data through a value
    /// reference. This does not impose runtime Copy on comptime receivers.
    pub(super) fn validate_container_replacement(
        &mut self,
        value: &Value,
        source: SourceRange,
        locked_storage: Option<usize>,
    ) -> Result<(), Error> {
        let mut seen = HashSet::new();
        if let Some(addr) = locked_storage {
            seen.insert((0, addr));
        }
        self.validate_container_replacement_at(value, source, 0, &mut seen)
    }

    fn validate_container_replacement_at(
        &mut self,
        value: &Value,
        source: SourceRange,
        depth: usize,
        seen: &mut HashSet<(u8, usize)>,
    ) -> Result<(), Error> {
        if depth >= 128 {
            return Err(self.failure(ErrorKind::DepthExhausted, Some(source)));
        }
        match value {
            Value::Pointer(_) | Value::Storage(_) | Value::ComptimeReference(_) => {
                Err(self.failure(ErrorKind::PhaseMismatch, Some(source)))
            }
            Value::Object(_) => {
                self.check_comptime_graph_with_seen(value, source, depth, seen)?;
                self.validate_immutable_input(value, &Term::Primitive(Primitive::Object))
                    .map_err(|mut error| {
                        if error.source.is_none() {
                            error.source = Some(source);
                        }
                        error
                    })
            }
            Value::List { ty, .. } => {
                self.check_comptime_graph_with_seen(value, source, depth, seen)?;
                self.validate_immutable_input(value, ty)
                    .map_err(|mut error| {
                        if error.source.is_none() {
                            error.source = Some(source);
                        }
                        error
                    })
            }
            Value::Closure(_) => self.check_comptime_graph_with_seen(value, source, depth, seen),
            Value::Tuple(fields) | Value::Record { fields, .. } | Value::Variant { fields, .. } => {
                self.tick(Some(source))?;
                for field in fields {
                    self.validate_container_replacement_at(field, source, depth + 1, seen)?;
                }
                Ok(())
            }
            Value::AnonymousRecord(fields) => {
                self.tick(Some(source))?;
                for (_, field) in fields {
                    self.validate_container_replacement_at(field, source, depth + 1, seen)?;
                }
                Ok(())
            }
            Value::Optional {
                value: Some(inner), ..
            }
            | Value::ErrorResult { value: inner, .. }
            | Value::Newtype { value: inner, .. } => {
                self.tick(Some(source))?;
                self.validate_container_replacement_at(inner, source, depth + 1, seen)
            }
            _ => Ok(()),
        }
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
            kind: K::Assign {
                place: nir::program::Place {
                    root: LocalId { owner: 1, index: 0 },
                    fields: vec![],
                },
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
            Value::ComptimeReference(value_reference::ValueReference::root(storage.clone())),
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
        for (old, new, expected) in [
            (object(false), invalid.clone(), ErrorKind::Type),
            (invalid, object(true), ErrorKind::Type),
            (object(false), active.clone(), ErrorKind::PhaseMismatch),
            (active, object(true), ErrorKind::PhaseMismatch),
        ] {
            let storage = Storage::new(old.clone());
            let mut env = environment(&storage, new);
            let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
            let error = vm.eval(&write(), &mut env).unwrap_err();
            assert_eq!(error.kind, expected);
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
                collector: None,
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
        let original = Value::List {
            ty: ty.clone(),
            values: vec![Value::Bool(false)].into(),
        };
        let expected = Value::List {
            ty: ty.clone(),
            values: vec![Value::Bool(false), Value::Bool(true)].into(),
        };
        let reference_expr = Expr {
            source,
            ty: Term::Primitive(Primitive::Void),
            kind: K::Assign {
                place: nir::program::Place {
                    root: LocalId { owner: 1, index: 0 },
                    fields: vec![],
                },
                value: Box::new(Expr {
                    source,
                    ty: ty.clone(),
                    kind: K::ListConcat(
                        Box::new(Expr {
                            source,
                            ty: ty.clone(),
                            kind: K::Local(LocalId { owner: 1, index: 0 }),
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
        for mode in 1..8 {
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
            if matches!(mode, 3 | 4 | 6 | 7) {
                let K::Assign { value, .. } = &mut reference_expr.kind else {
                    unreachable!()
                };
                let K::ListConcat(_, right) = &mut value.kind else {
                    unreachable!()
                };
                right.kind = K::Local(LocalId {
                    owner: 1,
                    index: if matches!(mode, 4 | 7) { 0 } else { 1 },
                });
            }
            let original = wrap(original.clone());
            let expected = wrap(if matches!(mode, 4 | 7) {
                Value::List {
                    ty: ty.clone(),
                    values: vec![Value::Bool(false); 2].into(),
                }
            } else {
                expected.clone()
            });
            let mut failures = 0;
            let mut successes = 0;
            for steps in 1..200 {
                let storage = Storage::new(original.clone());
                let mut env = Environment::new();
                let receiver = if mode >= 5 {
                    original.clone()
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
                        values: vec![Value::Bool(true)].into(),
                    },
                );
                let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
                vm.budget.steps = steps;
                let append = |vm: &mut Interpreter<'_>, env: &mut Environment| {
                    if mode >= 5 {
                        vm.eval_local_list_append(&reference_expr, env)
                            .expect("owned append")
                    } else {
                        vm.eval_local_list_append(&reference_expr, env)
                            .expect("reference append")
                    }
                };
                let observe = |env: &Environment| {
                    if mode >= 5 {
                        env[&LocalId { owner: 1, index: 0 }].clone()
                    } else {
                        storage.read().unwrap()
                    }
                };
                match append(&mut vm, &mut env) {
                    Ok(Flow::Value(Value::Unit)) => {
                        successes += 1;
                        assert_eq!(observe(&env), expected);
                    }
                    Err(error) => {
                        failures += 1;
                        assert_eq!(error.kind, ErrorKind::StepsExhausted);
                        assert_eq!(observe(&env), original);
                        vm.budget.steps = 1000;
                        append(&mut vm, &mut env).unwrap();
                        assert_eq!(observe(&env), expected);
                    }
                    other => panic!("unexpected append: {other:?}"),
                }
            }
            assert!(failures > 0 && successes > 0);
        }
    }

    #[test]
    fn nested_phase_or_mutable_cells_in_aggregates_are_rejected_while_self_capturing_closures_succeed()
     {
        let package = Package::default();
        let source = SourceRange { start: 1, end: 2 };
        let cell = Storage::new(Value::Bool(true));
        let ptr = Value::Pointer(cell.pointer(Term::Primitive(Primitive::Bool), vec![]));
        let cref = Value::ComptimeReference(value_reference::ValueReference::root(cell.clone()));
        let bad_obj = Value::Object(Object::from_entries(vec![ObjectEntry::Value(
            ObjectValue {
                ty: Term::Primitive(Primitive::Bool),
                value: Value::Char('x'),
            },
        )]));

        for invalid in [
            Value::Storage(cell.clone()),
            ptr.clone(),
            cref.clone(),
            Value::Tuple(vec![Value::Bool(true), Value::Storage(cell.clone())]),
            Value::AnonymousRecord(vec![("bad".into(), ptr.clone())]),
            Value::Optional {
                ty: Term::Optional(Box::new(Term::Primitive(Primitive::Bool))),
                value: Some(Box::new(cref.clone())),
            },
            Value::Newtype {
                ty: Term::Primitive(Primitive::Bool),
                value: Box::new(bad_obj),
            },
        ] {
            let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
            let target_cell = Storage::new(Value::Bool(false));
            let mut target = Value::ComptimeReference(value_reference::ValueReference::root(
                target_cell.clone(),
            ));
            let err = vm
                .write_comptime_place(&mut target, &[], invalid, source)
                .unwrap_err();
            assert!(
                matches!(err.kind, ErrorKind::PhaseMismatch | ErrorKind::Type),
                "unexpected error kind: {:?}",
                err.kind
            );
            assert_eq!(target_cell.read().unwrap(), Value::Bool(false));
        }

        // A closure capturing `target_cell` stored inside `target_cell` itself
        // can still be overwritten without self-locking on `old`.
        let closure_ty = Term::Closure {
            environment: vec![Term::Primitive(Primitive::Bool)],
            signature: Box::new(Term::ComptimeFunction(Box::new(Term::Function {
                parameters: vec![],
                result: Box::new(Term::Primitive(Primitive::Bool)),
            }))),
        };
        let captured_local = LocalId { owner: 2, index: 0 };
        let closure_expr = Expr {
            source,
            ty: closure_ty,
            kind: K::Closure {
                parameters: vec![],
                captures: vec![nir::program::Capture {
                    binding: captured_local,
                    ty: Term::Primitive(Primitive::Bool),
                    value: None,
                }],
                body: Arc::new(Expr {
                    source,
                    ty: Term::Primitive(Primitive::Bool),
                    kind: K::Local(captured_local),
                }),
            },
        };
        let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
        let mut make_env = Environment::new();
        let target_cell = Storage::new(Value::Bool(false));
        make_env.insert(captured_local, Value::Storage(target_cell.clone()));
        let Flow::Value(closure_val) = vm.eval(&closure_expr, &mut make_env).unwrap() else {
            panic!();
        };
        let wrapped = Value::Tuple(vec![closure_val, Value::Bool(true)]);
        let mut target =
            Value::ComptimeReference(value_reference::ValueReference::root(target_cell.clone()));
        vm.write_comptime_place(&mut target, &[], wrapped.clone(), source)
            .unwrap();
        assert_eq!(target_cell.read().unwrap(), wrapped);
        vm.write_comptime_place(
            &mut target,
            &[],
            Value::Tuple(vec![Value::Bool(false), Value::Bool(false)]),
            source,
        )
        .unwrap();
    }
}
