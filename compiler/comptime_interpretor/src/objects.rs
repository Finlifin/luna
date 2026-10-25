use super::*;

/// Internal lookup outcomes, deliberately separate from the source get API's
/// optional/error policy. A successful borrow grants no Copy or move permission.
#[derive(Debug)]
pub enum ObjectProperty<'a> {
    Missing,
    TypeMismatch(&'a ObjectValue),
    Found(&'a ObjectValue),
}

impl Interpreter<'_> {
    /// Read the last named entry, retaining its declared type and payload.
    /// Positional entries do not participate in this named-property lookup.
    /// All entry tags/payloads are validated before returning a result, including
    /// entries hidden by a later duplicate. No user conversion or NF unfolding
    /// participates in the requested-domain comparison.
    pub fn lookup_object_property<'v>(
        &mut self,
        object: &'v Object<ObjectValue>,
        key: &str,
        requested: &Term,
    ) -> Result<ObjectProperty<'v>, Error> {
        if !self.comptime_only {
            return Err(self.failure(ErrorKind::PhaseMismatch, None));
        }
        let work = self
            .type_description_work(requested)
            .ok_or_else(|| self.failure(ErrorKind::Type, None))?;
        self.integer_operation(None, |budget| {
            budget.consume(work.saturating_add(key.len()).saturating_add(1))
        })?;
        self.admit_type(requested, 0)?;
        let mut selected = None;
        for entry in object.entries() {
            let typed = entry.value();
            let work = self
                .type_description_work(&typed.ty)
                .ok_or_else(|| self.failure(ErrorKind::Type, None))?;
            self.integer_operation(None, |budget| budget.consume(work.saturating_add(1)))?;
            self.admit_type(&typed.ty, 0)?;
            self.admit_embedded_type_descriptions(&typed.value, 0)?;
            if !self.has_type(&typed.value, &typed.ty) {
                return Err(self.failure(ErrorKind::Type, None));
            }
            if let Some(name) = entry.key() {
                self.integer_operation(None, |budget| {
                    budget.consume(name.len().min(key.len()).saturating_add(1))
                })?;
                if name == key {
                    selected = Some(typed);
                }
            }
        }
        Ok(match selected {
            None => ObjectProperty::Missing,
            Some(value) if value.ty == *requested => ObjectProperty::Found(value),
            Some(value) => ObjectProperty::TypeMismatch(value),
        })
    }

    pub(super) fn eval_object(
        &mut self,
        expr: &Expr,
        environment: &mut Environment,
    ) -> Result<Flow, Error> {
        let K::Object { names, fields } = &expr.kind else {
            unreachable!()
        };
        if !self.comptime_only {
            return Err(self.failure(ErrorKind::PhaseMismatch, Some(expr.source)));
        }
        let diverges = fields
            .iter()
            .any(|field| field.ty == Term::Primitive(Primitive::Never));
        let expected = Term::Primitive(if diverges {
            Primitive::Never
        } else {
            Primitive::Object
        });
        if names.len() != fields.len() || expr.ty != expected {
            return Err(self.failure(ErrorKind::Type, Some(expr.source)));
        }
        self.integer_operation(Some(expr.source), |budget| budget.consume(fields.len()))?;
        self.with_prepared_frame(expr.source, |this| {
            let mut entries = Vec::with_capacity(fields.len());
            for (slot, (name, field)) in names.iter().zip(fields).enumerate() {
                let value = match this.eval(field, environment)? {
                    Flow::Value(value) => value,
                    transfer => return Ok(transfer),
                };
                if !this.has_type(&value, &field.ty) {
                    return Err(this.failure(ErrorKind::Type, Some(field.source)));
                }
                this.retain_prepared(slot, field.source, &value)?;
                let work = this
                    .type_description_work(&field.ty)
                    .ok_or_else(|| this.failure(ErrorKind::Type, Some(field.source)))?;
                this.integer_operation(Some(field.source), |budget| {
                    budget.consume(work.saturating_add(name.as_ref().map_or(0, String::len)))
                })?;
                let value = ObjectValue {
                    ty: field.ty.clone(),
                    value,
                };
                entries.push(match name {
                    None => ObjectEntry::Value(value),
                    Some(key) => ObjectEntry::Property {
                        key: key.clone(),
                        value,
                    },
                });
            }
            Ok(Flow::Value(Value::Object(Object::from_entries(entries))))
        })
    }
}
