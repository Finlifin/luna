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
        self.check_type_description(requested, None)?;
        self.integer_operation(None, |budget| budget.consume(key.len().saturating_add(1)))?;
        self.admit_type(requested, 0)?;
        let mut selected = None;
        for entry in object.entries() {
            let typed = entry.value();
            self.check_type_description(&typed.ty, None)?;
            self.integer_operation(None, |budget| budget.consume(1))?;
            self.admit_type(&typed.ty, 0)?;
            let source = SourceRange { start: 0, end: 0 };
            self.check_comptime_domain(&typed.ty, source)?;
            self.check_comptime_graph(&typed.value, source, 0)?;
            self.admit_embedded_type_descriptions(&typed.value, 0)?;
            self.check_value_type(&typed.value, &typed.ty, None)?;
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

    pub(super) fn prepare_object(
        &mut self,
        ty: &Term,
        names: usize,
        fields: usize,
        diverges: bool,
        source: SourceRange,
    ) -> Result<(), Error> {
        if !self.comptime_only {
            return Err(self.failure(ErrorKind::PhaseMismatch, Some(source)));
        }
        let expected = Term::Primitive(if diverges {
            Primitive::Never
        } else {
            Primitive::Object
        });
        if names != fields || *ty != expected {
            return Err(self.failure(ErrorKind::Type, Some(source)));
        }
        if fields > 100_000 {
            return Err(self.failure(ErrorKind::RepresentationSizeLimit, Some(source)));
        }
        self.integer_operation(Some(source), |budget| budget.consume(fields))
    }

    pub(super) fn check_object_entry(
        &mut self,
        value: &Value,
        ty: &Term,
        name: Option<&str>,
        source: SourceRange,
    ) -> Result<(), Error> {
        self.charge_object_entry(ty, name, source)?;
        self.check_comptime_domain(ty, source)?;
        self.check_comptime_graph(value, source, 0)?;
        self.admit_embedded_type_descriptions(value, 0)
            .map_err(|mut error| {
                if error.source.is_none() {
                    error.source = Some(source);
                }
                error
            })?;
        self.check_value_type(value, ty, Some(source))
    }

    pub(super) fn charge_object_entry(
        &mut self,
        ty: &Term,
        name: Option<&str>,
        source: SourceRange,
    ) -> Result<(), Error> {
        self.check_type_description(ty, Some(source))?;
        self.integer_operation(Some(source), |budget| {
            budget.consume(name.map_or(0, str::len))
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
        self.prepare_object(
            &expr.ty,
            names.len(),
            fields.len(),
            fields
                .iter()
                .any(|field| field.ty == Term::Primitive(Primitive::Never)),
            expr.source,
        )?;
        self.with_prepared_frame(expr.source, |this| {
            let mut entries = Vec::with_capacity(fields.len());
            for (slot, (name, field)) in names.iter().zip(fields).enumerate() {
                let value = match this.eval(field, environment)? {
                    Flow::Value(value) => value,
                    transfer => return Ok(transfer),
                };
                this.check_object_entry(&value, &field.ty, name.as_deref(), field.source)?;
                this.retain_prepared(slot, field.source, &value)?;
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
