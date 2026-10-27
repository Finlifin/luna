//! Functional Object editing keeps declaration order, keys, and payload domains.
use super::*;

impl Interpreter<'_> {
    pub(super) fn eval_object_edit(
        &mut self,
        expr: &Expr,
        environment: &mut Environment,
    ) -> Result<Flow, Error> {
        use nir::program::ObjectEditKind as O;
        let K::ObjectEdit { subject, operation } = &expr.kind else {
            unreachable!()
        };
        if !self.comptime_only {
            return Err(self.failure(ErrorKind::PhaseMismatch, Some(expr.source)));
        }
        let failure = || Error {
            kind: ErrorKind::Type,
            source: Some(expr.source),
        };
        if !matches!(
            expr.ty,
            Term::Primitive(Primitive::Object | Primitive::Never)
        ) {
            return Err(failure());
        }
        let Term::Tuple(domains) = &subject.ty else {
            return Err(failure());
        };
        let count = if matches!(operation, O::Append | O::Replace) {
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
        let packet = match self.eval(subject, environment)? {
            Flow::Value(value) => value,
            flow => return Ok(flow),
        };
        if expr.ty == Term::Primitive(Primitive::Never) {
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
                self.integer_operation(Some(expr.source), |budget| budget.consume(work))?;
                entries.extend(right.into_entries());
            }
            O::Append | O::Replace => {
                let value = packet.next().ok_or_else(failure)?;
                let ty = &domains[2];
                let work = self.type_description_work(ty).ok_or_else(failure)?;
                self.integer_operation(Some(expr.source), |budget| {
                    budget.consume(work.saturating_add(entries.len()).saturating_add(1))
                })?;
                if !self.has_type(&value, ty) {
                    return Err(failure());
                }
                let value = ObjectValue {
                    ty: ty.clone(),
                    value,
                };
                if *operation == O::Append {
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
                            self.integer_operation(Some(expr.source), |budget| {
                                budget.consume(key.len().saturating_add(1))
                            })?;
                            ObjectEntry::Property {
                                key: key.to_string(),
                                value,
                            }
                        }
                    });
                } else {
                    let index = self.object_edit_index(operand, entries.len(), expr.source)?;
                    match &mut entries[index] {
                        ObjectEntry::Value(old) | ObjectEntry::Property { value: old, .. } => {
                            *old = value
                        }
                    }
                }
            }
            O::Remove => {
                let index = self.object_edit_index(operand, entries.len(), expr.source)?;
                self.integer_operation(Some(expr.source), |budget| {
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
        source: SourceRange,
    ) -> Result<usize, Error> {
        let Value::Integer(index) = value else {
            return Err(self.failure(ErrorKind::Type, Some(source)));
        };
        index
            .to_u128()
            .ok()
            .and_then(|index| usize::try_from(index).ok())
            .filter(|index| *index < length)
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
