use super::*;

impl Interpreter<'_> {
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
