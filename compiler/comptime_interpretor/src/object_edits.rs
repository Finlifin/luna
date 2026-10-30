//! Functional Object editing keeps declaration order, keys, and payload domains.
use super::*;

impl Interpreter<'_> {
    pub(super) fn eval_object_edit(
        &mut self,
        expr: &Expr,
        environment: &mut Environment,
    ) -> Result<Flow, Error> {
        let K::ObjectEdit { subject, operation } = &expr.kind else {
            unreachable!()
        };
        self.prepare_object_edit(*operation, &expr.ty, &subject.ty, expr.source)?;
        let packet = match self.eval(subject, environment)? {
            Flow::Value(value) => value,
            flow => return Ok(flow),
        };
        self.complete_object_edit(*operation, &expr.ty, &subject.ty, packet, expr.source)
    }

    pub(super) fn prepare_object_edit(
        &mut self,
        operation: nir::program::ObjectEditKind,
        ty: &Term,
        input: &Term,
        source: SourceRange,
    ) -> Result<(), Error> {
        use nir::program::ObjectEditKind as O;
        if !self.comptime_only {
            return Err(self.failure(ErrorKind::PhaseMismatch, Some(source)));
        }
        let failure = || Error {
            kind: ErrorKind::Type,
            source: Some(source),
        };
        if !matches!(*ty, Term::Primitive(Primitive::Object | Primitive::Never)) {
            return Err(failure());
        }
        let Term::Tuple(domains) = input else {
            return Err(failure());
        };
        let count = if matches!(operation, O::Append | O::Replace | O::Slice) {
            3
        } else {
            2
        };
        if domains.len() != count
            || !matches!(
                domains[0],
                Term::Primitive(Primitive::Object | Primitive::Never)
            )
        {
            return Err(failure());
        }
        let second = match operation {
            O::Concat => Term::Primitive(Primitive::Object),
            O::Append => Term::Optional(Box::new(Term::Primitive(Primitive::Symbol))),
            _ => Term::Primitive(Primitive::Integer),
        };
        if domains[1] != second && domains[1] != Term::Primitive(Primitive::Never) {
            return Err(failure());
        }
        if operation == O::Slice
            && domains[2] != Term::Primitive(Primitive::Integer)
            && domains[2] != Term::Primitive(Primitive::Never)
        {
            return Err(failure());
        }
        Ok(())
    }

    pub(super) fn complete_object_edit(
        &mut self,
        operation: nir::program::ObjectEditKind,
        ty: &Term,
        input: &Term,
        packet: Value,
        source: SourceRange,
    ) -> Result<Flow, Error> {
        use nir::program::ObjectEditKind as O;
        let failure = || Error {
            kind: ErrorKind::Type,
            source: Some(source),
        };
        let Term::Tuple(domains) = input else {
            return Err(failure());
        };
        let count = if matches!(operation, O::Append | O::Replace | O::Slice) {
            3
        } else {
            2
        };
        let second = Term::Optional(Box::new(Term::Primitive(Primitive::Symbol)));
        if *ty == Term::Primitive(Primitive::Never) {
            return Err(failure());
        }
        let Value::Tuple(packet) = packet else {
            return Err(failure());
        };
        if packet.len() != count {
            return Err(failure());
        }
        let mut packet = packet.into_iter();
        let Some(Value::Object(object)) = packet.next() else {
            return Err(failure());
        };
        let operand = packet.next().ok_or_else(failure)?;
        let mut entries = object.into_entries();
        match operation {
            O::Concat => {
                let Value::Object(right) = operand else {
                    return Err(failure());
                };
                let work = entries
                    .len()
                    .saturating_add(right.entries().len())
                    .saturating_add(1);
                self.integer_operation(Some(source), |budget| budget.consume(work))?;
                entries.extend(right.into_entries());
            }
            O::Append | O::Replace => {
                let value = packet.next().ok_or_else(failure)?;
                let ty = &domains[2];
                let work = self.type_description_work(ty).ok_or_else(failure)?;
                self.integer_operation(Some(source), |budget| {
                    budget.consume(work.saturating_add(entries.len()).saturating_add(1))
                })?;
                if !self.has_type(&value, ty) {
                    return Err(failure());
                }
                let value = ObjectValue {
                    ty: ty.clone(),
                    value,
                };
                if operation == O::Append {
                    let Value::Optional { ty, value: key } = operand else {
                        return Err(failure());
                    };
                    if ty != second {
                        return Err(failure());
                    }
                    entries.push(match key {
                        None => ObjectEntry::Value(value),
                        Some(key) => {
                            let Value::Symbol(key) = *key else {
                                return Err(failure());
                            };
                            self.integer_operation(Some(source), |budget| {
                                budget.consume(key.len().saturating_add(1))
                            })?;
                            ObjectEntry::Property {
                                key: key.to_string(),
                                value,
                            }
                        }
                    });
                } else {
                    let index = self.object_edit_index(operand, entries.len(), false, source)?;
                    match &mut entries[index] {
                        ObjectEntry::Value(old) | ObjectEntry::Property { value: old, .. } => {
                            *old = value
                        }
                    }
                }
            }
            O::Slice => {
                let start = self.object_edit_index(operand, entries.len(), true, source)?;
                let end = self.object_edit_index(
                    packet.next().ok_or_else(failure)?,
                    entries.len(),
                    true,
                    source,
                )?;
                if start > end {
                    return Err(self.failure(ErrorKind::IndexOutOfBounds, Some(source)));
                }
                self.integer_operation(Some(source), |budget| {
                    budget.consume(entries.len().saturating_add(1))
                })?;
                entries.truncate(end);
                entries.drain(..start);
            }
            O::Remove => {
                let index = self.object_edit_index(operand, entries.len(), false, source)?;
                self.integer_operation(Some(source), |budget| {
                    budget.consume(entries.len().saturating_add(1))
                })?;
                entries.remove(index);
            }
        }
        Ok(Flow::Value(Value::Object(Object::from_entries(entries))))
    }

    fn object_edit_index(
        &self,
        value: Value,
        length: usize,
        inclusive: bool,
        source: SourceRange,
    ) -> Result<usize, Error> {
        let Value::Integer(index) = value else {
            return Err(self.failure(ErrorKind::Type, Some(source)));
        };
        index
            .to_u128()
            .ok()
            .and_then(|index| usize::try_from(index).ok())
            .filter(|index| *index < length || inclusive && *index == length)
            .ok_or_else(|| self.failure(ErrorKind::IndexOutOfBounds, Some(source)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nir::program::ObjectEditKind as O;

    fn replacement() -> Expr {
        let source = SourceRange { start: 1, end: 2 };
        let fields = vec![
            Expr {
                source,
                ty: Term::Primitive(Primitive::Object),
                kind: K::Local(LocalId { owner: 1, index: 0 }),
            },
            Expr {
                source,
                ty: Term::Primitive(Primitive::Integer),
                kind: K::Literal(nir::program::Literal::Integer("0".into())),
            },
            Expr {
                source,
                ty: Term::Primitive(Primitive::Bool),
                kind: K::Local(LocalId { owner: 1, index: 1 }),
            },
        ];
        Expr {
            source,
            ty: Term::Primitive(Primitive::Object),
            kind: K::ObjectEdit {
                operation: O::Replace,
                subject: Box::new(Expr {
                    source,
                    ty: Term::Tuple(fields.iter().map(|field| field.ty.clone()).collect()),
                    kind: K::Tuple(fields),
                }),
            },
        }
    }

    #[test]
    fn replacement_rejects_forged_payload_and_recovers_without_mutating_input() {
        let package = Package::default();
        let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
        let original = Value::Object(Object::from_entries(vec![ObjectEntry::Property {
            key: "flag".into(),
            value: ObjectValue {
                ty: Term::Primitive(Primitive::Bool),
                value: Value::Bool(true),
            },
        }]));
        let mut env = Environment::new();
        env.insert(LocalId { owner: 1, index: 0 }, original.clone());
        env.insert(LocalId { owner: 1, index: 1 }, Value::Char('x'));
        assert_eq!(
            vm.eval(&replacement(), &mut env).unwrap_err().kind,
            ErrorKind::Type
        );
        env.insert(LocalId { owner: 1, index: 1 }, Value::Bool(false));
        let Flow::Value(Value::Object(edited)) = vm.eval(&replacement(), &mut env).unwrap() else {
            panic!()
        };
        assert_eq!(edited.entries()[0].key(), Some("flag"));
        assert_eq!(edited.entries()[0].value().value, Value::Bool(false));
        assert_eq!(env.get(&LocalId { owner: 1, index: 0 }), Some(&original));
    }

    #[test]
    fn result_and_phase_are_checked_before_operands() {
        let package = Package::default();
        let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
        let mut expression = replacement();
        expression.ty = Term::Primitive(Primitive::Bool);
        assert_eq!(
            vm.eval(&expression, &mut Environment::new())
                .unwrap_err()
                .kind,
            ErrorKind::Type
        );
        vm.comptime_only = false;
        assert_eq!(
            vm.eval(&replacement(), &mut Environment::new())
                .unwrap_err()
                .kind,
            ErrorKind::PhaseMismatch
        );
    }
}

#[cfg(test)]
mod slice_tests {
    use super::*;
    use nir::program::{ObjectEditKind, Place};
    const SOURCE: SourceRange = SourceRange { start: 1, end: 2 };
    const TRACE: LocalId = LocalId { owner: 1, index: 3 };
    fn expr(ty: Term, kind: K) -> Expr {
        Expr {
            source: SOURCE,
            ty,
            kind,
        }
    }
    fn body(end: Primitive) -> Expr {
        let fields = [Primitive::Object, Primitive::Integer, end]
            .into_iter()
            .enumerate()
            .map(|(index, ty)| {
                expr(
                    Term::Primitive(ty),
                    K::Local(LocalId {
                        owner: 1,
                        index: index as u32,
                    }),
                )
            })
            .collect::<Vec<_>>();
        let packet = expr(
            Term::Tuple(fields.iter().map(|e| e.ty.clone()).collect()),
            K::Tuple(fields),
        );
        expr(
            Term::Primitive(Primitive::Object),
            K::ObjectEdit {
                operation: ObjectEditKind::Slice,
                subject: Box::new(expr(
                    packet.ty.clone(),
                    K::Block {
                        statements: vec![expr(
                            Term::Primitive(Primitive::Void),
                            K::Assign {
                                place: Place {
                                    root: TRACE,
                                    fields: vec![],
                                },
                                value: Box::new(expr(
                                    Term::Primitive(Primitive::Bool),
                                    K::Literal(nir::program::Literal::Bool(true)),
                                )),
                            },
                        )],
                        result: Some(Box::new(packet)),
                    },
                )),
            },
        )
    }
    #[test]
    fn object_slice_admission_precedes_effects_and_bounds_follow_them() {
        let package = Package::default();
        let original = Value::Object(Object::from_entries(vec![ObjectEntry::Property {
            key: "flag".into(),
            value: ObjectValue {
                ty: Term::Primitive(Primitive::Bool),
                value: Value::Bool(true),
            },
        }]));
        for compiled in [false, true] {
            let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
            for (end_ty, end_value, error, observed) in [
                (
                    Primitive::Bool,
                    Value::Bool(true),
                    Some(ErrorKind::Type),
                    false,
                ),
                (
                    Primitive::Integer,
                    Value::Integer(comptime_runtime::Integer::from_i128(2)),
                    Some(ErrorKind::IndexOutOfBounds),
                    true,
                ),
                (
                    Primitive::Integer,
                    Value::Bool(true),
                    Some(ErrorKind::Type),
                    true,
                ),
                (
                    Primitive::Integer,
                    Value::Integer(comptime_runtime::Integer::from_i128(1)),
                    None,
                    true,
                ),
            ] {
                let body = body(end_ty);
                let mut env = Environment::from([
                    (LocalId { owner: 1, index: 0 }, original.clone()),
                    (
                        LocalId { owner: 1, index: 1 },
                        Value::Integer(comptime_runtime::Integer::from_i128(0)),
                    ),
                    (LocalId { owner: 1, index: 2 }, end_value),
                    (TRACE, Value::Bool(false)),
                ]);
                let result = if compiled {
                    let code = Arc::new(comptime_compiler::compile(&body, &mut 10000).unwrap());
                    assert!(code.to_string().contains("(object-edit slice"));
                    vm.execute_code(&code, &mut env)
                } else {
                    vm.eval(&body, &mut env)
                };
                if let Some(error) = error {
                    assert_eq!(result.unwrap_err().kind, error);
                } else {
                    let Flow::Value(value) = result.unwrap() else {
                        panic!("value")
                    };
                    assert_eq!(value, original);
                }
                assert_eq!(
                    vm.local_value(&env, TRACE, Some(SOURCE)).unwrap(),
                    Value::Bool(observed)
                );
                assert_eq!(
                    vm.local_value(&env, LocalId { owner: 1, index: 0 }, Some(SOURCE))
                        .unwrap(),
                    original
                );
            }
        }
    }
}
