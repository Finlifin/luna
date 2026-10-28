//! Ordered Object reads preserve entry domains instead of erasing them.
use super::*;

impl Interpreter<'_> {
    pub(super) fn eval_object_query(
        &mut self,
        expr: &Expr,
        environment: &mut Environment,
    ) -> Result<Flow, Error> {
        use nir::program::ObjectQueryKind as Q;
        let K::ObjectQuery { subject, query } = &expr.kind else {
            unreachable!()
        };
        if !self.comptime_only {
            return Err(self.failure(ErrorKind::PhaseMismatch, Some(expr.source)));
        }
        let failure = || Error {
            kind: ErrorKind::Type,
            source: Some(expr.source),
        };
        let expected = match query {
            Q::Length => Term::Primitive(Primitive::Integer),
            Q::Key => Term::Optional(Box::new(Term::Primitive(Primitive::Symbol))),
            Q::EntryType => Term::Primitive(Primitive::Type),
            Q::Value | Q::NamedValue => {
                if !matches!(
                    expr.ty,
                    Term::Optional(_) | Term::Primitive(Primitive::Never)
                ) {
                    return Err(failure());
                }
                expr.ty.clone()
            }
        };
        if expr.ty != expected && expr.ty != Term::Primitive(Primitive::Never) {
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
        let mut packet = packet.into_iter();
        let Some(Value::Object(object)) = packet.next() else {
            return Err(failure());
        };
        if *query == Q::Length {
            if packet.next().is_some() {
                return Err(failure());
            }
            return Ok(Flow::Value(Value::Integer(
                comptime_runtime::Integer::from_u128(object.entries().len() as u128),
            )));
        }
        if *query == Q::NamedValue {
            let Some(Value::Symbol(key)) = packet.next() else {
                return Err(failure());
            };
            if packet.next().is_some() {
                return Err(failure());
            }
            let Term::Optional(target) = &expr.ty else {
                return Err(failure());
            };
            let property =
                self.lookup_object_property(&object, &key, target)
                    .map_err(|mut error| {
                        if error.source.is_none() {
                            error.source = Some(expr.source);
                        }
                        error
                    })?;
            let value = match property {
                ObjectProperty::Found(entry) => Some(Box::new(
                    self.checkpoint_representation(&entry.value, expr.source)?,
                )),
                ObjectProperty::Missing | ObjectProperty::TypeMismatch(_) => None,
            };
            return Ok(Flow::Value(Value::Optional {
                ty: expr.ty.clone(),
                value,
            }));
        }
        let Some(Value::Integer(index)) = packet.next() else {
            return Err(failure());
        };
        if packet.next().is_some() {
            return Err(failure());
        }
        let index = index
            .to_u128()
            .ok()
            .and_then(|value| usize::try_from(value).ok())
            .filter(|index| *index < object.entries().len())
            .ok_or_else(|| self.failure(ErrorKind::IndexOutOfBounds, Some(expr.source)))?;
        let entry = &object.entries()[index];
        let value = match query {
            Q::Length | Q::NamedValue => unreachable!(),
            Q::Key => {
                if let Some(key) = entry.key() {
                    self.integer_operation(Some(expr.source), |budget| {
                        budget.consume(key.len().saturating_add(1))
                    })?;
                }
                Value::Optional {
                    ty: expr.ty.clone(),
                    value: entry.key().map(|key| Box::new(Value::Symbol(key.into()))),
                }
            }
            Q::EntryType => {
                let work = self
                    .type_description_work(&entry.value().ty)
                    .ok_or_else(failure)?;
                self.integer_operation(Some(expr.source), |budget| budget.consume(work))?;
                Value::Type(entry.value().ty.clone())
            }
            Q::Value => {
                let Term::Optional(target) = &expr.ty else {
                    return Err(failure());
                };
                let work = self
                    .type_description_work(target)
                    .and_then(|left| {
                        self.type_description_work(&entry.value().ty)
                            .and_then(|right| left.checked_add(right))
                    })
                    .ok_or_else(failure)?;
                self.integer_operation(Some(expr.source), |budget| budget.consume(work))?;
                let value = if entry.value().ty == **target {
                    if !self.has_type(&entry.value().value, target) {
                        return Err(failure());
                    }
                    Some(Box::new(self.checkpoint_representation(
                        &entry.value().value,
                        expr.source,
                    )?))
                } else {
                    None
                };
                Value::Optional {
                    ty: expr.ty.clone(),
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
